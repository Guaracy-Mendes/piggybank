use super::*;
use bank_documents::{Terms, offers::*};
pub(super) async fn setup() -> Result<(Fixture, Terms, ActorContext, AccountId, Uuid)> {
    let f = Fixture::new().await?;
    let (policy, account) = super::basis::setup(&f).await?;
    let actor = super::disclosures::customer(&f, account).await?;
    let terms = Terms::new(f.db.module(Module::Documents).await?);
    Ok((f, terms, actor, account, policy))
}
pub(super) async fn offer(f: &Fixture, t: &Terms, policy: Uuid) -> Result<PublishedOffer> {
    let r = t
        .stage_periodic_offer(
            &f.f.controls,
            &f.maker,
            Uuid::new_v4(),
            &OfferInput {
                policy_id: policy,
                reason: "Publish separately approved synthetic enrollment terms".into(),
            },
        )
        .await?;
    t.approve_periodic_offer(&f.f.controls, &f.checker, r.id, &r.digest)
        .await?;
    Ok(
        t.publish_periodic_offer(&f.f.controls, &f.maker, r.id, &r.digest)
            .await?
            .publication
            .unwrap(),
    )
}
pub(super) fn accept(o: &PublishedOffer) -> EnrollmentInput {
    EnrollmentInput {
        offer_digest: o.digest.clone(),
        accepted: true,
    }
}
async fn money_snapshot(f: &Fixture) -> Result<serde_json::Value> {
    Ok(sqlx::query_scalar("SELECT jsonb_build_object('journals',(SELECT count(*) FROM core.journal_batches),'positions',(SELECT jsonb_agg(jsonb_build_array(id,booked,held,version) ORDER BY id) FROM core.ledger_accounts))").fetch_one(&f.db.migrator).await?)
}
#[tokio::test]
async fn offer_has_independent_exact_approval_and_stable_publication_without_reinterpreting_preview()
-> Result<()> {
    let (f, t, a, account, policy) = setup().await?;
    let before = money_snapshot(&f).await?;
    let key = Uuid::new_v4();
    let input = OfferInput {
        policy_id: policy,
        reason: "Separately approve synthetic financial enrollment material".into(),
    };
    let x = t
        .stage_periodic_offer(&f.f.controls, &f.maker, key, &input)
        .await?;
    let y = t
        .stage_periodic_offer(&f.f.controls, &f.maker, key, &input)
        .await?;
    assert_eq!(x.id, y.id);
    assert_eq!(x.state, "STAGED");
    assert!(
        t.customer_periodic_offer(&f.core, &f.f.controls, &a, account, x.id)
            .await
            .is_err()
    );
    assert!(
        t.publish_periodic_offer(&f.f.controls, &f.maker, x.id, &x.digest)
            .await
            .is_err()
    );
    assert!(
        t.approve_periodic_offer(&f.f.controls, &f.maker, x.id, &x.digest)
            .await
            .is_err()
    );
    assert!(
        t.approve_periodic_offer(&f.f.controls, &f.checker, x.id, &"0".repeat(64))
            .await
            .is_err()
    );
    t.approve_periodic_offer(&f.f.controls, &f.checker, x.id, &x.digest)
        .await?;
    let x = t
        .publish_periodic_offer(&f.f.controls, &f.maker, x.id, &x.digest)
        .await?;
    assert_eq!(x.state, "PUBLISHED");
    let p = t
        .customer_periodic_offer(&f.core, &f.f.controls, &a, account, x.id)
        .await?;
    p.validate()?;
    assert_eq!(p.digest, x.digest);
    assert!(p.material.text.contains("365 basis points"));
    assert!(p.material.text.contains("next UTC calendar day"));
    assert!(p.material.text.contains("PENDING_ACTIVATION"));
    let public = serde_json::to_string(&p)?;
    for private in [
        "makerPerson",
        "checkerPerson",
        "Separately approve",
        "actorContext",
    ] {
        assert!(!public.contains(private));
    }
    let replay = t
        .publish_periodic_offer(&f.f.controls, &f.maker, x.id, &x.digest)
        .await?
        .publication
        .unwrap();
    assert_eq!(replay.publication_digest, p.publication_digest);
    assert_eq!(replay.published_at, p.published_at);
    assert_eq!(
        f.core
            .periodic_policies()
            .get(policy)
            .await?
            .unwrap()
            .policy
            .activation,
        "PREVIEW_ONLY"
    );
    assert_eq!(money_snapshot(&f).await?, before);
    f.close().await
}
#[tokio::test]
async fn customer_accepts_exact_terms_once_with_future_day_and_withdraws_without_activation_or_money()
-> Result<()> {
    let (f, t, a, account, policy) = setup().await?;
    let o = offer(&f, &t, policy).await?;
    let before = money_snapshot(&f).await?;
    let key = Uuid::new_v4();
    let input = accept(&o);
    let (c, d) = tokio::join!(
        t.request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, key),
            &input
        ),
        t.request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &input
        )
    );
    let (c, d) = (c?, d?);
    c.validate()?;
    assert_eq!(c.consent_digest, d.consent_digest);
    assert_eq!(c.status, "PENDING_ACTIVATION");
    assert_eq!(c.financial_activation, "DISABLED");
    let at: chrono::DateTime<Utc> = c.consent.accepted_at.parse()?;
    assert_eq!(
        c.consent.earliest_period_start,
        at.date_naive() + Duration::days(1)
    );
    let withdraw = WithdrawalInput {
        consent_digest: c.consent_digest.clone(),
        withdraw: true,
    };
    let wk = Uuid::new_v4();
    let w = t
        .withdraw_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, wk),
            &withdraw,
        )
        .await?;
    w.validate()?;
    assert_eq!(w.status, "WITHDRAWN");
    assert_eq!(w.consent_digest, c.consent_digest);
    let replay = t
        .request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, key),
            &input,
        )
        .await?;
    assert_eq!(replay.status, "WITHDRAWN");
    assert_eq!(replay.withdrawal_digest, w.withdrawal_digest);
    assert!(
        t.request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &input
        )
        .await
        .is_err()
    );
    let wr = t
        .withdraw_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &withdraw,
        )
        .await?;
    assert_eq!(wr.withdrawal_digest, w.withdrawal_digest);
    let events:Vec<(i64,String)>=sqlx::query_as("SELECT aggregate_version,document->>'eventType' FROM documents.outbox WHERE aggregate_id=$1 ORDER BY aggregate_version").bind(c.consent.id).fetch_all(&t.pool).await?;
    assert_eq!(
        events,
        vec![
            (1, "periodic.enrollment.requested".into()),
            (2, "periodic.enrollment.withdrawn".into())
        ]
    );
    assert_eq!(money_snapshot(&f).await?, before);
    f.close().await
}
#[tokio::test]
async fn financial_consent_rejects_false_digest_foreign_staff_joint_and_restricted_accounts()
-> Result<()> {
    let (f, t, a, account, policy) = setup().await?;
    let o = offer(&f, &t, policy).await?;
    let mut no = accept(&o);
    no.accepted = false;
    assert!(
        t.request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &no
        )
        .await
        .is_err()
    );
    no.accepted = true;
    no.offer_digest = "0".repeat(64);
    assert!(
        t.request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &no
        )
        .await
        .is_err()
    );
    let mut foreign = a.clone();
    foreign.subject = "foreign-enrollment".into();
    for actor in [&foreign, &f.maker] {
        assert!(
            t.customer_periodic_offer(&f.core, &f.f.controls, actor, account, o.material.id)
                .await
                .is_err()
        );
        assert!(
            t.request_periodic_enrollment(
                &f.core,
                &f.f.controls,
                actor,
                (account, o.material.id, Uuid::new_v4()),
                &accept(&o)
            )
            .await
            .is_err()
        );
    }
    sqlx::query("UPDATE core.ledger_accounts SET credit_blocked=true WHERE id=$1")
        .bind(account.0)
        .execute(&f.db.migrator)
        .await?;
    assert!(
        t.request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &accept(&o)
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE core.ledger_accounts SET credit_blocked=false WHERE id=$1")
        .bind(account.0)
        .execute(&f.db.migrator)
        .await?;
    let mut joint = a.clone();
    joint.subject = "joint-enrollment-owner".into();
    let second = f
        .core
        .ensure_party(&joint, Uuid::new_v4(), &fingerprint(&joint.subject)?)
        .await?
        .0;
    sqlx::query(
        "INSERT INTO core.account_relationships(account_id,person_id,kind) VALUES($1,$2,'OWNER')",
    )
    .bind(account.0)
    .bind(second)
    .execute(&f.db.migrator)
    .await?;
    assert!(
        t.request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &accept(&o)
        )
        .await
        .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM documents.periodic_enrollment_consents")
            .fetch_one(&t.pool)
            .await?,
        0
    );
    f.close().await
}
#[tokio::test]
async fn published_history_and_consent_survive_live_source_damage_but_new_acceptance_refuses_it()
-> Result<()> {
    let (f, t, a, account, policy) = setup().await?;
    let o = offer(&f, &t, policy).await?;
    let c = t
        .request_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &accept(&o),
        )
        .await?;
    sqlx::query("ALTER TABLE core.periodic_policies DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE core.periodic_policies SET document=jsonb_set(document,'{calculation,annualBasisPoints}','9999') WHERE id=$1").bind(policy).execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE core.periodic_policies ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert_eq!(
        t.customer_periodic_offer(&f.core, &f.f.controls, &a, account, o.material.id)
            .await?
            .digest,
        o.digest
    );
    assert_eq!(
        t.periodic_enrollment(&f.core, &f.f.controls, &a, account, o.material.id)
            .await?
            .consent_digest,
        c.consent_digest
    );
    let w = t
        .withdraw_periodic_enrollment(
            &f.core,
            &f.f.controls,
            &a,
            (account, o.material.id, Uuid::new_v4()),
            &WithdrawalInput {
                consent_digest: c.consent_digest.clone(),
                withdraw: true,
            },
        )
        .await?;
    assert_eq!(w.status, "WITHDRAWN");
    sqlx::query("ALTER TABLE documents.periodic_enrollment_consents DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE documents.periodic_enrollment_consents SET actor_context=jsonb_set(actor_context,'{subject}','\"forged\"')").execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE documents.periodic_enrollment_consents ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(
        t.periodic_enrollment(&f.core, &f.f.controls, &a, account, o.material.id)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn offer_publication_rechecks_approval_expiry_after_native_lock_wait() -> Result<()> {
    let (f, t, _a, _account, policy) = setup().await?;
    let x = t
        .stage_periodic_offer(
            &f.f.controls,
            &f.maker,
            Uuid::new_v4(),
            &OfferInput {
                policy_id: policy,
                reason: "Approval expiry while waiting for offer lock".into(),
            },
        )
        .await?;
    t.approve_periodic_offer(&f.f.controls, &f.checker, x.id, &x.digest)
        .await?;
    let mut block = f.db.migrator.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::uuid::text,202))")
        .bind(policy)
        .execute(&mut *block)
        .await?;
    let tt = t.clone();
    let controls = f.f.controls.clone();
    let actor = f.maker.clone();
    let digest = x.digest.clone();
    let task = tokio::spawn(async move {
        tt.publish_periodic_offer(&controls, &actor, x.id, &digest)
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
    assert!(waiting);
    sqlx::query("UPDATE controls.change_proposals SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(x.id).execute(&mut *block).await?;
    block.commit().await?;
    assert!(task.await?.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM documents.periodic_offers")
            .fetch_one(&t.pool)
            .await?,
        0
    );
    f.close().await
}
