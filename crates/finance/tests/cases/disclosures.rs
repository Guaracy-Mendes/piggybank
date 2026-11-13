use super::*;
use bank_controls::entitlements::Action;
use bank_core::accruals::disclosures::DisclosureLimit;
use serde_json::Value;
async fn customer(f: &Fixture, account: AccountId) -> Result<ActorContext> {
    let mut actor = super::actor("basis-customer");
    actor.trust = TrustContext::Customer;
    actor.client_id = "customer-web".into();
    let person: Uuid = sqlx::query_scalar(
        "SELECT person_id FROM core.account_relationships WHERE account_id=$1 AND kind='OWNER'",
    )
    .bind(account.0)
    .fetch_one(&f.core.pool)
    .await?;
    sqlx::query("INSERT INTO controls.person_links(issuer,subject,person_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(&actor.issuer).bind(&actor.subject).bind(person).execute(&f.db.migrator).await?;
    Ok(actor)
}
#[tokio::test]
async fn customer_reads_exact_approved_material_and_private_approval_people_never_escape()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let before:Value=sqlx::query_scalar("SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held,version) ORDER BY id) FROM core.ledger_accounts))").fetch_one(&f.core.pool).await?;
    let list = f
        .core
        .periodic_disclosures(&f.f.controls, &actor, account)
        .await?;
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].material.policy.id, policy);
    let first = f
        .core
        .periodic_disclosure(&f.f.controls, &actor, account, policy)
        .await?;
    let second = f
        .core
        .periodic_disclosure(&f.f.controls, &actor, account, policy)
        .await?;
    assert_eq!(first.digest, second.digest);
    assert_eq!(first.digest, list.items[0].digest);
    assert!(first.material.text.contains("3.65%"));
    assert!(first.material.text.contains("UTC"));
    let wire = serde_json::to_string(&first)?;
    for private in [
        "makerPerson",
        "checkerPerson",
        "proposalId",
        "capturedPersonId",
    ] {
        assert!(!wire.contains(private));
    }
    let after:Value=sqlx::query_scalar("SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held,version) ORDER BY id) FROM core.ledger_accounts))").fetch_one(&f.core.pool).await?;
    assert_eq!(before, after);
    f.close().await
}
#[tokio::test]
async fn foreign_staff_service_unknown_and_revoked_ownership_cannot_read_customer_disclosures()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let mut foreign = actor.clone();
    foreign.subject = "other-customer".into();
    for who in [&foreign, &f.maker, &f.checker] {
        assert!(
            f.core
                .periodic_disclosure(&f.f.controls, who, account, policy)
                .await
                .is_err()
        );
    }
    let mut service = actor.clone();
    service.trust = TrustContext::Service;
    assert!(
        f.core
            .periodic_disclosures(&f.f.controls, &service, account)
            .await
            .is_err()
    );
    assert!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, Uuid::new_v4())
            .await
            .is_err()
    );
    assert!(
        f.core
            .periodic_disclosures(&f.f.controls, &actor, AccountId::new())
            .await
            .is_err()
    );
    assert!(
        f.f.controls
            .authorize(
                &actor,
                Action::ReadAccount,
                Some(&f.core.authority(account, Action::ReadAccount).await?),
                account.0
            )
            .await?
    );
    sqlx::query("UPDATE core.account_relationships SET revoked_at=clock_timestamp() WHERE account_id=$1 AND kind='OWNER'").bind(account.0).execute(&f.db.migrator).await?;
    assert!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn mismatched_product_and_unpublished_material_are_hidden_and_tampered_native_policy_fails_closed()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let original_policy = f
        .core
        .periodic_policies()
        .get(policy)
        .await?
        .unwrap()
        .policy;
    let mut draft = original_policy.clone();
    draft.id = Uuid::new_v4();
    draft.code = "UNPUBLISHED_DISCLOSURE".into();
    let input = bank_finance::periodic::PolicyInput {
        policy: draft.clone(),
        reason: "Staged disclosure must not become a published customer offer".into(),
    };
    f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &input)
        .await?;
    assert!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, draft.id)
            .await
            .is_err()
    );
    let mut other_product = f.core.product(original_policy.product_version_id).await?;
    other_product.id = Uuid::new_v4();
    other_product.code = "DISCLOSURE_OTHER_PRODUCT".into();
    let proposal =
        f.f.controls
            .propose_change(
                &f.maker,
                ChangeKind::Product,
                other_product.id,
                &other_product,
                Utc::now() + Duration::hours(1),
            )
            .await?;
    f.f.controls.approve_change(&f.checker, proposal).await?;
    f.core
        .publish_product(&f.f.controls, &f.maker, proposal, &other_product)
        .await?;
    let mut other_policy = original_policy;
    other_policy.id = Uuid::new_v4();
    other_policy.product_version_id = other_product.id;
    other_policy.code = "OTHER_PRODUCT_DISCLOSURE".into();
    let input = bank_finance::periodic::PolicyInput {
        policy: other_policy.clone(),
        reason: "Published different-product policy must remain private to its accounts".into(),
    };
    let request =
        f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &input)
            .await?;
    f.f.approve_periodic_policy(&f.checker, request.id, &request.policy_digest)
        .await?;
    f.f.publish_periodic_policy(&f.maker, request.id, &request.policy_digest)
        .await?;
    assert!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, other_policy.id)
            .await
            .is_err()
    );
    sqlx::query("ALTER TABLE core.periodic_policies DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE core.periodic_policies SET document=jsonb_set(document,'{calculation,annualBasisPoints}','999') WHERE id=$1").bind(policy).execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE core.periodic_policies ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn one_hundred_current_or_upcoming_rules_are_complete_and_one_hundred_one_refuses_truncation()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let original = f
        .core
        .periodic_policies()
        .get(policy)
        .await?
        .unwrap()
        .policy;
    for n in 1..=100 {
        if n == 100 {
            assert_eq!(
                f.core
                    .periodic_disclosures(&f.f.controls, &actor, account)
                    .await?
                    .items
                    .len(),
                100
            );
        }
        let mut p = original.clone();
        p.id = Uuid::new_v4();
        p.code = format!("DISC_BOUND_{n}");
        p.calendar.id = Uuid::new_v4();
        let input = bank_finance::periodic::PolicyInput {
            policy: p,
            reason: "Synthetic disclosure bound; charges remain disabled".into(),
        };
        let requested =
            f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &input)
                .await?;
        f.f.approve_periodic_policy(&f.checker, requested.id, &requested.policy_digest)
            .await?;
        f.f.publish_periodic_policy(&f.maker, requested.id, &requested.policy_digest)
            .await?;
    }
    let e = f
        .core
        .periodic_disclosures(&f.f.controls, &actor, account)
        .await
        .err()
        .unwrap();
    assert!(e.downcast_ref::<DisclosureLimit>().is_some());
    assert_eq!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await?
            .material
            .policy
            .id,
        policy
    );
    f.close().await
}

