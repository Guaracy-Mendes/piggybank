use super::*;
use bank_core::accruals::{Calculation, PeriodicPolicy};
use bank_documents::offers::*;
use bank_finance::periodic_execution::*;
use chrono::{Datelike, NaiveDate};
use sqlx::Executor;
async fn clock(f: &Fixture, date: NaiveDate) -> Result<()> {
    let sql = format!(
        "CREATE OR REPLACE FUNCTION core.periodic_clock() RETURNS timestamptz LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ SELECT '{}T00:00:00Z'::timestamptz $$",
        date
    );
    f.db.migrator.execute(sqlx::AssertSqlSafe(sql)).await?;
    Ok(())
}
async fn prepared(
    monthly: bool,
    amount: i64,
) -> Result<(
    Fixture,
    bank_documents::Terms,
    ActorContext,
    AccountId,
    Uuid,
    NaiveDate,
)> {
    let (mut f, t, a, account, policy) = super::enrollment::setup().await?;
    let mut p: PeriodicPolicy = f
        .core
        .periodic_policies()
        .get(policy)
        .await?
        .unwrap()
        .policy;
    p.id = Uuid::new_v4();
    p.code = if monthly {
        "R1_MONTHLY_EXEC"
    } else {
        "R1_DAILY_EXEC"
    }
    .into();
    let today = Utc::now().date_naive();
    let business_start = if monthly {
        today
            .with_day(1)
            .unwrap()
            .checked_sub_months(chrono::Months::new(2))
            .unwrap()
    } else {
        today - Duration::days(2)
    };
    clock(&f, business_start).await?;
    f.chart.effective_from = today - Duration::days(100);
    p.calendar.cutoff = "00:00:00".into();
    p.calendar.holidays = vec![];
    if monthly {
        p.effective_from = business_start.with_day(1).unwrap();
        p.effective_until = p
            .effective_from
            .checked_add_months(chrono::Months::new(3))
            .unwrap();
        p.calendar.coverage_from = p.effective_from;
        p.calendar.coverage_until = p.effective_until + Duration::days(15);
        p.calculation = Calculation::MonthlyFixedFee { amount: money(25) };
    }
    let r =
        f.f.propose_periodic_policy(
            &f.maker,
            Uuid::new_v4(),
            &bank_finance::periodic::PolicyInput {
                policy: p.clone(),
                reason: "Essential R1 execution fixture".into(),
            },
        )
        .await?;
    f.f.approve_periodic_policy(&f.checker, r.id, &r.policy_digest)
        .await?;
    f.f.publish_periodic_policy(&f.maker, r.id, &r.policy_digest)
        .await?;
    f.publish(&f.chart).await?;
    if !monthly {
        let mut chart = f.chart.clone();
        chart.id = Uuid::new_v4();
        chart.version = 2;
        chart
            .accounts
            .insert("INTEREST_EXPENSE".into(), "EXPENSE".into());
        chart.by_account.insert(
            Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000005")?,
            "INTEREST_EXPENSE".into(),
        );
        let approval =
            f.f.controls
                .propose_change(
                    &f.maker,
                    ChangeKind::FinanceChart,
                    chart.id,
                    &chart,
                    Utc::now() + Duration::hours(1),
                )
                .await?;
        f.f.controls.approve_change(&f.checker, approval).await?;
        f.f.publish_chart(&f.maker, approval, &chart).await?;
    }
    if amount > 0 {
        f.core
            .fund_synthetic(
                Uuid::new_v4(),
                account,
                AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?),
                money(amount),
            )
            .await?;
    }
    let o = super::enrollment::offer(&f, &t, p.id).await?;
    let c = t
        .request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &super::enrollment::accept(&o),
        )
        .await?;
    let counter = Uuid::parse_str(if monthly {
        "4f1a780b-3e31-48ef-a294-000000000004"
    } else {
        "4f1a780b-3e31-48ef-a294-000000000005"
    })?;
    let r =
        f.f.propose_periodic_activation(
            &f.maker,
            Uuid::new_v4(),
            &ActivationInput {
                consent_id: c.consent.id,
                counter_account_id: counter,
            },
        )
        .await?;
    assert!(
        f.f.execute_periodic_activation(&f.maker, r.id, &r.digest)
            .await
            .is_err()
    );
    assert!(
        f.f.approve_periodic_activation(&f.maker, r.id, &r.digest)
            .await
            .is_err()
    );
    assert!(
        f.f.approve_periodic_activation(&f.checker, r.id, &"0".repeat(64))
            .await
            .is_err()
    );
    f.f.approve_periodic_activation(&f.checker, r.id, &r.digest)
        .await?;
    let active =
        f.f.execute_periodic_activation(&f.maker, r.id, &r.digest)
            .await?;
    let first: NaiveDate =
        serde_json::from_value(active.receipt.unwrap().activation["firstPeriodStart"].clone())?;
    let view = t
        .periodic_enrollment(&f.core, &f.f.controls, &a, account, o.material.id)
        .await?;
    view.validate()?;
    assert_eq!(view.status, "ACTIVE");
    assert!(
        t.withdraw_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &WithdrawalInput {
                consent_digest: c.consent_digest,
                withdraw: true
            }
        )
        .await
        .is_err()
    );
    let end = if monthly {
        first.checked_add_months(chrono::Months::new(1)).unwrap()
    } else {
        first + Duration::days(1)
    };
    clock(&f, end).await?;
    Ok((f, t, a, account, r.id, first))
}
async fn correct(f: &Fixture, batch: Uuid) -> Result<Uuid> {
    let r =
        f.f.propose_correction(
            &f.maker,
            Uuid::new_v4(),
            &bank_finance::corrections::CorrectionInput {
                original_batch_id: batch,
                reason: "Reverse the exact native periodic posting".into(),
            },
        )
        .await?;
    f.f.approve_correction(&f.checker, r.id, &r.details_digest)
        .await?;
    let r =
        f.f.execute_correction(&f.maker, r.id, &r.details_digest)
            .await?;
    Ok(r.receipt.unwrap().batch_id)
}
#[tokio::test]
async fn monthly_fee_finalizes_once_posts_balanced_native_money_exports_gl_and_corrects_once()
-> Result<()> {
    let (f, _t, _a, account, id, start) = prepared(true, 10000).await?;
    // WP-25: a statement cannot be complete while an enrolled policy is unfinalized for its month.
    let statement_ready = |f: &Fixture| {
        let pool = f.db.migrator.clone();
        async move {
            let readiness: serde_json::Value = sqlx::query_scalar(
                "SELECT documents.statement_readiness($1,date_trunc('month',$2::date)::date)",
            )
            .bind(account.0)
            .bind(start)
            .fetch_one(&pool)
            .await?;
            anyhow::Ok(
                readiness["reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|r| r != "PERIODIC_FINALIZATION_PENDING"),
            )
        }
    };
    assert!(!statement_ready(&f).await?);
    let r =
        f.f.queue_periodic_run(
            &f.maker,
            Uuid::new_v4(),
            &PeriodInput {
                activation_id: id,
                period_start: start,
            },
        )
        .await?;
    let p = f.f.process_periodic_run(r.id).await?;
    assert!(p.posted);
    assert_eq!(p.amount_minor, "25");
    assert!(statement_ready(&f).await?);
    let batch = p.batch_id.unwrap();
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 9975);
    let replay = f.f.process_periodic_run(r.id).await?;
    assert_eq!(p.digest, replay.digest);
    assert_eq!(p.batch_id, replay.batch_id);
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 9975);
    let server = f.server()?;
    f.wait().await?;
    let export = f.f.stage_export(batch).await?;
    for _ in 0..10 {
        if matches!(
            f.f.process_export(export.id).await?,
            bank_finance::Progress::Done { .. }
        ) {
            break;
        }
    }
    assert_eq!(f.f.export_status(export.id).await?.state, "BOOKED");
    drop(server);
    let correction = correct(&f, batch).await?;
    assert_ne!(correction, batch);
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 10000);
    assert!(correct(&f, batch).await.is_err());
    assert_eq!(
        f.core
            .periodic_commands()
            .period(id, start)
            .await?
            .unwrap()
            .digest,
        p.digest
    );
    f.close().await
}
#[tokio::test]
async fn finalized_monthly_fee_keeps_holds_and_recovers_original_period_after_insufficient_funds()
-> Result<()> {
    let (f, _t, _a, account, id, start) = prepared(true, 100).await?;
    let hold = Uuid::new_v4();
    f.core.reserve(hold, account, money(80)).await?;
    let r =
        f.f.queue_periodic_run(
            &f.maker,
            Uuid::new_v4(),
            &PeriodInput {
                activation_id: id,
                period_start: start,
            },
        )
        .await?;
    assert!(f.f.process_periodic_run(r.id).await.is_err());
    let original = f.core.periodic_commands().period(id, start).await?.unwrap();
    assert!(!original.posted);
    let b = f.core.balance(account).await?;
    assert_eq!(b.booked.minor_units(), 100);
    assert_eq!(b.held.minor_units(), 80);
    f.core.release(hold).await?;
    let posted = f.f.process_periodic_run(r.id).await?;
    assert_eq!(posted.digest, original.digest);
    assert!(posted.posted);
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 75);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM finance.periodic_issues WHERE case_id IS NOT NULL"
        )
        .fetch_one(&f.f.pool)
        .await?,
        1
    );
    f.close().await
}
#[tokio::test]
async fn daily_accrual_closes_native_booked_prefix_prevents_backdating_and_has_linked_reversal()
-> Result<()> {
    let (f, _t, _a, account, id, start) = prepared(false, 100000).await?;
    let p = f
        .core
        .periodic_commands()
        .post(&f.f.controls, &f.maker, id, start)
        .await?;
    assert_eq!(p.amount_minor, "10");
    assert_eq!(p.period["closingBookedMinor"], json!("100000"));
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 100010);
    clock(&f, start).await?;
    assert!(
        f.core
            .fund_synthetic(
                Uuid::new_v4(),
                account,
                AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?),
                money(1)
            )
            .await
            .is_err()
    );
    clock(&f, start + Duration::days(1)).await?;
    let corrected = correct(&f, p.batch_id.unwrap()).await?;
    assert_ne!(corrected, p.batch_id.unwrap());
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 100000);
    assert_eq!(
        f.core
            .periodic_commands()
            .period(id, start)
            .await?
            .unwrap()
            .digest,
        p.digest
    );
    f.close().await
}
#[tokio::test]
async fn zero_accrual_records_completion_without_zero_journal_and_suspension_blocks_new_effects()
-> Result<()> {
    let (f, _t, _a, account, id, start) = prepared(false, 0).await?;
    let r = f
        .core
        .periodic_commands()
        .post(&f.f.controls, &f.maker, id, start)
        .await?;
    assert!(r.posted);
    assert!(r.batch_id.is_none());
    assert_eq!(r.amount_minor, "0");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.core.pool)
            .await?,
        0
    );
    f.core
        .periodic_commands()
        .stop(
            &f.f.controls,
            &f.maker,
            id,
            "Stop future periodic processing",
        )
        .await?;
    clock(&f, start + Duration::days(2)).await?;
    assert!(f.core.periodic_commands().next_due(id).await?.is_none());
    assert!(
        f.core
            .periodic_commands()
            .post(&f.f.controls, &f.maker, id, start + Duration::days(1))
            .await
            .is_err()
    );
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 0);
    f.close().await
}
#[tokio::test]
async fn rehashed_finalization_amount_cannot_authorize_a_different_financial_effect() -> Result<()>
{
    let (f, _t, _a, account, id, start) = prepared(false, 100000).await?;
    let r = f
        .core
        .periodic_commands()
        .finalize(&f.f.controls, &f.maker, id, start)
        .await?;
    let before = f.core.balance(account).await?;
    sqlx::query("ALTER TABLE core.periodic_finalizations DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE core.periodic_finalizations SET document=jsonb_set(document,'{amountMinor}','\"999\"')").execute(&f.db.migrator).await?;
    sqlx::query("UPDATE core.periodic_finalizations SET digest=encode(sha256(convert_to(core.periodic_policy_canonical(document),'UTF8')),'hex')").execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE core.periodic_finalizations ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(
        f.core
            .periodic_commands()
            .post(&f.f.controls, &f.maker, id, start)
            .await
            .is_err()
    );
    assert_eq!(
        f.core.balance(account).await?.booked.minor_units(),
        before.booked.minor_units()
    );
    assert!(r.batch_id.is_none());
    f.close().await
}

