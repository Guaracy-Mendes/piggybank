use super::*;
use bank_core::{
    accruals::{BusinessCalendar, Calculation, PeriodicPolicy, PreviewInput},
    products::ProductVersion,
};
use bank_finance::periodic::PolicyInput;
use chrono::{Datelike, Months};
pub(super) async fn setup(f: &Fixture) -> Result<PolicyInput> {
    let today = Utc::now().date_naive();
    let start = today.with_day(1).unwrap();
    let end = start.checked_add_months(Months::new(1)).unwrap();
    let product = ProductVersion {
        id: Uuid::new_v4(),
        code: "PERIODIC_DEMO".into(),
        version: 1,
        name: "Synthetic periodic preview product".into(),
        label: "SYNTHETIC".into(),
        effective_from: start - Duration::days(400),
        effective_until: end + Duration::days(800),
        minimum_age: 18,
        payment_limit: money(10000),
        daily_limit: money(100000),
        flat_fee: money(0),
        fee_basis_points: 0,
        chart_version: "synthetic-chart-v1".into(),
    };
    for (actor, role) in [
        (&f.maker, "product_admin"),
        (&f.checker, "product_approver"),
    ] {
        let person = f.f.controls.person(actor).await?;
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&f.db.migrator).await?;
    }
    let proposal =
        f.f.controls
            .propose_change(
                &f.maker,
                ChangeKind::Product,
                product.id,
                &product,
                Utc::now() + Duration::hours(1),
            )
            .await?;
    f.f.controls.approve_change(&f.checker, proposal).await?;
    f.core
        .publish_product(&f.f.controls, &f.maker, proposal, &product)
        .await?;
    Ok(PolicyInput {
        policy: PeriodicPolicy {
            id: Uuid::new_v4(),
            code: "ACCOUNT_FEE".into(),
            version: 1,
            label: "SYNTHETIC".into(),
            activation: "PREVIEW_ONLY".into(),
            product_version_id: product.id,
            effective_from: start,
            effective_until: end,
            calendar: BusinessCalendar {
                id: Uuid::new_v4(),
                code: "TEST_UTC".into(),
                version: 1,
                timezone: "UTC".into(),
                coverage_from: start,
                coverage_until: end + Duration::days(100),
                business_weekdays: vec![1, 2, 3, 4, 5],
                holidays: vec![],
                cutoff: "17:00:00".into(),
                roll_convention: "FOLLOWING_14_DAYS".into(),
            },
            calculation: Calculation::MonthlyFixedFee {
                amount: money(9_007_199_254_740_993),
            },
        },
        reason: "Review synthetic periodic policy without enabling charges".into(),
    })
}
async fn approve(
    f: &Fixture,
    input: &PolicyInput,
) -> Result<bank_finance::periodic::PolicyRequest> {
    let p =
        f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), input)
            .await?;
    f.f.approve_periodic_policy(&f.checker, p.id, &p.policy_digest)
        .await
}
#[tokio::test]
async fn durable_registration_canonical_independence_and_core_receipt_replay() -> Result<()> {
    let f = Fixture::new().await?;
    let input = setup(&f).await?;
    let before = f.source(50000).await?;
    let source = f.core.financial_batch(before).await?;
    let key = Uuid::new_v4();
    let (a, b) = tokio::join!(
        f.f.propose_periodic_policy(&f.maker, key, &input),
        f.f.propose_periodic_policy(&f.maker, key, &input)
    );
    let a = a?;
    assert_eq!(a.id, b?.id);
    assert_eq!(a.state, "STAGED");
    f.f.controls
        .propose_change_id(
            &f.maker,
            a.id,
            ChangeKind::PeriodicPolicy,
            input.policy.id,
            &input.policy,
            a.expires_at,
        )
        .await?;
    assert!(
        f.f.periodic_policy_request(&f.maker, a.id)
            .await?
            .registered_at
            .is_none()
    );
    f.f.recover_periodic_policies().await?;
    assert!(
        f.f.periodic_policy_request(&f.maker, a.id)
            .await?
            .registered_at
            .is_some()
    );
    let alias = actor("maker-other-login");
    f.f.controls
        .link_person(&alias, f.f.controls.person(&f.maker).await?)
        .await?;
    let person = f.f.controls.person(&f.maker).await?;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'finance_approver',now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('finance-operations',$1,'APPROVE')").bind(person.0).execute(&f.db.migrator).await?;
    assert!(
        f.f.approve_periodic_policy(&alias, a.id, &a.policy_digest)
            .await
            .is_err()
    );
    assert!(
        f.f.approve_periodic_policy(&f.checker, a.id, &"0".repeat(64))
            .await
            .is_err()
    );
    let a =
        f.f.approve_periodic_policy(&f.checker, a.id, &a.policy_digest)
            .await?;
    // Core committed, but Finance/caller never observed an acknowledgment.
    let native =
        f.f.periodic
            .publish(&f.f.controls, &f.maker, a.id, &input.policy)
            .await?;
    let saved = f.f.periodic_policy_request(&f.maker, a.id).await?;
    assert_eq!(saved.state, "PUBLISHED");
    assert_eq!(saved.publication.unwrap().digest, native.digest);
    let (x, y) = tokio::join!(
        f.f.publish_periodic_policy(&f.maker, a.id, &a.policy_digest),
        f.f.publish_periodic_policy(&alias, a.id, &a.policy_digest)
    );
    assert_eq!(
        x?.publication.unwrap().digest,
        y?.publication.unwrap().digest
    );
    let replay = f.f.propose_periodic_policy(&f.maker, key, &input).await?;
    assert_eq!(replay.id, a.id);
    assert_eq!(replay.state, "PUBLISHED");
    let preview =
        f.f.preview_periodic_policy(
            &f.checker,
            a.id,
            &PreviewInput {
                period_start: input.policy.effective_from,
                closing_booked: None,
                evaluated_at: Utc::now(),
            },
        )
        .await?;
    assert_eq!(preview.amount.minor_units(), 9_007_199_254_740_993);
    assert_eq!(preview.activation, "PREVIEW_ONLY");
    assert_eq!(
        fingerprint(&f.core.financial_batch(before).await?)?,
        fingerprint(&source)?
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core.journal_batches")
        .fetch_one(&f.db.migrator)
        .await?;
    assert_eq!(count, 1);
    f.close().await
}
#[tokio::test]
async fn exact_key_scope_unpublished_preview_and_unrelated_roles_denied() -> Result<()> {
    let f = Fixture::new().await?;
    let input = setup(&f).await?;
    let key = Uuid::new_v4();
    let p = f.f.propose_periodic_policy(&f.maker, key, &input).await?;
    let mut changed = input.clone();
    changed.policy.calendar.cutoff = "16:00:00".into();
    assert!(
        f.f.propose_periodic_policy(&f.maker, key, &changed)
            .await
            .is_err()
    );
    changed = input.clone();
    changed.reason = "Another purpose for the same policy".into();
    assert!(
        f.f.propose_periodic_policy(&f.maker, key, &changed)
            .await
            .is_err()
    );
    assert!(
        f.f.preview_periodic_policy(
            &f.maker,
            p.id,
            &PreviewInput {
                period_start: input.policy.effective_from,
                closing_booked: None,
                evaluated_at: Utc::now()
            }
        )
        .await
        .is_err()
    );
    assert!(
        f.f.propose_periodic_policy(&f.checker, Uuid::new_v4(), &input)
            .await
            .is_err()
    );
    assert!(
        f.f.publish_periodic_policy(&f.checker, p.id, &p.policy_digest)
            .await
            .is_err()
    );
    let mut customer = f.maker.clone();
    customer.trust = TrustContext::Customer;
    assert!(f.f.periodic_policy_requests(&customer).await.is_err());
    assert!(
        f.f.periodic_policy_requests(&actor("unknown-security"))
            .await
            .is_err()
    );
    let mut invalid = input.clone();
    invalid.policy.activation = "ENABLED".into();
    assert!(
        f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &invalid)
            .await
            .is_err()
    );
    invalid = input.clone();
    invalid.policy.effective_until += Duration::days(1000);
    assert!(
        f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &invalid)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn publication_rechecks_current_roles_queues_and_exact_proposal_kind() -> Result<()> {
    let f = Fixture::new().await?;
    let input = setup(&f).await?;
    let p = approve(&f, &input).await?;
    let checker = f.f.controls.person(&f.checker).await?.0;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=now() WHERE person_id=$1 AND queue='finance-operations' AND capability='APPROVE'").bind(checker).execute(&f.db.migrator).await?;
    assert!(
        f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
            .await
            .is_err()
    );
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=NULL WHERE person_id=$1")
        .bind(checker)
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='finance_approver'").bind(checker).execute(&f.db.migrator).await?;
    assert!(
        f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
            .await
            .is_err()
    );
    sqlx::query("UPDATE controls.role_grants SET revoked_at=NULL WHERE person_id=$1")
        .bind(checker)
        .execute(&f.db.migrator)
        .await?;
    let wrong =
        f.f.controls
            .propose_change(
                &f.maker,
                ChangeKind::FinanceChart,
                input.policy.id,
                &input.policy,
                Utc::now() + Duration::hours(1),
            )
            .await?;
    f.f.controls.approve_change(&f.checker, wrong).await?;
    assert!(
        f.f.periodic
            .publish(&f.f.controls, &f.maker, wrong, &input.policy)
            .await
            .is_err()
    );
    let mut changed = input.policy.clone();
    changed.calendar.cutoff = "15:00:00".into();
    assert!(
        f.f.periodic
            .publish(&f.f.controls, &f.maker, p.id, &changed)
            .await
            .is_err()
    );
    f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
        .await?;
    let maker = f.f.controls.person(&f.maker).await?.0;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=now() WHERE person_id=$1 AND capability='REVIEW'").bind(maker).execute(&f.db.migrator).await?;
    assert!(
        f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn immutable_versions_resolve_exact_boundaries_and_reject_overlap_races() -> Result<()> {
    let f = Fixture::new().await?;
    let first = setup(&f).await?;
    let p = approve(&f, &first).await?;
    f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
        .await?;
    let mut second = first.clone();
    second.policy.id = Uuid::new_v4();
    second.policy.version = 2;
    second.policy.effective_from = first.policy.effective_until;
    second.policy.effective_until = second
        .policy
        .effective_from
        .checked_add_months(Months::new(1))
        .unwrap();
    second.policy.calculation = Calculation::MonthlyFixedFee { amount: money(500) };
    let a = approve(&f, &second).await?;
    let mut rival = second.clone();
    rival.policy.id = Uuid::new_v4();
    let b = approve(&f, &rival).await?;
    let (x, y) = tokio::join!(
        f.f.publish_periodic_policy(&f.maker, a.id, &a.policy_digest),
        f.f.publish_periodic_policy(&f.maker, b.id, &b.policy_digest)
    );
    assert_ne!(x.is_ok(), y.is_ok());
    let winner = if let Ok(x) = x { x } else { y? };
    let registry = &f.f.periodic;
    let product = first.policy.product_version_id;
    assert_eq!(
        registry
            .resolve(
                product,
                &first.policy.code,
                first.policy.effective_until - Duration::days(1)
            )
            .await?
            .unwrap()
            .policy
            .id,
        first.policy.id
    );
    assert_eq!(
        registry
            .resolve(product, &first.policy.code, first.policy.effective_until)
            .await?
            .unwrap()
            .policy
            .id,
        winner.policy.id
    );
    assert!(
        registry
            .resolve(product, &first.policy.code, second.policy.effective_until)
            .await?
            .is_none()
    );
    let mut overlap = second.clone();
    overlap.policy.id = Uuid::new_v4();
    overlap.policy.version = 3;
    let c = approve(&f, &overlap).await?;
    assert!(
        f.f.publish_periodic_policy(&f.maker, c.id, &c.policy_digest)
            .await
            .is_err()
    );
    let native = f.db.module(Module::Core).await?;
    assert!(
        sqlx::query("UPDATE core.periodic_policies SET version=99")
            .execute(&native)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM core.periodic_policies")
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE finance.periodic_policy_requests SET reason='changed reason'")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn bounded_cursor_advances_past_invalid_commands_and_repairs_lost_registration_ack()
-> Result<()> {
    let f = Fixture::new().await?;
    let input = setup(&f).await?;
    let p =
        f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &input)
            .await?;
    // Valid sealed commands whose original maker has lost the required queue capability.
    let bad = actor("poisoned-maker");
    f.f.controls.link_person(&bad, PartyId::new()).await?;
    let person = f.f.controls.person(&bad).await?.0;
    for n in 1..=50 {
        sqlx::query("INSERT INTO finance.periodic_policy_requests(id,policy_id,issuer,subject,client_id,request_key,actor_context,maker_person,policy,policy_digest,reason,expires_at) SELECT $2,policy_id,$3,$4,$5,$6,$7,$8,policy,policy_digest,reason,expires_at FROM finance.periodic_policy_requests WHERE id=$1").bind(p.id).bind(Uuid::from_u128(n)).bind(&bad.issuer).bind(&bad.subject).bind(&bad.client_id).bind(Uuid::new_v4()).bind(serde_json::to_value(&bad)?).bind(person).execute(&f.db.migrator).await?;
    }
    f.f.recover_periodic_policies().await?;
    assert!(
        f.f.periodic_policy_request(&f.maker, p.id)
            .await?
            .registered_at
            .is_none()
    );
    f.f.recover_periodic_policies().await?;
    assert!(
        f.f.periodic_policy_request(&f.maker, p.id)
            .await?
            .registered_at
            .is_some()
    );
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM finance.periodic_policy_requests WHERE registration_attempts=1 AND registered_at IS NULL").fetch_one(&f.f.pool).await?;
    assert_eq!(count, 50);
    f.close().await
}
#[tokio::test]
async fn native_function_binds_exact_document_and_expired_authority_cannot_publish() -> Result<()> {
    let f = Fixture::new().await?;
    let input = setup(&f).await?;
    let p = approve(&f, &input).await?;
    let maker = f.f.controls.person(&f.maker).await?.0;
    let checker = f.f.controls.person(&f.checker).await?.0;
    let mut altered = input.policy.clone();
    altered.calculation = Calculation::MonthlyFixedFee { amount: money(123) };
    let native = f.db.module(Module::Core).await?;
    assert!(
        sqlx::query("SELECT core.publish_periodic_policy($1,$2,$3,$4,$5)")
            .bind(serde_json::to_value(altered)?)
            .bind(&p.policy_digest)
            .bind(p.id)
            .bind(maker)
            .bind(checker)
            .execute(&native)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO core.periodic_policies SELECT * FROM core.periodic_policies")
            .execute(&native)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM core.periodic_policies")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    // Trusted clock fault: the original sealed request stays immutable while Controls' approval has expired.
    sqlx::query(
        "UPDATE controls.change_proposals SET expires_at=now()-interval '1 minute' WHERE id=$1",
    )
    .bind(p.id)
    .execute(&f.db.migrator)
    .await?;
    assert!(
        f.f.periodic
            .publish(&f.f.controls, &f.maker, p.id, &input.policy)
            .await
            .is_err()
    );
    assert!(
        f.f.publish_periodic_policy(&f.maker, p.id, &p.policy_digest)
            .await
            .is_err()
    );
    assert!(f.f.periodic.get(input.policy.id).await?.is_none());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core.journal_batches")
        .fetch_one(&f.db.migrator)
        .await?;
    assert_eq!(count, 0);
    f.close().await
}
#[tokio::test]
async fn publication_rechecks_expiry_after_waiting_for_the_native_rule_lock() -> Result<()> {
    let f = Fixture::new().await?;
    let input = setup(&f).await?;
    for role_expiry in [false, true] {
        let until = Utc::now() + Duration::seconds(if role_expiry { 3600 } else { 2 });
        let proposal =
            f.f.controls
                .propose_change(
                    &f.maker,
                    ChangeKind::PeriodicPolicy,
                    input.policy.id,
                    &input.policy,
                    until,
                )
                .await?;
        f.f.controls.approve_change(&f.checker, proposal).await?;
        let mut guard = f.db.migrator.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,176))")
            .bind(format!(
                "{}/{}",
                input.policy.product_version_id, input.policy.code
            ))
            .execute(&mut *guard)
            .await?;
        let registry = f.f.periodic.clone();
        let controls = f.f.controls.clone();
        let maker = f.maker.clone();
        let policy = input.policy.clone();
        let task =
            tokio::spawn(
                async move { registry.publish(&controls, &maker, proposal, &policy).await },
            );
        let mut waiting = false;
        for _ in 0..100 {
            waiting=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))").fetch_one(&f.db.migrator).await?;
            if waiting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(waiting, "native publication must reach its held rule lock");
        if role_expiry {
            let checker = f.f.controls.person(&f.checker).await?.0;
            sqlx::query("UPDATE controls.role_grants SET valid_until=clock_timestamp()+interval '1 second' WHERE person_id=$1 AND role='finance_approver'").bind(checker).execute(&f.db.migrator).await?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(if role_expiry {
            1200
        } else {
            2200
        }))
        .await;
        guard.commit().await?;
        assert!(
            task.await?.is_err(),
            "authority expiring during the lock wait must prevent a new publication"
        );
        assert!(f.f.periodic.get(input.policy.id).await?.is_none());
    }
    f.close().await
}