#[tokio::test]
async fn upcoming_version_and_ended_rule_do_not_rewrite_a_saved_disclosure() -> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let old = f
        .core
        .periodic_disclosure(&f.f.controls, &actor, account, policy)
        .await?;
    let mut next = old.material.policy.clone();
    next.id = Uuid::new_v4();
    next.version = 2;
    next.effective_from = next.effective_until;
    next.effective_until = next.effective_from + Duration::days(30);
    next.calendar.id = Uuid::new_v4();
    next.calendar.version = 2;
    next.calendar.coverage_from = next.effective_from;
    next.calendar.coverage_until = next.effective_until + Duration::days(15);
    next.calendar.holidays.clear();
    let mut ended = old.material.policy.clone();
    ended.id = Uuid::new_v4();
    ended.code = "ENDED_DISCLOSURE".into();
    ended.effective_from = Utc::now().date_naive() - Duration::days(20);
    ended.effective_until = Utc::now().date_naive() - Duration::days(10);
    ended.calendar.id = Uuid::new_v4();
    ended.calendar.coverage_from = ended.effective_from;
    ended.calendar.coverage_until = ended.effective_until + Duration::days(15);
    ended.calendar.holidays.clear();
    for p in [&next, &ended] {
        let input=bank_finance::periodic::PolicyInput{policy:p.clone(),reason:"Preview version and saved disclosure stability; no enrollment or financial activation".into()};
        let r =
            f.f.propose_periodic_policy(&f.maker, Uuid::new_v4(), &input)
                .await?;
        f.f.approve_periodic_policy(&f.checker, r.id, &r.policy_digest)
            .await?;
        f.f.publish_periodic_policy(&f.maker, r.id, &r.policy_digest)
            .await?;
    }
    let page = f
        .core
        .periodic_disclosures(&f.f.controls, &actor, account)
        .await?;
    assert_eq!(page.items.len(), 2);
    assert!(
        page.items
            .iter()
            .any(|d| d.material.policy.id == next.id && d.availability == "UPCOMING_PREVIEW")
    );
    assert!(!page.items.iter().any(|d| d.material.policy.id == ended.id));
    assert_eq!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, ended.id)
            .await?
            .availability,
        "ENDED_PREVIEW"
    );
    assert_eq!(
        old.digest,
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await?
            .digest
    );
    f.close().await
}