#[tokio::test]
async fn automatic_due_scan_repairs_unscheduled_intent_and_concurrent_jobs_share_one_native_effect()
-> Result<()> {
    let (f, _t, _a, account, id, start) = prepared(false, 100000).await?;
    let gap = Uuid::new_v4();
    sqlx::query("INSERT INTO finance.periodic_runs(id,issuer,subject,client_id,request_key,actor_context,activation_id,period_start) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(gap).bind(&f.maker.issuer).bind(&f.maker.subject).bind(&f.maker.client_id).bind(Uuid::new_v4()).bind(serde_json::to_value(&f.maker)?).bind(id).bind(start).execute(&f.f.pool).await?;
    assert!(f.f.periodic_run(&f.maker, gap).await?.receipt.is_none());
    assert!(
        sqlx::query("UPDATE finance.periodic_runs SET period_start=period_start+1 WHERE id=$1")
            .bind(gap)
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT core.post_periodic_period($1,$2,$3)")
            .bind(Uuid::new_v4())
            .bind(json!({}))
            .bind(Uuid::new_v4())
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("CREATE OR REPLACE FUNCTION core.periodic_clock() RETURNS timestamptz LANGUAGE sql AS $$ SELECT now() $$").execute(&f.core.pool).await.is_err());
    f.f.recover_periodic_execution().await?;
    let scheduled = f.f.periodic_run(&f.maker, gap).await?;
    assert!(scheduled.operation_id.is_some());
    let runs = f.f.periodic_runs(&f.maker).await?;
    assert_eq!(runs.len(), 2);
    let other = runs.iter().find(|r| r.id != gap).unwrap().id;
    let (a, b) = tokio::join!(
        f.f.process_periodic_run(gap),
        f.f.process_periodic_run(other)
    );
    let a = a?;
    let b = b?;
    assert!(a.posted && b.posted);
    assert_eq!(a.batch_id, b.batch_id);
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 100010);
    sqlx::query("UPDATE controls.role_grants SET revoked_at=clock_timestamp() WHERE role='finance_adjuster'").execute(&f.db.migrator).await?;
    clock(&f, start + Duration::days(2)).await?;
    f.f.recover_periodic_execution().await?;
    // Next scan wraps the retained activation cursor.
    f.f.recover_periodic_execution().await?;
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 100010);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM finance.periodic_issues WHERE case_id IS NOT NULL"
        )
        .fetch_one(&f.f.pool)
        .await?,
        1
    );
    f.close().await
}
