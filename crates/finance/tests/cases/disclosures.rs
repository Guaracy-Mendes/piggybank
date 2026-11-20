use super::*;
use bank_controls::entitlements::Action;
use bank_core::accruals::disclosures::DisclosureLimit;
use serde_json::Value;
pub(super) async fn customer(f: &Fixture, account: AccountId) -> Result<ActorContext> {
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
async fn reviews(f: &Fixture) -> Result<bank_documents::Terms> {
    Ok(bank_documents::Terms::new(
        f.db.module(Module::Documents).await?,
    ))
}
fn review_input(digest: &str) -> bank_documents::periodic::ReviewInput {
    bank_documents::periodic::ReviewInput {
        disclosure_digest: digest.into(),
        acknowledged: true,
    }
}
async fn review_count(f: &Fixture) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM documents.periodic_preview_reviews")
            .fetch_one(&f.db.migrator)
            .await?,
    )
}
#[tokio::test]
async fn preview_review_native_renderer_matches_original_daily_and_exact_monthly_v1_without_locale_drift()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let terms = reviews(&f).await?;
    let disclosure = f
        .core
        .periodic_disclosure(&f.f.controls, &actor, account, policy)
        .await?;
    let before:Value=sqlx::query_scalar("SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held,version) ORDER BY id) FROM core.ledger_accounts))").fetch_one(&f.db.migrator).await?;
    let first = terms
        .acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &actor,
            (account, policy, Uuid::new_v4()),
            &review_input(&disclosure.digest),
        )
        .await?;
    assert_eq!(
        serde_json::to_value(&first.review.material)?,
        serde_json::to_value(&disclosure.material)?
    );
    first.validate()?;
    let mut monthly = disclosure.material.policy.clone();
    monthly.id = Uuid::new_v4();
    monthly.code = "REVIEW_MONTHLY".into();
    use chrono::Datelike;
    monthly.effective_from = Utc::now().date_naive().with_day(1).unwrap();
    monthly.effective_until = monthly
        .effective_from
        .checked_add_months(chrono::Months::new(1))
        .unwrap();
    monthly.calendar.coverage_from = monthly.effective_from;
    monthly.calendar.coverage_until = monthly.effective_until + Duration::days(15);
    monthly.calendar.holidays = vec![];
    monthly.calculation = bank_core::accruals::Calculation::MonthlyFixedFee {
        amount: money(9_007_199_254_740_993),
    };
    let staged =
        f.f.propose_periodic_policy(
            &f.maker,
            Uuid::new_v4(),
            &bank_finance::periodic::PolicyInput {
                policy: monthly.clone(),
                reason: "Independently publish the exact monthly review fixture".into(),
            },
        )
        .await?;
    f.f.approve_periodic_policy(&f.checker, staged.id, &staged.policy_digest)
        .await?;
    f.f.publish_periodic_policy(&f.maker, staged.id, &staged.policy_digest)
        .await?;
    let rendered = f
        .core
        .periodic_disclosure(&f.f.controls, &actor, account, monthly.id)
        .await?;
    let mut tx = f.core.pool.begin().await?;
    sqlx::query("SET LOCAL DateStyle='German, DMY'")
        .execute(&mut *tx)
        .await?;
    let native: Value = sqlx::query_scalar("SELECT core.periodic_disclosure_material_v1($1,$2)")
        .bind(account.0)
        .bind(monthly.id)
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(native, serde_json::to_value(&rendered.material)?);
    tx.rollback().await?;
    let second = terms
        .acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &actor,
            (account, monthly.id, Uuid::new_v4()),
            &review_input(&rendered.digest),
        )
        .await?;
    assert!(
        second
            .review
            .material
            .text
            .contains("90071992547409.93 (9007199254740993 minor units)")
    );
    let public = serde_json::to_string(&second)?;
    for private in [
        "issuer",
        "subject",
        "personId",
        "actorContext",
        "makerPerson",
        "checkerPerson",
    ] {
        assert!(!public.contains(private));
    }
    let after:Value=sqlx::query_scalar("SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held,version) ORDER BY id) FROM core.ledger_accounts))").fetch_one(&f.db.migrator).await?;
    assert_eq!(before, after);
    f.close().await
}
#[tokio::test]
async fn preview_review_concurrent_keys_and_canonical_aliases_recover_one_original_receipt_and_event()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let terms = reviews(&f).await?;
    let input = review_input(
        &f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await?
            .digest,
    );
    let key = Uuid::new_v4();
    let (a, b) = tokio::join!(
        terms.acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &actor,
            (account, policy, key),
            &input
        ),
        terms.acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &actor,
            (account, policy, Uuid::new_v4()),
            &input
        )
    );
    let (a, b) = (a?, b?);
    assert_eq!(a.digest, b.digest);
    assert_eq!(a.review.id, b.review.id);
    assert_eq!(a.review.reviewed_at, b.review.reviewed_at);
    let mut alias = actor.clone();
    alias.subject = "same-human-review-alias".into();
    alias.client_id = "another-customer-client".into();
    let person = f.f.controls.person(&actor).await?;
    sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
        .bind(&alias.issuer)
        .bind(&alias.subject)
        .bind(person.0)
        .execute(&f.db.migrator)
        .await?;
    let c = terms
        .acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &alias,
            (account, policy, Uuid::new_v4()),
            &input,
        )
        .await?;
    assert_eq!(a.digest, c.digest);
    let replay = terms
        .acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &actor,
            (account, policy, key),
            &input,
        )
        .await?;
    assert_eq!(a.digest, replay.digest);
    assert_eq!(review_count(&f).await?, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM documents.periodic_review_requests")
            .fetch_one(&terms.pool)
            .await?,
        3
    );
    let event: Value =
        sqlx::query_scalar("SELECT document FROM documents.outbox WHERE event_id=$1")
            .bind(a.review.id)
            .fetch_one(&terms.pool)
            .await?;
    let typed: bank_platform::domain::EventEnvelope = serde_json::from_value(event.clone())?;
    typed.validate()?;
    assert_eq!(typed.event_type, "periodic.preview.reviewed");
    assert_eq!(event["data"]["reviewId"], json!(a.review.id));
    let stored: String =
        sqlx::query_scalar("SELECT fingerprint FROM documents.outbox WHERE event_id=$1")
            .bind(a.review.id)
            .fetch_one(&terms.pool)
            .await?;
    assert_eq!(stored, fingerprint(&event)?);
    assert!(!serde_json::to_string(&event)?.contains("basis-customer"));
    assert!(!serde_json::to_string(&event)?.contains("material\":"));
    assert!(
        terms
            .acknowledge_periodic_preview(
                &f.core,
                &f.f.controls,
                &actor,
                (account, policy, key),
                &review_input(&"0".repeat(64))
            )
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn preview_review_requires_explicit_exact_material_current_owner_and_narrow_native_grants()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let terms = reviews(&f).await?;
    let input = review_input(
        &f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await?
            .digest,
    );
    let mut no = review_input(&input.disclosure_digest);
    no.acknowledged = false;
    assert!(
        terms
            .acknowledge_periodic_preview(
                &f.core,
                &f.f.controls,
                &actor,
                (account, policy, Uuid::new_v4()),
                &no
            )
            .await
            .is_err()
    );
    for digest in ["0".repeat(64), "bad".into()] {
        assert!(
            terms
                .acknowledge_periodic_preview(
                    &f.core,
                    &f.f.controls,
                    &actor,
                    (account, policy, Uuid::new_v4()),
                    &review_input(&digest)
                )
                .await
                .is_err()
        );
    }
    assert!(
        terms
            .acknowledge_periodic_preview(
                &f.core,
                &f.f.controls,
                &actor,
                (account, policy, Uuid::nil()),
                &input
            )
            .await
            .is_err()
    );
    let mut foreign = actor.clone();
    foreign.subject = "foreign-preview-reviewer".into();
    let mut service = actor.clone();
    service.trust = TrustContext::Service;
    for who in [&foreign, &f.maker, &service] {
        assert!(
            terms
                .acknowledge_periodic_preview(
                    &f.core,
                    &f.f.controls,
                    who,
                    (account, policy, Uuid::new_v4()),
                    &input
                )
                .await
                .is_err()
        );
        assert!(
            terms
                .periodic_preview_review(&f.core, &f.f.controls, who, account, policy)
                .await
                .is_err()
        );
    }
    assert!(
        terms
            .periodic_preview_review(&f.core, &f.f.controls, &actor, account, policy)
            .await
            .is_err()
    );
    let result =
        sqlx::query("SELECT documents.acknowledge_periodic_preview($1,$2,$3,$4,$5,$6,true)")
            .bind(account.0)
            .bind(policy)
            .bind(Uuid::new_v4())
            .bind(serde_json::to_value(&actor)?)
            .bind(Uuid::new_v4())
            .bind(&input.disclosure_digest)
            .execute(&terms.pool)
            .await;
    assert!(result.is_err());
    let allowed:bool=sqlx::query_scalar("SELECT has_function_privilege('bank_core','documents.acknowledge_periodic_preview(uuid,uuid,uuid,jsonb,uuid,text,boolean)','EXECUTE')").fetch_one(&f.db.migrator).await?;
    assert!(!allowed);
    assert_eq!(review_count(&f).await?, 0);
    f.close().await
}
#[tokio::test]
async fn preview_review_retains_original_material_after_source_damage_and_recovers_post_commit_retries()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let terms = reviews(&f).await?;
    let disclosure = f
        .core
        .periodic_disclosure(&f.f.controls, &actor, account, policy)
        .await?;
    let input = review_input(&disclosure.digest);
    let key = Uuid::new_v4();
    let original = terms
        .acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &actor,
            (account, policy, key),
            &input,
        )
        .await?;
    sqlx::query("ALTER TABLE core.periodic_policies DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE core.periodic_policies SET document=jsonb_set(document,'{calculation,annualBasisPoints}','9999') WHERE id=$1").bind(policy).execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE core.periodic_policies ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(
        f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await
            .is_err()
    );
    let archived = terms
        .periodic_preview_review(&f.core, &f.f.controls, &actor, account, policy)
        .await?;
    assert_eq!(archived.digest, original.digest);
    assert_eq!(archived.review.material_digest, disclosure.digest);
    for k in [key, Uuid::new_v4()] {
        assert_eq!(
            terms
                .acknowledge_periodic_preview(
                    &f.core,
                    &f.f.controls,
                    &actor,
                    (account, policy, k),
                    &input
                )
                .await?
                .digest,
            original.digest
        );
    }
    assert_eq!(review_count(&f).await?, 1);
    sqlx::query("ALTER TABLE documents.periodic_preview_reviews DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE documents.periodic_preview_reviews SET document=jsonb_set(document,'{material,text}','\"changed archived material\"')").execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE documents.periodic_preview_reviews ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(
        terms
            .periodic_preview_review(&f.core, &f.f.controls, &actor, account, policy)
            .await
            .is_err()
    );
    assert!(
        terms
            .acknowledge_periodic_preview(
                &f.core,
                &f.f.controls,
                &actor,
                (account, policy, key),
                &input
            )
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn preview_review_rechecks_owner_after_a_real_native_request_lock_wait() -> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let terms = reviews(&f).await?;
    let input = review_input(
        &f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await?
            .digest,
    );
    let key = Uuid::new_v4();
    let mut block = f.db.migrator.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(jsonb_build_array($1::text,$2::text,$3::text,$4::uuid)::text,193))").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).execute(&mut *block).await?;
    let core = f.core.clone();
    let controls = f.f.controls.clone();
    let task = tokio::spawn(async move {
        terms
            .acknowledge_periodic_preview(&core, &controls, &actor, (account, policy, key), &input)
            .await
    });
    let mut waiting = false;
    for _ in 0..200 {
        let n:i64=sqlx::query_scalar("SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())").fetch_one(&f.db.migrator).await?;
        if n > 0 {
            waiting = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        waiting,
        "Native request must actually wait before authority is revoked"
    );
    sqlx::query("UPDATE core.account_relationships SET revoked_at=clock_timestamp() WHERE account_id=$1 AND kind='OWNER'").bind(account.0).execute(&mut *block).await?;
    block.commit().await?;
    assert!(task.await?.is_err());
    assert_eq!(review_count(&f).await?, 0);
    f.close().await
}
#[tokio::test]
async fn preview_review_archive_and_private_attribution_are_immutable_and_independently_verified()
-> Result<()> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = customer(&f, account).await?;
    let terms = reviews(&f).await?;
    let input = review_input(
        &f.core
            .periodic_disclosure(&f.f.controls, &actor, account, policy)
            .await?
            .digest,
    );
    let original = terms
        .acknowledge_periodic_preview(
            &f.core,
            &f.f.controls,
            &actor,
            (account, policy, Uuid::new_v4()),
            &input,
        )
        .await?;
    for statement in [
        "UPDATE documents.periodic_preview_reviews SET person_id=gen_random_uuid()",
        "DELETE FROM documents.periodic_preview_reviews",
        "TRUNCATE documents.periodic_preview_reviews",
        "DELETE FROM documents.periodic_review_requests",
    ] {
        assert!(sqlx::query(statement).execute(&terms.pool).await.is_err());
        assert!(
            sqlx::query(statement)
                .execute(&f.db.migrator)
                .await
                .is_err()
        );
    }
    assert_eq!(
        terms
            .periodic_preview_review(&f.core, &f.f.controls, &actor, account, policy)
            .await?
            .digest,
        original.digest
    );
    sqlx::query("ALTER TABLE documents.periodic_preview_reviews DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE documents.periodic_preview_reviews SET actor_context=jsonb_set(actor_context,'{subject}','\"forged attribution\"')").execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE documents.periodic_preview_reviews ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(
        terms
            .periodic_preview_review(&f.core, &f.f.controls, &actor, account, policy)
            .await
            .is_err()
    );
    f.close().await
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
