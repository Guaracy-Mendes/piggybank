use bank_core::Core;
use bank_platform::{
    database::Module,
    domain::{AccountId, Currency, Money},
    testing::TestDatabase,
};
use sqlx::Executor;
use uuid::Uuid;
fn money(value: i64) -> Money {
    Money::new(value, Currency::synthetic())
}
#[tokio::test]
async fn ledger_balances_holds_and_postings_remain_atomic() -> anyhow::Result<()> {
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Core).await?;
    let core = Core::new(pool.clone());
    let alice = AccountId::new();
    let bob = AccountId::new();
    let clearing = AccountId::new();
    for (id, kind) in [
        (alice, "CUSTOMER"),
        (bob, "CUSTOMER"),
        (clearing, "SETTLEMENT"),
    ] {
        sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,$2,'ACTIVE',false)").bind(id.0).bind(kind).execute(&db.migrator).await?;
    }
    let funding = Uuid::new_v4();
    let funded = core
        .fund_synthetic(funding, alice, clearing, money(1000))
        .await?;
    assert_eq!(
        funded,
        core.fund_synthetic(funding, alice, clearing, money(1000))
            .await?
    );
    assert!(
        core.fund_synthetic(funding, alice, clearing, money(1001))
            .await
            .is_err()
    );
    assert!(
        pool.execute("UPDATE core.ledger_accounts SET booked=999999")
            .await
            .is_err()
    );
    assert!(
        pool.execute("DELETE FROM core.journal_entries")
            .await
            .is_err()
    );
    assert!(
        pool.execute("SELECT * FROM core.post(gen_random_uuid(),'BYPASS','[]'::jsonb)")
            .await
            .is_err()
    );
    assert!(
        db.migrator
            .execute("UPDATE core.journal_entries SET delta=delta")
            .await
            .is_err()
    );
    // Even an owner cannot commit an empty/unbalanced batch.
    let mut tx = db.migrator.begin().await?;
    sqlx::query("INSERT INTO core.journal_batches(operation_id,posting_kind,fingerprint) VALUES($1,'INVALID','x')").bind(Uuid::new_v4()).execute(&mut *tx).await?;
    assert!(tx.commit().await.is_err());
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let core = core.clone();
        tasks.push(tokio::spawn(async move {
            let op = Uuid::new_v4();
            (op, core.reserve(op, alice, money(100)).await)
        }));
    }
    let mut holds = Vec::new();
    for task in tasks {
        let (op, result) = task.await?;
        if result.is_ok() {
            holds.push(op);
        }
    }
    assert_eq!(holds.len(), 10);
    assert_eq!(core.balance(alice).await?.available.minor_units(), 0);
    let consumed = holds[0];
    let transfer = Uuid::new_v4();
    let batch = core
        .transfer(transfer, alice, bob, money(100), Some(consumed))
        .await?;
    assert_eq!(
        batch,
        core.transfer(transfer, alice, bob, money(100), Some(consumed))
            .await?
    );
    assert!(core.release(consumed).await.is_err());
    assert_eq!(
        core.posting_by_key(transfer, "INTERNAL").await?,
        Some(batch)
    );
    assert_eq!(core.balance(alice).await?.booked.minor_units(), 900);
    assert_eq!(core.balance(bob).await?.booked.minor_units(), 100);
    for hold in &holds[1..] {
        core.release(*hold).await?;
        core.release(*hold).await?;
    }
    assert_eq!(core.balance(alice).await?.available.minor_units(), 900);
    let unknown = Uuid::new_v4();
    core.reserve(unknown, alice, money(300)).await?;
    core.mark_submitted(unknown).await?;
    assert!(core.release(unknown).await.is_err());
    assert_eq!(core.balance(alice).await?.held.minor_units(), 300);
    // A fabricated evidence identifier is not exposed as a release authority.
    // This submitted hold remains reserved for the rail recovery package.
    // Credit restriction rolls back hold consumption, debit, batch and outbox together.
    let hold = Uuid::new_v4();
    core.reserve(hold, alice, money(200)).await?;
    sqlx::query("UPDATE core.ledger_accounts SET credit_blocked=true WHERE id=$1")
        .bind(bob.0)
        .execute(&db.migrator)
        .await?;
    let failed = Uuid::new_v4();
    assert!(
        core.transfer(failed, alice, bob, money(200), Some(hold))
            .await
            .is_err()
    );
    assert_eq!(core.balance(alice).await?.booked.minor_units(), 900);
    assert_eq!(core.balance(alice).await?.held.minor_units(), 500);
    assert!(core.posting_by_key(failed, "INTERNAL").await?.is_none());
    core.release(hold).await?;
    // Checked PostgreSQL integer arithmetic must reject overflow without a partial batch.
    assert!(
        core.fund_synthetic(Uuid::new_v4(), alice, clearing, money(i64::MAX))
            .await
            .is_err()
    );
    assert_eq!(core.balance(alice).await?.booked.minor_units(), 900);
    let total: i64 = sqlx::query_scalar("SELECT sum(booked)::bigint FROM core.ledger_accounts")
        .fetch_one(&pool)
        .await?;
    assert_eq!(total, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&pool)
            .await?,
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core.outbox WHERE document->>'eventType'='journal.batch-posted'"
        )
        .fetch_one(&pool)
        .await?,
        2
    );
    use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
    use bank_platform::domain::{ActorContext, TrustContext};
    let controls_pool = db.module(Module::Controls).await?;
    let controls = Entitlements::new(controls_pool.clone());
    let actor = |subject: &str| ActorContext {
        issuer: "urn:core-fixture".into(),
        subject: subject.into(),
        trust: TrustContext::Workforce,
        client_id: "staff-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let maker = actor("maker");
    let checker = actor("checker");
    for (who, role) in [(&maker, "finance_adjuster"), (&checker, "finance_approver")] {
        let person = Uuid::new_v4();
        sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
            .bind(&who.issuer)
            .bind(&who.subject)
            .bind(person)
            .execute(&db.migrator)
            .await?;
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
    }
    let details = bank_core::correction_details(batch);
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Correction,
            batch,
            &details,
            chrono::Utc::now() + chrono::Duration::hours(1),
        )
        .await?;
    assert!(controls.approve_change(&maker, proposal).await.is_err());
    assert!(
        controls
            .approved_change(&maker, proposal, ChangeKind::Correction, batch, &details)
            .await
            .is_err()
    );
    controls.approve_change(&checker, proposal).await?;
    let _approval = controls
        .approved_change(&maker, proposal, ChangeKind::Correction, batch, &details)
        .await?;
    assert!(
        core.reverse(Uuid::new_v4(), funded, &controls, &maker, proposal)
            .await
            .is_err()
    );
    // Remove the deliberately seeded credit restriction before the authorized reversal.
    sqlx::query("UPDATE core.ledger_accounts SET credit_blocked=false WHERE id=$1")
        .bind(bob.0)
        .execute(&db.migrator)
        .await?;
    let correction = Uuid::new_v4();
    let reversed = core
        .reverse(correction, batch, &controls, &maker, proposal)
        .await?;
    assert_eq!(
        reversed,
        core.reverse(correction, batch, &controls, &maker, proposal)
            .await?
    );
    assert!(
        core.reverse(Uuid::new_v4(), batch, &controls, &maker, proposal)
            .await
            .is_err()
    );
    assert_eq!(core.balance(alice).await?.booked.minor_units(), 1000);
    assert_eq!(core.balance(bob).await?.booked.minor_units(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT original_batch FROM core.journal_batches WHERE id=$1"
        )
        .bind(reversed)
        .fetch_one(&pool)
        .await?,
        batch
    );
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE role='finance_approver'")
        .execute(&db.migrator)
        .await?;
    assert!(
        controls
            .approved_change(&maker, proposal, ChangeKind::Correction, batch, &details)
            .await
            .is_err()
    );
    assert!(
        core.reverse(correction, batch, &controls, &maker, proposal)
            .await
            .is_err()
    );
    controls_pool.close().await;
    pool.close().await;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn published_product_pricing_is_versioned_and_independently_approved() -> anyhow::Result<()> {
    use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
    use bank_core::products::ProductVersion;
    use bank_platform::domain::{ActorContext, TrustContext};
    use chrono::{Duration, NaiveDate, Utc};
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Core).await?;
    let controls_pool = db.module(Module::Controls).await?;
    let core = Core::new(pool.clone());
    let controls = Entitlements::new(controls_pool.clone());
    let actor = |subject: &str| ActorContext {
        issuer: "urn:product-fixture".into(),
        subject: subject.into(),
        trust: TrustContext::Workforce,
        client_id: "staff-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let maker = actor("maker");
    let checker = actor("checker");
    let alias = actor("alias");
    let maker_id = Uuid::new_v4();
    let checker_id = Uuid::new_v4();
    for (who, id) in [
        (&maker, maker_id),
        (&alias, maker_id),
        (&checker, checker_id),
    ] {
        sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
            .bind(&who.issuer)
            .bind(&who.subject)
            .bind(id)
            .execute(&db.migrator)
            .await?;
    }
    for (person, role) in [
        (maker_id, "product_admin"),
        (maker_id, "product_approver"),
        (checker_id, "product_approver"),
    ] {
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
    }
    let date = |year| NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
    let product = ProductVersion {
        id: Uuid::new_v4(),
        code: "SYNTHETIC_CURRENT".into(),
        version: 1,
        name: "Synthetic current account".into(),
        label: "SYNTHETIC".into(),
        effective_from: date(2026),
        effective_until: date(2027),
        minimum_age: 18,
        payment_limit: money(100_000),
        daily_limit: money(500_000),
        flat_fee: money(2),
        fee_basis_points: 1000,
        chart_version: "synthetic-chart-v1".into(),
    };
    assert_eq!(product.fee(money(5))?.minor_units(), 3);
    assert!(product.fee(money(100_001)).is_err());
    assert!(!product.eligible(17, date(2026)));
    assert!(!product.eligible(30, date(2027)));
    assert!(product.eligible(18, date(2026)));
    let id = controls
        .propose_change(
            &maker,
            ChangeKind::Product,
            product.id,
            &product,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    assert!(
        core.publish_product(&controls, &maker, id, &product)
            .await
            .is_err()
    );
    assert!(controls.approve_change(&alias, id).await.is_err());
    controls.approve_change(&checker, id).await?;
    core.publish_product(&controls, &maker, id, &product)
        .await?;
    core.publish_product(&controls, &maker, id, &product)
        .await?;
    assert_eq!(core.products(date(2026)).await?.len(), 1);
    assert!(core.products(date(2027)).await?.is_empty());
    let mut next = product.clone();
    next.id = Uuid::new_v4();
    next.version = 2;
    next.effective_from = date(2027);
    next.effective_until = date(2028);
    next.flat_fee = money(7);
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Product,
            next.id,
            &next,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    controls.approve_change(&checker, proposal).await?;
    core.publish_product(&controls, &maker, proposal, &next)
        .await?;
    assert_eq!(
        core.product(product.id).await?.fee(money(5))?.minor_units(),
        3
    );
    assert_eq!(
        core.products(date(2027)).await?[0]
            .fee(money(5))?
            .minor_units(),
        8
    );
    let mut overlap = product.clone();
    overlap.id = Uuid::new_v4();
    overlap.version = 3;
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Product,
            overlap.id,
            &overlap,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    controls.approve_change(&checker, proposal).await?;
    assert!(
        core.publish_product(&controls, &maker, proposal, &overlap)
            .await
            .is_err()
    );
    let mut changed = product.clone();
    changed.flat_fee = money(100);
    assert!(
        core.publish_product(&controls, &maker, id, &changed)
            .await
            .is_err()
    );
    assert!(
        pool.execute("DELETE FROM core.product_versions")
            .await
            .is_err()
    );
    assert!(
        db.migrator
            .execute("UPDATE core.product_versions SET version=99")
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.product_versions")
            .fetch_one(&pool)
            .await?,
        2
    );
    pool.close().await;
    controls_pool.close().await;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn canonical_parties_account_activation_and_closure_are_guarded() -> anyhow::Result<()> {
    use bank_controls::entitlements::{Action, Entitlements};
    use bank_core::accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest};
    use bank_platform::domain::{ActorContext, TrustContext, fingerprint};
    use chrono::{Duration, Utc};
    use futures_util::future::BoxFuture;
    struct Gates(bool);
    impl OpeningPolicy for Gates {
        fn validate<'a>(
            &'a self,
            r: &'a OpeningRequest,
        ) -> BoxFuture<'a, anyhow::Result<ActivationEvidence>> {
            Box::pin(async move {
                anyhow::ensure!(self.0, "Required control/terms gate denied");
                Ok(ActivationEvidence {
                    application_id: r.application_id,
                    party_id: r.party_id,
                    product_version_id: r.product_version_id,
                    acceptance_id: r.acceptance_id,
                    controls_revision: "synthetic-check-set-v1".into(),
                    expires_at: Utc::now() + Duration::minutes(5),
                })
            })
        }
    }
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Core).await?;
    let core = Core::new(pool.clone());
    let cp = db.module(Module::Controls).await?;
    let controls = Entitlements::new(cp.clone());
    let actor = |subject: &str| ActorContext {
        issuer: "urn:account-fixture".into(),
        subject: subject.into(),
        trust: TrustContext::Customer,
        client_id: "customer-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let alice = actor("alice");
    let bob = actor("bob");
    let application = Uuid::new_v4();
    let digest = fingerprint(
        &serde_json::json!({"name":"Synthetic Applicant","email":"same@example.invalid"}),
    )?;
    let person = core.ensure_party(&alice, application, &digest).await?;
    assert_eq!(
        person,
        core.ensure_party(&alice, application, &digest).await?
    );
    let other = core.ensure_party(&bob, Uuid::new_v4(), &digest).await?;
    assert_ne!(person, other);
    assert!(
        core.ensure_party(&alice, application, &"a".repeat(64))
            .await
            .is_err()
    );
    controls.link_person(&alice, person).await?;
    assert!(controls.link_person(&alice, other).await.is_err());
    controls.link_person(&bob, other).await?;
    let prelinked = actor("prelinked");
    let canonical = bank_platform::domain::PartyId::new();
    controls.link_person(&prelinked, canonical).await?;
    assert_eq!(
        core.ensure_party_linked(&prelinked, Uuid::new_v4(), &digest, Some(canonical))
            .await?,
        canonical
    );
    assert!(
        core.ensure_party_linked(
            &actor("another-subject"),
            Uuid::new_v4(),
            &digest,
            Some(canonical)
        )
        .await
        .is_err()
    );
    assert!(
        core.ensure_party_linked(&alice, application, &digest, Some(canonical))
            .await
            .is_err()
    );

    let product = Uuid::new_v4();
    // Published fixture metadata: governance itself is exercised in the product test.
    sqlx::query("INSERT INTO core.product_versions(id,code,version,effective_from,effective_until,document,approval_id,maker_person,checker_person) VALUES($1,'FIXTURE',1,current_date-1,current_date+30,'{\"label\":\"SYNTHETIC\",\"chartVersion\":\"synthetic-chart-v1\"}',gen_random_uuid(),gen_random_uuid(),gen_random_uuid())").bind(product).execute(&db.migrator).await?;
    let request = OpeningRequest {
        application_id: application,
        party_id: person,
        product_version_id: product,
        acceptance_id: Uuid::new_v4(),
    };
    let mut tasks = vec![];
    for _ in 0..10 {
        let core = core.clone();
        let request = request.clone();
        tasks.push(tokio::spawn(
            async move { core.stage_account(&request).await },
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for t in tasks {
        ids.insert(t.await??);
    }
    assert_eq!(ids.len(), 1);
    let account = *ids.iter().next().unwrap();
    assert_eq!(core.accounts_for(person).await?[0].state, "PROVISIONING");
    assert!(
        core.reserve(Uuid::new_v4(), account, money(1))
            .await
            .is_err()
    );
    let mut changed = request.clone();
    changed.acceptance_id = Uuid::new_v4();
    assert!(core.stage_account(&changed).await.is_err());
    assert!(
        core.activate(account, &request, &Gates(false))
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT core.activate_account($1,'{\"controlsRevision\":\"fake\"}'::jsonb)")
            .bind(account.0)
            .execute(&pool)
            .await
            .is_err()
    );
    core.activate(account, &request, &Gates(true)).await?;
    core.activate(account, &request, &Gates(true)).await?;
    let authority = core.authority(account, Action::ReadAccount).await?;
    assert!(
        controls
            .authorize(&alice, Action::ReadAccount, Some(&authority), account.0)
            .await?
    );
    assert!(
        !controls
            .authorize(&bob, Action::ReadAccount, Some(&authority), account.0)
            .await?
    );
    let obligation = Uuid::new_v4();
    core.obligation(obligation, account, "UNSETTLED_PAYMENT", false)
        .await?;
    assert!(
        core.close_account(&alice, &controls, account)
            .await
            .is_err()
    );
    core.obligation(obligation, account, "UNSETTLED_PAYMENT", true)
        .await?;
    let clearing = AccountId::new();
    sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'SETTLEMENT','ACTIVE',false)").bind(clearing.0).execute(&db.migrator).await?;
    let funded = core
        .fund_synthetic(Uuid::new_v4(), account, clearing, money(100))
        .await?;
    assert!(
        core.close_account(&alice, &controls, account)
            .await
            .is_err()
    );
    // A selected snapshot retains its exact movement set across later commits.
    core.fund_synthetic(Uuid::new_v4(), account, clearing, money(1))
        .await?;
    core.fund_synthetic(Uuid::new_v4(), account, clearing, money(2))
        .await?;
    let range = (
        Utc::now().date_naive() - Duration::days(1),
        Utc::now().date_naive() + Duration::days(1),
    );
    let first = core
        .history(&alice, &controls, account, range, 1, None)
        .await?;
    assert_eq!(first.total, 3);
    let snapshot = first.next.unwrap();
    core.fund_synthetic(Uuid::new_v4(), account, clearing, money(3))
        .await?;
    let second = core
        .history(&alice, &controls, account, range, 1, Some(snapshot))
        .await?;
    let third = core
        .history(&alice, &controls, account, range, 1, second.next)
        .await?;
    assert!(third.next.is_none());
    let ids = std::collections::HashSet::from([
        first.items[0].id.clone(),
        second.items[0].id.clone(),
        third.items[0].id.clone(),
    ]);
    assert_eq!(ids.len(), 3);
    assert_eq!(
        core.history(&alice, &controls, account, range, 100, None)
            .await?
            .total,
        4
    );
    assert!(
        core.history(&bob, &controls, account, range, 1, Some(snapshot))
            .await
            .is_err()
    );
    let hold = Uuid::new_v4();
    core.reserve(hold, account, money(10)).await?;
    assert!(
        core.close_account(&alice, &controls, account)
            .await
            .is_err()
    );
    core.release(hold).await?;
    core.restrict(&alice, &controls, account, true, false)
        .await?;
    assert!(
        core.reserve(Uuid::new_v4(), account, money(1))
            .await
            .is_err()
    );
    assert!(
        core.restrict(&alice, &controls, account, false, false)
            .await
            .is_err()
    );
    // A customer cannot clear a compliance credit block by requesting a debit freeze.
    sqlx::query("UPDATE core.ledger_accounts SET credit_blocked=true WHERE id=$1")
        .bind(account.0)
        .execute(&db.migrator)
        .await?;
    assert!(
        core.restrict(&alice, &controls, account, true, false)
            .await
            .is_err()
    );
    // Fixture reconciliation removes value through owner-only balanced posting, never a balance edit.
    sqlx::query(
        "UPDATE core.ledger_accounts SET debit_blocked=false,credit_blocked=false WHERE id=$1",
    )
    .bind(account.0)
    .execute(&db.migrator)
    .await?;
    let remaining = core.balance(account).await?.booked.minor_units();
    sqlx::query("SELECT * FROM core.post($1,'FIXTURE_RECONCILIATION',$2,NULL,$3)").bind(Uuid::new_v4()).bind(serde_json::json!([{"accountId":account,"delta":-remaining},{"accountId":clearing,"delta":remaining}])).bind(funded).execute(&db.migrator).await?;
    assert!(core.close_account(&bob, &controls, account).await.is_err());
    core.close_account(&alice, &controls, account).await?;
    core.close_account(&alice, &controls, account).await?;
    assert_eq!(core.accounts_for(person).await?[0].state, "CLOSED");
    assert!(
        core.obligation(Uuid::new_v4(), account, "LATE_INBOUND", false)
            .await
            .is_err()
    );
    assert!(
        core.fund_synthetic(Uuid::new_v4(), account, clearing, money(1))
            .await
            .is_err()
    );
    pool.close().await;
    cp.close().await;
    db.close().await?;
    Ok(())
}
