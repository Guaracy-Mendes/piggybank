//! Local fixture publication through the same independent approval paths as application configuration.
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_core::{Core, products::ProductVersion};
use bank_documents::{Terms, TermsDraft};
use bank_platform::{
    config::Config,
    database::Module,
    domain::{ActorContext, Currency, Money, TrustContext},
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use std::io::Read;
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Actors {
    issuer: String,
    maker_subject: String,
    checker_subject: String,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = Config::load(4181)?;
    let mut input = String::new();
    std::io::stdin().take(8192).read_to_string(&mut input)?;
    let args: Actors = serde_json::from_str(&input)?;
    anyhow::ensure!(
        args.issuer == "http://127.0.0.1:18280/realms/workforce",
        "Fixture issuer must be the local workforce realm"
    );
    Uuid::parse_str(&args.maker_subject)?;
    Uuid::parse_str(&args.checker_subject)?;
    let actor = |subject: String| ActorContext {
        issuer: args.issuer.clone(),
        subject,
        trust: TrustContext::Workforce,
        client_id: "staff-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let maker = actor(args.maker_subject);
    let checker = actor(args.checker_subject);
    let core = Core::new(Module::Core.connect().await?);
    let controls = Entitlements::new(Module::Controls.connect().await?);
    let finance = bank_finance::local().await?;
    let terms = Terms::new(Module::Documents.connect().await?);
    let money = |n| Money::new(n, Currency::synthetic());
    let chart = bank_finance::chart::ChartVersion {
        id: Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000007")?,
        code: "synthetic-chart-v1".into(),
        version: 1,
        label: "SYNTHETIC".into(),
        effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        effective_until: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
        accounts: std::collections::BTreeMap::from([
            ("CUSTOMER_DEPOSITS".into(), "LIABILITY".into()),
            ("CLEARING_ASSET".into(), "ASSET".into()),
            ("FEE_REVENUE".into(), "REVENUE".into()),
            ("SUSPENSE".into(), "LIABILITY".into()),
        ]),
        by_kind: std::collections::BTreeMap::from([
            ("CUSTOMER".into(), "CUSTOMER_DEPOSITS".into()),
            ("SETTLEMENT".into(), "CLEARING_ASSET".into()),
        ]),
        by_account: std::collections::BTreeMap::from([
            (
                Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000004")?,
                "FEE_REVENUE".into(),
            ),
            (
                Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000006")?,
                "SUSPENSE".into(),
            ),
        ]),
    };
    let existing: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT document FROM finance.chart_versions WHERE id=$1")
            .bind(chart.id)
            .fetch_optional(&finance.pool)
            .await?;
    if let Some(existing) = existing {
        anyhow::ensure!(
            bank_platform::domain::fingerprint(&existing)?
                == bank_platform::domain::fingerprint(&chart)?,
            "Fixture chart differs; publish a new approved version"
        );
    } else {
        let p = controls
            .propose_change(
                &maker,
                ChangeKind::FinanceChart,
                chart.id,
                &chart,
                Utc::now() + Duration::hours(1),
            )
            .await?;
        controls.approve_change(&checker, p).await?;
        finance.publish_chart(&maker, p, &chart).await?;
    }
    // Preserve chart v1 byte for byte; new approved v2 adds explicit interest expense.
    let mut periodic_chart = chart.clone();
    periodic_chart.id = Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000008")?;
    periodic_chart.version = 2;
    periodic_chart
        .accounts
        .insert("INTEREST_EXPENSE".into(), "EXPENSE".into());
    periodic_chart.by_account.insert(
        Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000005")?,
        "INTEREST_EXPENSE".into(),
    );
    let existing: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT document FROM finance.chart_versions WHERE id=$1")
            .bind(periodic_chart.id)
            .fetch_optional(&finance.pool)
            .await?;
    if let Some(existing) = existing {
        anyhow::ensure!(
            bank_platform::domain::fingerprint(&existing)?
                == bank_platform::domain::fingerprint(&periodic_chart)?,
            "Fixture chart v2 differs"
        );
    } else {
        let p = controls
            .propose_change(
                &maker,
                ChangeKind::FinanceChart,
                periodic_chart.id,
                &periodic_chart,
                Utc::now() + Duration::hours(1),
            )
            .await?;
        controls.approve_change(&checker, p).await?;
        finance.publish_chart(&maker, p, &periodic_chart).await?;
    }
    // A new independently approved chart preserves the sealed v1 and v2 documents.
    let mut consumer_chart = periodic_chart.clone();
    consumer_chart.id = Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000009")?;
    consumer_chart.version = 3;
    consumer_chart
        .accounts
        .insert("BROKER_CLEARING".into(), "ASSET".into());
    consumer_chart
        .accounts
        .insert("CONSUMER_SIMULATOR_CLEARING".into(), "ASSET".into());
    consumer_chart.by_account.insert(
        Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000020")?,
        "BROKER_CLEARING".into(),
    );
    consumer_chart.by_account.insert(
        Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000021")?,
        "CONSUMER_SIMULATOR_CLEARING".into(),
    );
    let existing: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT document FROM finance.chart_versions WHERE id=$1")
            .bind(consumer_chart.id)
            .fetch_optional(&finance.pool)
            .await?;
    if let Some(existing) = existing {
        anyhow::ensure!(
            bank_platform::domain::fingerprint(&existing)?
                == bank_platform::domain::fingerprint(&consumer_chart)?,
            "Fixture chart v3 differs"
        );
    } else {
        let proposal = controls
            .propose_change(
                &maker,
                ChangeKind::FinanceChart,
                consumer_chart.id,
                &consumer_chart,
                Utc::now() + Duration::hours(1),
            )
            .await?;
        controls.approve_change(&checker, proposal).await?;
        finance
            .publish_chart(&maker, proposal, &consumer_chart)
            .await?;
    }
    let product = ProductVersion {
        id: Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000001")?,
        code: "PIGGY_TST".into(),
        version: 1,
        name: "Piggybank synthetic current account".into(),
        label: "SYNTHETIC".into(),
        effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        effective_until: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
        minimum_age: 18,
        payment_limit: money(1_000_000),
        daily_limit: money(5_000_000),
        flat_fee: money(0),
        fee_basis_points: 0,
        chart_version: "synthetic-chart-v1".into(),
    };
    match core.product(product.id).await {
        Ok(existing) => anyhow::ensure!(
            bank_platform::domain::fingerprint(&existing)?
                == bank_platform::domain::fingerprint(&product)?,
            "Fixture product differs; publish a new approved version"
        ),
        Err(e)
            if matches!(
                e.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::RowNotFound)
            ) =>
        {
            let proposal = controls
                .propose_change(
                    &maker,
                    ChangeKind::Product,
                    product.id,
                    &product,
                    Utc::now() + Duration::hours(1),
                )
                .await?;
            controls.approve_change(&checker, proposal).await?;
            core.publish_product(&controls, &maker, proposal, &product)
                .await?;
        }
        Err(e) => return Err(e),
    }
    let draft=TermsDraft{id:Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000002")?,product_version_id:product.id,version:1,title:"Synthetic demonstration terms".into(),text:"Synthetic demonstration only. TST has no monetary value. This local environment is not a bank account or a real payment service. Use synthetic identities and example.invalid contacts. No deposits, interest, legal protection or actual banking terms are offered. Configuration and all provider outcomes are fixtures. Preserve your application reference to resume the demonstration.".into(),label:"SYNTHETIC".into(),effective_from:product.effective_from,effective_until:product.effective_until};
    match terms.version(draft.id).await {
        Ok(existing) => anyhow::ensure!(
            bank_platform::domain::fingerprint(&existing)?
                == bank_platform::domain::fingerprint(&draft)?,
            "Fixture terms differ; publish a new approved version"
        ),
        Err(e)
            if matches!(
                e.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::RowNotFound)
            ) =>
        {
            let proposal = controls
                .propose_change(
                    &maker,
                    ChangeKind::Terms,
                    draft.id,
                    &draft,
                    Utc::now() + Duration::hours(1),
                )
                .await?;
            controls.approve_change(&checker, proposal).await?;
            terms
                .publish(&core, &controls, &maker, proposal, &draft)
                .await?;
        }
        Err(e) => return Err(e),
    }
    let payments = bank_payments::local().await?;
    if payments.rail_policy().await?.policy.is_none() {
        let policy = bank_payments::policy::RailPolicyVersion {
            id: Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000005")?,
            rail: "SIMULATED".into(),
            version: 1,
            label: "SYNTHETIC".into(),
            enabled: true,
            effective_from: product.effective_from,
            effective_until: product.effective_until,
            payment_limit: money(1_000_000),
            calendar: "SYNTHETIC_UTC_24X7".into(),
        };
        let proposal = payments.propose_rail_policy(&maker, &policy).await?;
        controls.approve_change(&checker, proposal).await?;
        payments.apply_rail_change(&maker, proposal).await?;
    }
    println!(
        "Synthetic product and exact terms published through distinct maker/checker identities."
    );
    Ok(())
}
