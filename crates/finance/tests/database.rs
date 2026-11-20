use anyhow::Result;
#[path = "cases/audit_delivery.rs"]
mod audit_delivery;
#[path = "cases/basis.rs"]
mod basis;
#[path = "cases/corrections.rs"]
mod corrections;
#[path = "cases/disclosures.rs"]
mod disclosures;
#[path = "cases/enrollment.rs"]
mod enrollment;
#[path = "cases/liquidity.rs"]
mod liquidity;
#[path = "cases/periodic.rs"]
mod periodic;
#[path = "cases/periodic_execution.rs"]
mod periodic_execution;
#[path = "cases/reconciliation.rs"]
mod reconciliation;
#[path = "cases/revisions.rs"]
mod revisions;
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_core::{Core, finance::FinanceRecords};
use bank_finance::{
    Finance, Progress,
    chart::ChartVersion,
    client::GlClient,
    protocol::{GlBatch, GlEntry},
};
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, Currency, Money, PartyId, TrustContext, fingerprint},
    operations::{JobSpec, Operations},
    testing::TestDatabase,
};
use chrono::{Duration, Utc};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;
fn range() -> bank_core::finance_snapshot::ReportRange {
    let today = Utc::now().date_naive();
    bank_core::finance_snapshot::ReportRange {
        from_date: today - Duration::days(2),
        to_date: today + Duration::days(1),
    }
}
fn cash(version: i64, stale: bool) -> bank_finance::liquidity::LiquidityEvidence {
    let now = Utc::now()
        - if stale {
            Duration::minutes(2)
        } else {
            Duration::zero()
        };
    bank_finance::liquidity::LiquidityEvidence {
        evidence_id: Uuid::new_v4(),
        provider: "SYNTHETIC_LIQUIDITY".into(),
        scope: bank_finance::liquidity::LIQUIDITY_SCOPE.into(),
        currency: Currency::synthetic(),
        version,
        as_of: now,
        valid_until: now + Duration::seconds(60),
        settled: "9007199254741993".into(),
        available: "9007199254740993".into(),
        reserved: "1000".into(),
        expected_incoming: "4321".into(),
        expected_outgoing: "2345".into(),
    }
}
async fn publish_cash(
    f: &Fixture,
    value: &bank_finance::liquidity::LiquidityEvidence,
) -> Result<()> {
    let r = reqwest::Client::new()
        .post(format!("{}/admin/liquidity-positions", f.base))
        .header("X-Liquidity-Key", bank_finance::liquidity::liquidity_key())
        .json(value)
        .send()
        .await?;
    assert_eq!(r.status(), reqwest::StatusCode::CREATED);
    Ok(())
}
fn actor(s: &str) -> ActorContext {
    ActorContext {
        issuer: "urn:finance-fixture".into(),
        subject: s.into(),
        client_id: "staff-web".into(),
        trust: TrustContext::Workforce,
        roles: vec![],
        person_id: None,
        represented_party: None,
    }
}
fn money(n: i64) -> Money {
    Money::new(n, Currency::synthetic())
}
struct Server(std::process::Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Fixture {
    db: TestDatabase,
    f: Finance,
    core: Core,
    maker: ActorContext,
    checker: ActorContext,
    chart: ChartVersion,
    base: String,
    address: std::net::SocketAddr,
}
impl Fixture {
    async fn new() -> Result<Self> {
        let db = TestDatabase::new().await?;
        let core = Core::new(db.module(Module::Core).await?);
        let controls = Entitlements::new(db.module(Module::Controls).await?);
        let maker = actor("maker");
        let checker = actor("checker");
        for (a, role) in [(&maker, "finance_adjuster"), (&checker, "finance_approver")] {
            let person = PartyId::new();
            controls.link_person(a, person).await?;
            sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&db.migrator).await?;
            sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('finance-operations',$1,$2)").bind(person.0).bind(if role=="finance_adjuster" {"REVIEW"} else {"APPROVE"}).execute(&db.migrator).await?;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        drop(listener);
        let base = format!("http://{address}");
        let f = Finance {
            pool: db.module(Module::Finance).await?,
            core: Arc::new(core.clone()),
            periodic: core.periodic_policies(),
            bases: core.periodic_bases(),
            periodic_commands: core.periodic_commands(),
            corrections: core.finance_corrections(),
            payments: bank_payments::finance::FinanceReader::new(
                db.module(Module::Payments).await?,
            ),
            controls,
            operations: Operations::new(db.module(Module::Platform).await?),
            client: GlClient::new(&base)?,
        };
        let chart = ChartVersion {
            id: Uuid::new_v4(),
            code: "synthetic-chart-v1".into(),
            version: 1,
            label: "SYNTHETIC".into(),
            effective_from: Utc::now().date_naive() - Duration::days(10),
            effective_until: Utc::now().date_naive() + Duration::days(365),
            accounts: BTreeMap::from([
                ("CUSTOMER_DEPOSITS".into(), "LIABILITY".into()),
                ("CLEARING_ASSET".into(), "ASSET".into()),
                ("FEE_REVENUE".into(), "REVENUE".into()),
                ("SUSPENSE".into(), "LIABILITY".into()),
            ]),
            by_kind: BTreeMap::from([
                ("CUSTOMER".into(), "CUSTOMER_DEPOSITS".into()),
                ("SETTLEMENT".into(), "CLEARING_ASSET".into()),
            ]),
            by_account: BTreeMap::from([
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
        Ok(Self {
            db,
            f,
            core,
            maker,
            checker,
            chart,
            base,
            address,
        })
    }
    async fn publish(&self, chart: &ChartVersion) -> Result<Uuid> {
        let p = self
            .f
            .controls
            .propose_change(
                &self.maker,
                ChangeKind::FinanceChart,
                chart.id,
                chart,
                Utc::now() + Duration::hours(1),
            )
            .await?;
        self.f.controls.approve_change(&self.checker, p).await?;
        self.f.publish_chart(&self.maker, p, chart).await?;
        Ok(p)
    }
    async fn source(&self, n: i64) -> Result<Uuid> {
        let customer = AccountId::new();
        let clearing = AccountId::new();
        for (id, kind) in [(customer, "CUSTOMER"), (clearing, "SETTLEMENT")] {
            sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,$2,'ACTIVE',false)").bind(id.0).bind(kind).execute(&self.db.migrator).await?;
        }
        let op = Uuid::new_v4();
        self.core
            .fund_synthetic(op, customer, clearing, money(n))
            .await?;
        Ok(
            sqlx::query_scalar("SELECT id FROM core.journal_batches WHERE operation_id=$1")
                .bind(op)
                .fetch_one(&self.db.migrator)
                .await?,
        )
    }
    fn server(&self) -> Result<Server> {
        Ok(Server(
            std::process::Command::new(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/debug/provider-simulator"),
            )
            .env("BANK_BIND", self.address.to_string())
            .env(
                "BANK_SIMULATOR_DATABASE_URL",
                self.db.module_url(Module::Simulator),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
        ))
    }
    async fn wait(&self) -> Result<()> {
        for _ in 0..100 {
            if reqwest::Client::new()
                .get(format!("{}/health/live", self.base))
                .send()
                .await
                .is_ok()
            {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        anyhow::bail!("GL provider did not start")
    }
    async fn mode(&self, source: Uuid, mode: &str) -> Result<()> {
        sqlx::query("INSERT INTO simulator.gl_scenarios VALUES($1,$2) ON CONFLICT(source_batch_id) DO UPDATE SET mode=EXCLUDED.mode").bind(source).bind(mode).execute(&self.db.migrator).await?;
        Ok(())
    }
    async fn close(self) -> Result<()> {
        self.f.pool.close().await;
        self.f.controls.pool.close().await;
        self.f.operations.pool.close().await;
        self.core.pool.close().await;
        self.db.close().await
    }
}

#[tokio::test]
async fn independent_chart_sealed_exact_export_and_provider_dedup() -> Result<()> {
    let f = Fixture::new().await?;
    let proposal =
        f.f.controls
            .propose_change(
                &f.maker,
                ChangeKind::FinanceChart,
                f.chart.id,
                &f.chart,
                Utc::now() + Duration::hours(1),
            )
            .await?;
    assert!(
        f.f.controls
            .approve_change(&f.maker, proposal)
            .await
            .is_err()
    );
    let alias = actor("same-person-alias");
    f.f.controls
        .link_person(&alias, f.f.controls.person(&f.maker).await?)
        .await?;
    assert!(f.f.controls.approve_change(&alias, proposal).await.is_err());
    assert!(
        f.f.publish_chart(&f.maker, proposal, &f.chart)
            .await
            .is_err()
    );
    f.f.controls.approve_change(&f.checker, proposal).await?;
    let other = actor("other-finance-maker");
    let person = PartyId::new();
    f.f.controls.link_person(&other, person).await?;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'finance_adjuster',now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).execute(&f.db.migrator).await?;
    assert!(f.f.publish_chart(&other, proposal, &f.chart).await.is_err());
    let mut changed = f.chart.clone();
    changed.version = 2;
    assert!(
        f.f.publish_chart(&f.maker, proposal, &changed)
            .await
            .is_err()
    );
    f.f.publish_chart(&f.maker, proposal, &f.chart).await?;
    f.f.publish_chart(&f.maker, proposal, &f.chart).await?;
    let source = f.source(9_007_199_254_740_993).await?;
    let (a, b) = tokio::join!(f.f.stage_export(source), f.f.stage_export(source));
    let a = a?;
    assert_eq!(a.id, b?.id);
    assert_eq!(a.batch.debit_total, "9007199254740993");
    assert_eq!(a.batch.credit_total, a.batch.debit_total);
    let v = serde_json::to_value(&a.batch)?;
    assert!(v["entries"][0]["delta"]["minorUnits"].is_string());
    let server = f.server()?;
    f.wait().await?;
    assert!(matches!(
        f.f.process_export(a.id).await?,
        Progress::Done { .. }
    ));
    let receipt = f.f.export_status(a.id).await?.receipt.unwrap();
    let mut chart = f.chart.clone();
    chart.id = Uuid::new_v4();
    chart.version = 2;
    chart
        .accounts
        .insert("NEW_DEPOSITS".into(), "LIABILITY".into());
    chart
        .by_kind
        .insert("CUSTOMER".into(), "NEW_DEPOSITS".into());
    f.publish(&chart).await?;
    assert_eq!(
        fingerprint(&f.f.stage_export(source).await?.batch)?,
        fingerprint(&a.batch)?
    );
    assert!(matches!(
        f.f.process_export(a.id).await?,
        Progress::Done { .. }
    ));
    assert_eq!(
        f.f.client.book(&a.batch).await?.evidence.evidence_id,
        receipt.evidence_id
    );
    let mut replacement = a.batch.clone();
    replacement.export_id = Uuid::new_v4();
    assert!(f.f.client.book(&replacement).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM simulator.gl_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    assert!(
        sqlx::query("UPDATE finance.chart_versions SET version=99")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE finance.gl_exports SET document='{}'")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE finance.gl_receipts SET digest='altered'")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM finance.gl_history")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    let denied = sqlx::query("SELECT * FROM core.journal_entries")
        .execute(&f.f.pool)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("42501")
    );
    let denied = sqlx::query("SELECT core.fund_synthetic($1,$2,$3,1)")
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .execute(&f.f.pool)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("42501")
    );
    f.f.relay_audit().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM finance.audit_pending WHERE delivered_at IS NULL"
        )
        .fetch_one(&f.f.pool)
        .await?,
        0
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn lost_gl_ack_restart_inquiry_and_exact_not_found_replay() -> Result<()> {
    let f = Fixture::new().await?;
    f.publish(&f.chart).await?;
    let source = f.source(10007).await?;
    let work = f.f.stage_export(source).await?;
    let server = f.server()?;
    f.wait().await?;
    f.mode(source, "LOST_RESPONSE").await?;
    assert!(matches!(
        f.f.process_export(work.id).await?,
        Progress::Pending
    ));
    assert_eq!(f.f.export_status(work.id).await?.state, "OUTCOME_UNKNOWN");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM simulator.gl_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    drop(server);
    let server = f.server()?;
    f.wait().await?;
    assert!(matches!(
        f.f.process_export(work.id).await?,
        Progress::Done { .. }
    ));
    assert_eq!(f.f.export_status(work.id).await?.attempts, 2);
    let other = f.source(207).await?;
    let other = f.f.stage_export(other).await?;
    f.mode(other.source_batch_id, "BEFORE_ACCEPT_TIMEOUT")
        .await?;
    assert!(matches!(
        f.f.process_export(other.id).await?,
        Progress::Pending
    ));
    f.mode(other.source_batch_id, "ACCEPT").await?;
    assert!(matches!(
        f.f.process_export(other.id).await?,
        Progress::Done { .. }
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM finance.gl_observations WHERE export_id=$1 AND kind='NOT_FOUND'"
        )
        .bind(other.id)
        .fetch_one(&f.f.pool)
        .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM simulator.gl_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        2
    );
    // Platform accepted a job but Finance never received its acknowledgment/link.
    let system = ActorContext {
        issuer: "urn:piggybank:finance".into(),
        subject: "gl-export".into(),
        client_id: "bank-worker".into(),
        trust: TrustContext::Service,
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let first =
        f.f.operations
            .accept(
                &system,
                "finance.export",
                &work.id.to_string(),
                &work.batch,
                JobSpec {
                    kind: "finance.export".into(),
                    step_key: "export".into(),
                    payload: json!({"exportId":work.id,"generation":0}),
                    delay_seconds: 0,
                },
            )
            .await?;
    assert!(f.f.export_status(work.id).await?.operation_id.is_none());
    f.f.recover().await?;
    f.f.recover().await?;
    // The source was sealed and the mapping gap matched, but its workflow
    // acknowledgment and case assignment were lost in another process.
    let lost = Uuid::new_v4();
    sqlx::query("INSERT INTO finance.export_breaks(id,source_batch_id,code,state) VALUES($1,$2,'SOURCE_MAPPING_MISSING','OPEN')").bind(lost).bind(source).execute(&f.db.migrator).await?;
    f.f.stage_export(source).await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM finance.export_breaks WHERE id=$1")
            .bind(lost)
            .fetch_one(&f.f.pool)
            .await?,
        "MATCHED"
    );
    f.f.recover().await?;
    assert!(
        sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT case_id FROM finance.export_breaks WHERE id=$1"
        )
        .bind(lost)
        .fetch_one(&f.f.pool)
        .await?
        .is_some()
    );
    assert_eq!(
        f.f.export_status(work.id).await?.operation_id,
        Some(first.id)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM platform.jobs WHERE operation_id=$1")
            .bind(first.id)
            .fetch_one(&f.f.operations.pool)
            .await?,
        1
    );
    sqlx::query("UPDATE finance.gl_exports SET operation_id=NULL WHERE id=$1")
        .bind(work.id)
        .execute(&f.db.migrator)
        .await
        .expect_err("Even migrator cannot erase a sealed operation link");
    assert_eq!(f.f.schedule_export(work.id).await?.id, first.id);
    drop(server);
    f.close().await
}

#[tokio::test]
async fn conflicting_gl_receipt_quarantined_and_owned_without_core_changes() -> Result<()> {
    let f = Fixture::new().await?;
    f.publish(&f.chart).await?;
    let source = f.source(15000).await?;
    let work = f.f.stage_export(source).await?;
    let server = f.server()?;
    f.wait().await?;
    f.mode(source, "CONFLICT").await?;
    assert!(matches!(
        f.f.process_export(work.id).await?,
        Progress::Review { .. }
    ));
    let status = f.f.export_status(work.id).await?;
    assert_eq!(status.state, "REVIEW_REQUIRED");
    assert!(status.receipt.is_none());
    assert_eq!(f.f.staff_exports(&f.checker).await?.len(), 1);
    let key = Uuid::new_v4();
    assert!(
        f.f.retry_export(&f.checker, work.id, key, "Provider receipt corrected")
            .await
            .is_err()
    );
    let (r1, r2) = tokio::join!(
        f.f.retry_export(&f.maker, work.id, key, "Provider receipt corrected"),
        f.f.retry_export(&f.maker, work.id, key, "Provider receipt corrected")
    );
    let request = r1?;
    assert_eq!(request.id, r2?.id);
    assert_eq!(request.generation, 1);
    assert!(
        f.f.retry_export(&f.maker, work.id, key, "Changed reason")
            .await
            .is_err()
    );
    assert!(matches!(
        f.f.process_export_at(work.id, 0).await?,
        Progress::Superseded { .. }
    ));
    f.mode(source, "ACCEPT").await?;
    // Recovery scheduling is separate from the exhausted original operation, with no new GL identity.
    f.f.recover().await?;
    f.f.recover().await?;
    let detail = f.f.staff_detail(&f.maker, work.id).await?;
    assert!(detail["recoveryRequests"][0]["operationId"].is_string());
    assert!(matches!(
        f.f.process_export_at(work.id, 1).await?,
        Progress::Done { .. }
    ));
    assert!(f.f.export_status(work.id).await?.receipt.is_some());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM simulator.gl_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    let person = f.f.controls.person(&f.maker).await?;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=now() WHERE queue='finance-operations' AND person_id=$1").bind(person.0).execute(&f.db.migrator).await?;
    assert!(
        f.f.retry_export(&f.maker, work.id, key, "Provider receipt corrected")
            .await
            .is_err()
    );
    assert!(f.f.staff_detail(&f.maker, work.id).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM finance.gl_observations WHERE kind='QUARANTINED'"
        )
        .fetch_one(&f.f.pool)
        .await?,
        1
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM controls.cases WHERE kind='FINANCE' AND queue='finance-operations' AND resource_id=$1").bind(source).fetch_one(&f.db.migrator).await?,1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn missing_mapping_discovery_repaired_and_revoked_chart_authority_denied() -> Result<()> {
    let f = Fixture::new().await?;
    let source = f.source(100).await?;
    f.f.recover().await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM finance.export_breaks WHERE source_batch_id=$1"
        )
        .bind(source)
        .fetch_one(&f.f.pool)
        .await?,
        "OPEN"
    );
    let p = f.publish(&f.chart).await?;
    f.f.recover().await?;
    f.f.recover().await?;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM finance.gl_exports WHERE source_batch_id=$1")
        .bind(source)
        .fetch_one(&f.f.pool)
        .await?;
    assert!(f.f.export_status(id).await?.operation_id.is_some());
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM finance.export_breaks WHERE source_batch_id=$1"
        )
        .bind(source)
        .fetch_one(&f.f.pool)
        .await?,
        "MATCHED"
    );
    let checker = f.f.controls.person(&f.checker).await?;
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='finance_approver'").bind(checker.0).execute(&f.db.migrator).await?;
    assert!(f.f.publish_chart(&f.maker, p, &f.chart).await.is_err());
    let values = f.core.batch_ids(Uuid::nil()).await?;
    assert_eq!(values, vec![source]);
    f.close().await
}

