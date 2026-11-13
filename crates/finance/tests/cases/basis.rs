use super::*;
use bank_core::{
    accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest},
    accruals::{
        Calculation,
        basis::{BasisInput, BasisLimit},
    },
};
use chrono::{Datelike, Months};
use futures_util::future::BoxFuture;
struct TestOpening;
impl OpeningPolicy for TestOpening {
    fn validate<'a>(&'a self, r: &'a OpeningRequest) -> BoxFuture<'a, Result<ActivationEvidence>> {
        Box::pin(async move {
            Ok(ActivationEvidence {
                application_id: r.application_id,
                party_id: r.party_id,
                product_version_id: r.product_version_id,
                acceptance_id: r.acceptance_id,
                controls_revision: "basis-test-opening".into(),
                expires_at: Utc::now() + Duration::minutes(10),
            })
        })
    }
}
pub(super) async fn setup(f: &Fixture) -> Result<(Uuid, AccountId)> {
    let mut input = super::periodic::setup(f).await?;
    let today = Utc::now().date_naive();
    input.policy.code = "NATIVE_DAILY".into();
    input.policy.effective_from = today - Duration::days(60);
    input.policy.effective_until = today + Duration::days(30);
    input.policy.calendar.coverage_from = input.policy.effective_from;
    input.policy.calendar.coverage_until = input.policy.effective_until + Duration::days(15);
    input.policy.calendar.business_weekdays = vec![1, 2, 3, 4, 5, 6, 7];
    input.policy.calendar.holidays = vec![today + Duration::days(1)];
    input.policy.calculation = Calculation::DailyCreditAccrual {
        annual_basis_points: 365,
        day_count: "ACT_365_FIXED".into(),
        basis: "NONNEGATIVE_CLOSING_BOOKED".into(),
        rounding: "HALF_UP_EACH_DAY".into(),
    };
    let p =
        f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &input)
            .await?;
    f.f.approve_periodic_policy(&f.checker, p.id, &p.policy_digest)
        .await?;
    f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
        .await?;
    let mut who = actor("basis-customer");
    who.trust = TrustContext::Customer;
    who.client_id = "customer-web".into();
    let application = Uuid::new_v4();
    let party = f
        .core
        .ensure_party(&who, application, &fingerprint(&who.subject)?)
        .await?;
    let request = OpeningRequest {
        application_id: application,
        party_id: party,
        product_version_id: input.policy.product_version_id,
        acceptance_id: Uuid::new_v4(),
    };
    let account = f.core.stage_account(&request).await?;
    f.core.activate(account, &request, &TestOpening).await?;
    sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES('4f1a780b-3e31-48ef-a294-000000000003','SETTLEMENT','ACTIVE',false) ON CONFLICT(id) DO NOTHING").execute(&f.db.migrator).await?;
    Ok((input.policy.id, account))
}
async fn fund(f: &Fixture, account: AccountId, amount: i64) -> Result<Uuid> {
    f.core
        .fund_synthetic(
            Uuid::new_v4(),
            account,
            AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?),
            money(amount),
        )
        .await
}
fn input(policy: Uuid, account: AccountId) -> BasisInput {
    BasisInput {
        policy_id: policy,
        account_id: account.0,
        period_start: Utc::now().date_naive(),
    }
}
#[tokio::test]
async fn native_basis_exact_large_money_holds_holiday_due_and_original_replay_never_post()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    let source = fund(&f, account, 9_007_199_254_740_993).await?;
    let original = f.core.financial_batch(source).await?;
    let hold = Uuid::new_v4();
    f.core.reserve(hold, account, money(500)).await?;
    let before = f.core.balance(account).await?;
    let key = Uuid::new_v4();
    let request = input(policy, account);
    let a = f.f.capture_periodic_basis(&f.maker, key, &request).await?;
    assert_eq!(a.basis.observed_closing_booked, "9007199254740993");
    assert_eq!(a.basis.current_held, "500");
    assert_eq!(a.basis.current_available, "9007199254740493");
    assert_eq!(a.basis.source_count, 1);
    assert_eq!(a.basis.period_status, "OPEN_PERIOD_PROVISIONAL");
    let calc = a.calculation.as_ref().unwrap();
    assert_eq!(calc.amount.minor_units(), 900_719_925_474);
    assert_eq!(calc.exact_numerator, "3287627727980462445");
    assert_eq!(calc.basis_source, "CORE_BUSINESS_DATE_BASIS_V1");
    assert!(calc.provisional);
    assert_eq!(
        a.basis
            .due_at
            .parse::<chrono::DateTime<Utc>>()?
            .date_naive(),
        request.period_start + Duration::days(2)
    );
    let after = f.core.balance(account).await?;
    assert_eq!(before.version, after.version);
    assert_eq!(before.booked, after.booked);
    assert_eq!(before.held, after.held);
    assert_eq!(
        fingerprint(&original)?,
        fingerprint(&f.core.financial_batch(source).await?)?
    );
    let page =
        f.f.periodic_basis_items(&f.checker, a.basis.id, 0, 1)
            .await?;
    assert_eq!(page.items[0].batch_id, source);
    assert_eq!(page.items[0].delta_minor, "9007199254740993");
    assert!(page.next_after.is_none());
    fund(&f, account, 5000).await?;
    let replay = f.f.capture_periodic_basis(&f.maker, key, &request).await?;
    assert_eq!(a.digest, replay.digest);
    assert_eq!(replay.basis.current_booked, a.basis.current_booked);
    let b =
        f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &request)
            .await?;
    assert_eq!(b.basis.source_count, 2);
    assert_eq!(b.basis.observed_closing_booked, "9007199254745993");
    assert_ne!(b.digest, a.digest);
    assert_eq!(
        f.f.periodic_basis(&f.checker, a.basis.id).await?.digest,
        a.digest
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.audit_pending WHERE id=$1")
        .bind(a.basis.id)
        .fetch_one(&f.f.pool)
        .await?;
    assert_eq!(count, 1);
    f.close().await
}
#[tokio::test]
async fn historical_prefix_excludes_later_postings_and_does_not_fall_back_to_current_booked()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    fund(&f, account, 10000).await?;
    let mut request = input(policy, account);
    request.period_start -= Duration::days(1);
    let a =
        f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &request)
            .await?;
    assert_eq!(a.basis.period_status, "CLOSED_PERIOD_OBSERVATION");
    assert_eq!(a.basis.source_count, 0);
    assert_eq!(a.basis.observed_closing_booked, "0");
    assert_eq!(a.basis.current_booked, "10000");
    assert!(a.calculation.is_none());
    assert_eq!(
        a.calculation_reason.as_deref(),
        Some("ACCOUNT_REGISTERED_AFTER_PERIOD_START")
    );
    // Historical account-registration fixture metadata only. No financial facts or clocks are rewritten.
    sqlx::query(
        "UPDATE core.account_registry SET created_at=created_at-interval '90 days' WHERE id=$1",
    )
    .bind(account.0)
    .execute(&f.db.migrator)
    .await?;
    let b =
        f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &request)
            .await?;
    assert_eq!(b.calculation.unwrap().amount.minor_units(), 0);
    assert_eq!(
        f.f.periodic_basis(&f.maker, a.basis.id)
            .await?
            .calculation_reason
            .as_deref(),
        Some("ACCOUNT_REGISTERED_AFTER_PERIOD_START")
    );

    f.close().await
}
#[tokio::test]
async fn key_scope_account_binding_current_roles_and_invalid_pages_fail_closed() -> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    let request = input(policy, account);
    let key = Uuid::new_v4();
    let a = f.f.capture_periodic_basis(&f.maker, key, &request).await?;
    let mut changed = request.clone();
    changed.period_start -= Duration::days(1);
    assert!(
        f.f.capture_periodic_basis(&f.maker, key, &changed)
            .await
            .is_err()
    );
    changed = request.clone();
    changed.account_id = Uuid::new_v4();
    assert!(
        f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &changed)
            .await
            .is_err()
    );
    changed = request.clone();
    changed.period_start += Duration::days(1);
    assert!(
        f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &changed)
            .await
            .is_err()
    );
    assert!(
        f.f.capture_periodic_basis(&f.checker, Uuid::new_v4(), &request)
            .await
            .is_err()
    );
    let mut customer = f.maker.clone();
    customer.trust = TrustContext::Customer;
    assert!(f.f.periodic_basis(&customer, a.basis.id).await.is_err());
    assert!(
        f.f.periodic_bases(&actor("unrelated-security"), None, None)
            .await
            .is_err()
    );
    assert!(
        f.f.periodic_basis_items(&f.maker, a.basis.id, 0, 0)
            .await
            .is_err()
    );
    assert!(
        f.f.periodic_basis_items(&f.maker, a.basis.id, 1, 100)
            .await
            .is_err()
    );
    let native = f.db.module(Module::Core).await?;
    assert!(
        sqlx::query("DELETE FROM core.periodic_bases")
            .execute(&native)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM core.periodic_bases")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    let person = f.f.controls.person(&f.maker).await?.0;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=clock_timestamp() WHERE person_id=$1 AND capability='REVIEW'").bind(person).execute(&f.db.migrator).await?;
    assert!(
        f.f.capture_periodic_basis(&f.maker, key, &request)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn rehashed_basis_claims_cannot_replace_actual_native_journal_facts() -> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    fund(&f, account, 10000).await?;
    let a =
        f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &input(policy, account))
            .await?;
    let mut raw: serde_json::Value =
        sqlx::query_scalar("SELECT document FROM core.periodic_bases WHERE id=$1")
            .bind(a.basis.id)
            .fetch_one(&f.db.migrator)
            .await?;
    raw["items"][0]["deltaMinor"] = "12000".into();
    raw["periodChange"] = "12000".into();
    raw["observedClosingBooked"] = "12000".into();
    raw["sourceDigest"] = fingerprint(&raw["items"])?.into();
    sqlx::query("ALTER TABLE core.periodic_bases DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE core.periodic_bases SET document=$2,digest=$3 WHERE id=$1")
        .bind(a.basis.id)
        .bind(&raw)
        .bind(fingerprint(&raw)?)
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("ALTER TABLE core.periodic_bases ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(f.f.periodic_basis(&f.maker, a.basis.id).await.is_err());
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 10000);
    f.close().await
}
#[tokio::test]
async fn native_source_bound_refuses_1001_without_storing_a_partial_basis() -> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    for _ in 0..1000 {
        fund(&f, account, 1).await?;
    }
    let key = Uuid::new_v4();
    let request = input(policy, account);
    let a = f.f.capture_periodic_basis(&f.maker, key, &request).await?;
    assert_eq!(a.basis.source_count, 1000);
    assert_eq!(a.basis.observed_closing_booked, "1000");
    let page =
        f.f.periodic_basis_items(&f.checker, a.basis.id, 900, 100)
            .await?;
    assert_eq!(page.items.len(), 100);
    assert!(page.next_after.is_none());
    assert_eq!(page.items[99].ordinal, 1000);
    fund(&f, account, 1).await?;
    let error =
        f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &request)
            .await
            .err()
            .unwrap();
    assert!(error.downcast_ref::<BasisLimit>().is_some());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core.periodic_bases")
        .fetch_one(&f.db.migrator)
        .await?;
    assert_eq!(count, 1);
    assert_eq!(
        f.f.capture_periodic_basis(&f.maker, key, &request)
            .await?
            .digest,
        a.digest
    );
    f.close().await
}
#[tokio::test]
async fn a_single_native_capture_view_stays_consistent_during_concurrent_financial_commits()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    let core = f.core.clone();
    let clearing = AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?);
    let writer = tokio::spawn(async move {
        for _ in 0..60 {
            core.fund_synthetic(Uuid::new_v4(), account, clearing, money(1))
                .await?;
        }
        Ok::<_, anyhow::Error>(())
    });
    for _ in 0..20 {
        let a =
            f.f.capture_periodic_basis(&f.maker, Uuid::new_v4(), &input(policy, account))
                .await?;
        assert_eq!(a.basis.current_booked, a.basis.observed_closing_booked);
        assert_eq!(
            a.basis.source_count.to_string(),
            a.basis.observed_closing_booked
        );
    }
    writer.await??;
    assert_eq!(f.core.balance(account).await?.booked.minor_units(), 60);
    f.close().await
}
#[tokio::test]
async fn lost_audit_ack_is_repaired_after_cursor_passes_fifty_damaged_artifacts() -> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    let a =
        f.f.bases
            .capture(
                &f.f.controls,
                &f.maker,
                Uuid::new_v4(),
                &input(policy, account),
            )
            .await?;
    for n in 1..=50 {
        sqlx::query("INSERT INTO core.periodic_bases(id,issuer,subject,client_id,request_key,actor_context,person_id,policy_id,account_id,period_start,document,digest,captured_at) SELECT $2,issuer,subject,client_id,$3,actor_context,person_id,policy_id,account_id,period_start,document,digest,captured_at FROM core.periodic_bases WHERE id=$1").bind(a.detail.basis.id).bind(Uuid::from_u128(n)).bind(Uuid::new_v4()).execute(&f.db.migrator).await?;
    }
    f.f.recover_basis_audit().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.audit_pending WHERE id=$1")
        .bind(a.detail.basis.id)
        .fetch_one(&f.f.pool)
        .await?;
    assert_eq!(count, 0);
    f.f.recover_basis_audit().await?;
    f.f.recover_basis_audit().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.audit_pending WHERE id=$1")
        .bind(a.detail.basis.id)
        .fetch_one(&f.f.pool)
        .await?;
    assert_eq!(count, 1);
    f.close().await
}
#[tokio::test]
async fn capture_authority_expiring_during_key_lock_wait_cannot_save_an_observation() -> Result<()>
{
    let f = Fixture::new().await?;
    let (policy, account) = setup(&f).await?;
    let key = Uuid::new_v4();
    let mut guard = f.db.migrator.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(jsonb_build_array($1::text,$2::text,$3::text,$4::uuid)::text,179))").bind(&f.maker.issuer).bind(&f.maker.subject).bind(&f.maker.client_id).bind(key).execute(&mut *guard).await?;
    let records = f.f.bases.clone();
    let controls = f.f.controls.clone();
    let maker = f.maker.clone();
    let request = input(policy, account);
    let task = tokio::spawn(async move { records.capture(&controls, &maker, key, &request).await });
    let core = f.db.module(Module::Core).await?;
    let mut waiting = false;
    for _ in 0..100 {
        waiting=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename=current_user AND wait_event_type='Lock' AND query LIKE '%core.capture_periodic_basis%')").fetch_one(&core).await?;
        if waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(waiting);
    let person = f.f.controls.person(&f.maker).await?.0;
    sqlx::query("UPDATE controls.role_grants SET valid_until=clock_timestamp()+interval '200 milliseconds' WHERE person_id=$1 AND role='finance_adjuster'").bind(person).execute(&f.db.migrator).await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    guard.commit().await?;
    assert!(task.await?.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core.periodic_bases")
        .fetch_one(&f.db.migrator)
        .await?;
    assert_eq!(count, 0);
    f.close().await
}
#[tokio::test]
async fn closed_monthly_basis_prices_the_exact_month_without_using_current_balance_or_posting()
-> Result<()> {
    let f = Fixture::new().await?;
    let (daily, account) = setup(&f).await?;
    let published = f.f.periodic.get(daily).await?.unwrap();
    let mut policy = published.policy;
    let first = Utc::now().date_naive().with_day(1).unwrap();
    let start = first.checked_sub_months(Months::new(1)).unwrap();
    policy.id = Uuid::new_v4();
    policy.code = "NATIVE_MONTHLY".into();
    policy.effective_from = start;
    policy.effective_until = first;
    policy.calendar.id = Uuid::new_v4();
    policy.calendar.coverage_from = start;
    policy.calendar.coverage_until = first + Duration::days(15);
    policy.calendar.holidays.clear();
    policy.calculation = Calculation::MonthlyFixedFee { amount: money(250) };
    let request = bank_finance::periodic::PolicyInput {
        policy: policy.clone(),
        reason: "Observe a closed synthetic month; no enrollment or financial posting".into(),
    };
    let p =
        f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &request)
            .await?;
    f.f.approve_periodic_policy(&f.checker, p.id, &p.policy_digest)
        .await?;
    f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
        .await?;
    // Account-registration fixture metadata represents an older account; no journal or balance is altered.
    sqlx::query("UPDATE core.account_registry SET created_at=$2 WHERE id=$1")
        .bind(account.0)
        .bind(
            (start - Duration::days(1))
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc(),
        )
        .execute(&f.db.migrator)
        .await?;
    fund(&f, account, 5000).await?;
    let before = f.core.balance(account).await?;
    let a =
        f.f.capture_periodic_basis(
            &f.maker,
            Uuid::new_v4(),
            &BasisInput {
                policy_id: policy.id,
                account_id: account.0,
                period_start: start,
            },
        )
        .await?;
    assert_eq!(a.basis.period_status, "CLOSED_PERIOD_OBSERVATION");
    assert_eq!(a.basis.current_booked, "5000");
    assert_eq!(a.basis.observed_closing_booked, "0");
    assert_eq!(a.basis.source_count, 0);
    let calculation = a.calculation.unwrap();
    assert_eq!(calculation.amount.minor_units(), 250);
    assert_eq!(calculation.exact_denominator, "1");
    assert!(!calculation.provisional);
    let after = f.core.balance(account).await?;
    assert_eq!(before.version, after.version);
    assert_eq!(before.booked, after.booked);
    f.close().await
}