#[test]
fn gl_control_totals_exceed_i64_without_float_or_overflow() {
    let entries = vec![
        GlEntry {
            core_account_id: Uuid::new_v4(),
            gl_account: "CUSTOMER".into(),
            delta: money(i64::MAX),
        },
        GlEntry {
            core_account_id: Uuid::new_v4(),
            gl_account: "CUSTOMER".into(),
            delta: money(i64::MAX),
        },
        GlEntry {
            core_account_id: Uuid::new_v4(),
            gl_account: "ASSET".into(),
            delta: money(-i64::MAX),
        },
        GlEntry {
            core_account_id: Uuid::new_v4(),
            gl_account: "ASSET".into(),
            delta: money(-i64::MAX),
        },
    ];
    assert_eq!(
        GlBatch::totals(&entries),
        ("18446744073709551614".into(), "18446744073709551614".into())
    );
    let _ = json!({"totals":GlBatch::totals(&entries)});
}

#[tokio::test]
async fn transport_budget_and_expired_lease_require_audited_bounded_recovery() -> Result<()> {
    let f = Fixture::new().await?;
    f.publish(&f.chart).await?;
    let source = f.source(1009).await?;
    let work = f.f.stage_export(source).await?;
    // An unexpired lease fences a second sender even if the original process disappeared.
    sqlx::query("UPDATE finance.gl_exports SET state='OUTCOME_UNKNOWN',attempted_at=now(),lease_id=$2,lease_until=now()+interval '6 seconds',lease_fence=1 WHERE id=$1").bind(work.id).bind(Uuid::new_v4()).execute(&f.db.migrator).await?;
    assert!(matches!(
        f.f.process_export(work.id).await?,
        Progress::Pending
    ));
    assert_eq!(f.f.export_status(work.id).await?.attempts, 0);
    sqlx::query("UPDATE finance.gl_exports SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(work.id)
        .execute(&f.db.migrator)
        .await?;
    for _ in 0..8 {
        assert!(matches!(
            f.f.process_export(work.id).await?,
            Progress::Pending
        ));
    }
    assert!(matches!(
        f.f.process_export(work.id).await?,
        Progress::Review { .. }
    ));
    assert_eq!(f.f.export_status(work.id).await?.attempts, 8);
    let command =
        f.f.retry_export(
            &f.maker,
            work.id,
            Uuid::new_v4(),
            "Provider restarted; recover the original source",
        )
        .await?;
    let status = f.f.export_status(work.id).await?;
    assert_eq!(status.attempt_limit, 16);
    assert_eq!(status.attempts, 8);
    let server = f.server()?;
    f.wait().await?;
    assert!(matches!(
        f.f.process_export_at(work.id, command.generation).await?,
        Progress::Done { .. }
    ));
    assert_eq!(f.f.export_status(work.id).await?.attempts, 9);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM finance.gl_observations WHERE kind='NOT_FOUND'"
        )
        .fetch_one(&f.f.pool)
        .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM simulator.gl_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn core_reporting_snapshot_stays_consistent_during_real_concurrent_postings() -> Result<()> {
    let f = Fixture::new().await?;
    let id = f.source(1003).await?;
    let source = f.core.financial_batch(id).await?;
    let customer = source
        .journal
        .entries
        .iter()
        .find(|l| l.amount.minor_units() > 0)
        .unwrap()
        .account_id;
    let clearing = source
        .journal
        .entries
        .iter()
        .find(|l| l.amount.minor_units() < 0)
        .unwrap()
        .account_id;
    let writer = async {
        for _ in 0..20 {
            f.core
                .fund_synthetic(Uuid::new_v4(), customer, clearing, money(101))
                .await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let reader = async {
        for _ in 0..20 {
            let s = f.core.snapshot(range()).await?;
            let total: i128 = s
                .batches
                .iter()
                .flat_map(|b| b.journal.entries.iter())
                .filter(|l| l.account_id == customer)
                .map(|l| i128::from(l.amount.minor_units()))
                .sum();
            assert_eq!(s.positions.customer_booked, total.to_string());
            assert_eq!(s.positions.ledger_control_total, "0");
        }
        Ok::<_, anyhow::Error>(())
    };
    let (a, b) = tokio::join!(writer, reader);
    a?;
    b?;
    assert_eq!(f.core.snapshot(range()).await?.batches.len(), 21);
    assert!(
        f.core
            .snapshot(bank_core::finance_snapshot::ReportRange {
                from_date: range().to_date,
                to_date: range().from_date
            })
            .await
            .is_err()
    );
    f.close().await
}

#[tokio::test]
async fn report_membership_exact_totals_and_missing_evidence_are_immutable_across_replay()
-> Result<()> {
    let f = Fixture::new().await?;
    f.publish(&f.chart).await?;
    let source = f.source(9_007_199_254_740_993).await?;
    let work = f.f.stage_export(source).await?;
    let server = f.server()?;
    f.wait().await?;
    let key = Uuid::new_v4();
    let (a, b) = tokio::join!(
        f.f.create_report(&f.maker, key, range()),
        f.f.create_report(&f.maker, key, range())
    );
    let before = a?;
    assert_eq!(before.report.id, b?.report.id);
    assert_eq!(before.report.completeness, "PARTIAL");
    assert_eq!(before.report.liquidity.status, "MISSING");
    assert_eq!(before.report.pending_count, 1);
    assert_eq!(before.report.core_debit_total, "9007199254740993");
    assert!(
        before
            .issues
            .iter()
            .any(|i| i.code == "LIQUIDITY_MISSING" && i.case_id.is_some())
    );
    assert!(matches!(
        f.f.process_export(work.id).await?,
        Progress::Done { .. }
    ));
    let next = f.source(7).await?;
    let next = f.f.stage_export(next).await?;
    assert!(matches!(
        f.f.process_export(next.id).await?,
        Progress::Done { .. }
    ));
    let evidence = cash(1, false);
    publish_cash(&f, &evidence).await?;
    let replay = f.f.create_report(&f.maker, key, range()).await?;
    assert_eq!(replay.digest, before.digest);
    assert_eq!(replay.report.source_count, 1);
    assert_eq!(replay.report.pending_count, 1);
    let fresh = f.f.create_report(&f.maker, Uuid::new_v4(), range()).await?;
    assert_eq!(fresh.report.source_count, 2);
    assert_eq!(fresh.report.matched_count, 2);
    assert_eq!(fresh.report.completeness, "COMPLETE");
    assert_eq!(fresh.report.core_debit_total, "9007199254741000");
    assert_eq!(
        fresh.report.confirmed_gl_credit_total,
        fresh.report.core_credit_total
    );
    let first = f.f.report_items(&f.checker, fresh.report.id, 0, 1).await?;
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.next_after, Some(1));
    assert_eq!(first.items[0].classification, "MATCHED");
    let second = f.f.report_items(&f.checker, fresh.report.id, 1, 1).await?;
    assert_eq!(second.next_after, None);
    assert_ne!(
        first.items[0].source.journal.id,
        second.items[0].source.journal.id
    );
    assert!(
        f.f.report_items(&f.maker, fresh.report.id, -1, 100)
            .await
            .is_err()
    );
    let mut changed = range();
    changed.from_date -= Duration::days(1);
    assert!(f.f.create_report(&f.maker, key, changed).await.is_err());
    assert!(
        f.f.create_report(&f.checker, Uuid::new_v4(), range())
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE finance.reports SET document='{}'")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM finance.report_items")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    let mut short = cash(2, false);
    short.valid_until = short.as_of + Duration::seconds(2);
    publish_cash(&f, &short).await?;
    let timed = f.f.create_report(&f.maker, Uuid::new_v4(), range()).await?;
    assert_eq!(timed.report.liquidity.status, "FRESH");
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    let expired = f.f.report_detail(&f.maker, timed.report.id).await?;
    assert_eq!(expired.digest, timed.digest);
    assert_eq!(expired.report.liquidity.status, "FRESH");
    assert_eq!(expired.current_liquidity_status, "STALE");
    assert!(f.f.staff_liquidity(&f.maker).await?.available.is_none());
    let person = f.f.controls.person(&f.maker).await?;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=now() WHERE person_id=$1 AND queue='finance-operations'").bind(person.0).execute(&f.db.migrator).await?;
    assert!(f.f.create_report(&f.maker, key, range()).await.is_err());
    assert!(f.f.report_detail(&f.maker, before.report.id).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        2
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn reporting_classifies_restored_gl_value_and_date_gaps_and_owns_each_observation()
-> Result<()> {
    let f = Fixture::new().await?;
    f.publish(&f.chart).await?;
    let source = f.source(1009).await?;
    let original = f.core.financial_batch(source).await?;
    let entries = original
        .journal
        .entries
        .iter()
        .map(|e| GlEntry {
            core_account_id: e.account_id.0,
            gl_account: f.chart.by_kind[&original.account_kinds[&e.account_id.0]].clone(),
            delta: money(e.amount.minor_units() * 2),
        })
        .collect::<Vec<_>>();
    let (d, c) = GlBatch::totals(&entries);
    let batch = GlBatch {
        export_id: Uuid::new_v4(),
        source_batch_id: source,
        original_source_batch_id: None,
        core_operation_id: original.journal.operation_id,
        posting_kind: original.journal.posting_kind.clone(),
        business_date: original.journal.business_date - Duration::days(1),
        chart_id: f.chart.id,
        chart_version: f.chart.version,
        chart_digest: fingerprint(&f.chart)?,
        source_digest: fingerprint(&original)?,
        debit_total: d,
        credit_total: c,
        entries,
    };
    // Disposable restore-corruption fixture: runtime immutability prevents altering a sealed export.
    sqlx::query("INSERT INTO finance.gl_exports(id,source_batch_id,chart_id,document,fingerprint) VALUES($1,$2,$3,$4,$5)").bind(batch.export_id).bind(source).bind(f.chart.id).bind(serde_json::to_value(&batch)?).bind(fingerprint(&batch)?).execute(&f.db.migrator).await?;
    let server = f.server()?;
    f.wait().await?;
    publish_cash(&f, &cash(1, false)).await?;
    let report = f.f.create_report(&f.maker, Uuid::new_v4(), range()).await?;
    assert_eq!(report.report.conflicting_count, 1);
    assert_eq!(report.report.confirmed_gl_debit_total, "0");
    assert_eq!(report.report.completeness, "PARTIAL");
    for code in ["GL_DATE_MISMATCH", "GL_VALUE_MISMATCH"] {
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == code && i.case_id.is_some())
        );
    }
    assert_eq!(
        f.f.report_items(&f.maker, report.report.id, 0, 25)
            .await?
            .items[0]
            .classification,
        "GL_CONFLICT"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn liquidity_requires_current_authenticated_scope_and_never_falls_back_to_old_or_core_cash()
-> Result<()> {
    let f = Fixture::new().await?;
    let server = f.server()?;
    f.wait().await?;
    let missing = f.f.staff_liquidity(&f.maker).await?;
    assert_eq!(missing.status, "MISSING");
    assert!(missing.available.is_none());
    assert_eq!(missing.core_reserved_principal, "0");
    let stale = cash(1, true);
    publish_cash(&f, &stale).await?;
    let view = f.f.staff_liquidity(&f.maker).await?;
    assert_eq!(view.status, "STALE");
    assert!(view.available.is_none());
    assert!(view.externally_settled.is_none());
    let fresh = cash(2, false);
    publish_cash(&f, &fresh).await?;
    publish_cash(&f, &fresh).await?;
    let view = f.f.staff_liquidity(&f.checker).await?;
    assert_eq!(view.status, "FRESH");
    assert_eq!(view.available.as_deref(), Some("9007199254740993"));
    assert_eq!(view.expected_incoming.as_deref(), Some("4321"));
    let mut invalid = cash(3, false);
    invalid.scope = "FOREIGN_CLEARING".into();
    assert!(invalid.validate().is_err());
    sqlx::query("INSERT INTO simulator.liquidity_positions VALUES($1,$2,$3,$4,now())")
        .bind(invalid.evidence_id)
        .bind(invalid.version)
        .bind(serde_json::to_value(&invalid)?)
        .bind(fingerprint(&invalid)?)
        .execute(&f.db.migrator)
        .await?;
    let invalid = f.f.staff_liquidity(&f.maker).await?;
    assert_eq!(invalid.status, "INVALID");
    assert!(invalid.available.is_none());
    assert!(invalid.evidence_id.is_none());
    drop(server);
    assert_eq!(f.f.staff_liquidity(&f.maker).await?.status, "UNAVAILABLE");
    f.close().await
}

#[tokio::test]
async fn oversized_report_period_rejects_instead_of_publishing_truncated_complete_totals()
-> Result<()> {
    let f = Fixture::new().await?;
    let id = f.source(1).await?;
    let source = f.core.financial_batch(id).await?;
    let customer = source
        .journal
        .entries
        .iter()
        .find(|l| l.amount.minor_units() > 0)
        .unwrap()
        .account_id;
    let clearing = source
        .journal
        .entries
        .iter()
        .find(|l| l.amount.minor_units() < 0)
        .unwrap()
        .account_id;
    sqlx::query(
        "SELECT core.fund_synthetic(gen_random_uuid(),$1,$2,1) FROM generate_series(1,1000)",
    )
    .bind(customer.0)
    .bind(clearing.0)
    .execute(&f.db.migrator)
    .await?;
    assert!(f.core.snapshot(range()).await.is_err());
    assert!(
        f.f.create_report(&f.maker, Uuid::new_v4(), range())
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM finance.reports")
            .fetch_one(&f.f.pool)
            .await?,
        0
    );
    assert_eq!(f.core.balance(customer).await?.booked, money(1001));
    f.close().await
}

#[tokio::test]
async fn authenticated_gl_receipt_cannot_hide_a_wrong_customer_account_classification() -> Result<()>
{
    let mut f = Fixture::new().await?;
    let id = f.source(1024).await?;
    let source = f.core.financial_batch(id).await?;
    let customer = source
        .journal
        .entries
        .iter()
        .find(|e| e.amount.minor_units() > 0)
        .unwrap()
        .account_id
        .0;
    f.chart.by_account.insert(customer, "FEE_REVENUE".into());
    f.publish(&f.chart).await?;
    assert!(f.f.stage_export(id).await.is_err());
    let entries = source
        .journal
        .entries
        .iter()
        .map(|e| GlEntry {
            core_account_id: e.account_id.0,
            gl_account: if e.account_id.0 == customer {
                "FEE_REVENUE"
            } else {
                "CLEARING_ASSET"
            }
            .into(),
            delta: e.amount,
        })
        .collect::<Vec<_>>();
    let (d, c) = GlBatch::totals(&entries);
    let batch = GlBatch {
        export_id: Uuid::new_v4(),
        source_batch_id: id,
        original_source_batch_id: None,
        core_operation_id: source.journal.operation_id,
        posting_kind: source.journal.posting_kind.clone(),
        business_date: source.journal.business_date,
        chart_id: f.chart.id,
        chart_version: 1,
        chart_digest: fingerprint(&f.chart)?,
        source_digest: fingerprint(&source)?,
        debit_total: d,
        credit_total: c,
        entries,
    };
    let server = f.server()?;
    f.wait().await?;
    publish_cash(&f, &cash(1, false)).await?;
    // Restore fixture: the external GL accepted a balanced but semantically wrong mapping.
    let receipt = f.f.client.book(&batch).await?;
    sqlx::query("INSERT INTO finance.gl_exports(id,source_batch_id,chart_id,document,fingerprint,state) VALUES($1,$2,$3,$4,$5,'BOOKED')").bind(batch.export_id).bind(id).bind(f.chart.id).bind(serde_json::to_value(&batch)?).bind(fingerprint(&batch)?).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO finance.gl_receipts(export_id,evidence_id,digest,document) VALUES($1,$2,$3,$4)").bind(batch.export_id).bind(receipt.evidence.evidence_id).bind(fingerprint(&receipt.evidence)?).bind(serde_json::to_value(receipt.document)?).execute(&f.db.migrator).await?;
    let report = f.f.create_report(&f.maker, Uuid::new_v4(), range()).await?;
    assert_eq!(report.report.completeness, "PARTIAL");
    assert_eq!(report.report.matched_count, 0);
    assert_eq!(report.report.conflicting_count, 1);
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.code == "GL_MAPPING_MISMATCH" && i.case_id.is_some())
    );
    drop(server);
    f.close().await
}
