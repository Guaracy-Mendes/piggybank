use bank_adapters::object_store::FilesystemStore;
use bank_controls::{
    checks::{CheckRequest, CheckResult, Checks, Outcome, simulator_key},
    entitlements::Entitlements,
    evidence::EvidenceService,
    governance::ChangeKind,
};
use bank_core::{Core, products::ProductVersion};
use bank_documents::{Terms, TermsDraft};
use bank_journeys::{ApplicantProfile, Journeys, Progress};
use bank_platform::{
    database::Module,
    domain::{ActorContext, Currency, Money, TrustContext},
    operations::{Completion, Operations},
    testing::TestDatabase,
};
use chrono::{Duration, NaiveDate, Utc};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;
#[tokio::test]
async fn onboarding_recovers_submissions_controls_terms_and_lost_core_response()
-> anyhow::Result<()> {
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Journeys).await?;
    let cp = db.module(Module::Controls).await?;
    let core_pool = db.module(Module::Core).await?;
    let dp = db.module(Module::Documents).await?;
    let op = db.module(Module::Platform).await?;
    let path = std::env::temp_dir().join(format!("piggybank_onboard_{}", Uuid::new_v4().simple()));
    let core = Core::new(core_pool.clone());
    let controls = Entitlements::new(cp.clone());
    let terms = Terms::new(dp.clone());
    let checks = Checks::new(cp.clone())?;
    let journeys = Arc::new(Journeys {
        pool: pool.clone(),
        core: core.clone(),
        controls: controls.clone(),
        terms: terms.clone(),
        checks: checks.clone(),
        operations: Operations::new(op.clone()),
        evidence: Arc::new(EvidenceService::new(
            cp.clone(),
            Arc::new(FilesystemStore::new(&path)?),
        )),
    });
    let actor = |subject: &str, trust| ActorContext {
        issuer: "urn:onboarding-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "fixture-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let customer = actor("customer", TrustContext::Customer);
    let foreign = actor("foreign", TrustContext::Customer);
    let maker = actor("maker", TrustContext::Workforce);
    let checker = actor("checker", TrustContext::Workforce);
    for (who, role) in [(&maker, "product_admin"), (&checker, "product_approver")] {
        let person = Uuid::new_v4();
        sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
            .bind(&who.issuer)
            .bind(&who.subject)
            .bind(person)
            .execute(&db.migrator)
            .await?;
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
    }
    let today = Utc::now().date_naive();
    let money = |n| Money::new(n, Currency::synthetic());
    let product = ProductVersion {
        id: Uuid::new_v4(),
        code: "ONBOARD_FIXTURE".into(),
        version: 1,
        name: "Synthetic current account".into(),
        label: "SYNTHETIC".into(),
        effective_from: today - Duration::days(1),
        effective_until: today + Duration::days(30),
        minimum_age: 18,
        payment_limit: money(10000),
        daily_limit: money(20000),
        flat_fee: money(0),
        fee_basis_points: 0,
        chart_version: "synthetic-chart-v1".into(),
    };
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
    let draft = TermsDraft {
        id: Uuid::new_v4(),
        product_version_id: product.id,
        version: 1,
        title: "Synthetic demonstration terms".into(),
        text: "Synthetic terms; no actual money.".into(),
        label: "SYNTHETIC".into(),
        effective_from: product.effective_from,
        effective_until: product.effective_until,
    };
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
    let mut tasks = vec![];
    for _ in 0..10 {
        let journeys = journeys.clone();
        let customer = customer.clone();
        tasks.push(tokio::spawn(async move {
            journeys
                .create(
                    &customer,
                    "same-application",
                    product.id,
                    true,
                    Some("fixture_campaign".into()),
                )
                .await
        }));
    }
    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        ids.insert(task.await??.id);
    }
    assert_eq!(ids.len(), 1);
    let id = *ids.iter().next().unwrap();
    assert!(journeys.get(&foreign, id).await.is_err());
    assert!(
        journeys
            .submit(&customer, id, "first-submit")
            .await
            .is_err()
    );
    let profile = ApplicantProfile {
        full_name: "Synthetic Applicant".into(),
        email: "fixture@example.invalid".into(),
        date_of_birth: NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(),
        residency: "SYNTHETIC".into(),
        consent_version: "synthetic-consent-v1".into(),
        marketing: true,
    };
    let app = journeys.get(&customer, id).await?;
    let app = journeys.save(&customer, id, app.version, profile).await?;
    assert!(
        journeys
            .save(&customer, id, app.version - 1, app.profile.clone().unwrap())
            .await
            .is_err()
    );
    journeys
        .attach_evidence(&customer, id, b"Synthetic identity evidence".to_vec())
        .await?;
    journeys.accept_terms(&customer, id, draft.id).await?;
    let operation = journeys.submit(&customer, id, "first-submit").await?;
    assert_eq!(
        operation.id,
        journeys.submit(&customer, id, "first-submit").await?.id
    );
    journeys.withdraw_marketing(&customer, id).await?;
    assert_eq!(
        operation.id,
        journeys.submit(&customer, id, "first-submit").await?.id
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM platform.jobs WHERE operation_id=$1")
            .bind(operation.id)
            .fetch_one(&op)
            .await?,
        1
    );
    assert!(matches!(journeys.process(id, 1).await?, Progress::Pending));
    assert!(
        core.accounts_for(bank_platform::domain::PartyId::new())
            .await?
            .is_empty()
    );
    let requests = sqlx::query("SELECT * FROM controls.check_requests WHERE resource_id=$1")
        .bind(id)
        .fetch_all(&cp)
        .await?;
    assert_eq!(requests.len(), 3);
    for row in requests {
        let request = CheckRequest {
            id: row.get("id"),
            resource_id: id,
            kind: serde_json::from_value(serde_json::json!(row.get::<String, _>("kind")))?,
            input_digest: row.get("input_digest"),
            policy_version: row.get("policy_version"),
        };
        checks
            .record(&bank_platform::authenticity::SignedDocument::sign(
                simulator_key().as_bytes(),
                &CheckResult {
                    request,
                    outcome: Outcome::Allow,
                    evidence_id: Uuid::new_v4(),
                    expires_at: Utc::now() + Duration::minutes(5),
                    reason_code: "SIMULATED_ALLOW".into(),
                },
            )?)
            .await?;
    }
    // Drop the Journey acknowledgement after Core activation. The next attempt must recover one account.
    sqlx::query("CREATE FUNCTION journeys.inject_lost_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.state='ACTIVE' THEN RAISE EXCEPTION 'Synthetic lost Core acknowledgement'; END IF;RETURN NEW;END $$").execute(&db.migrator).await?;
    sqlx::query("CREATE TRIGGER inject_lost_ack BEFORE UPDATE ON journeys.applications FOR EACH ROW EXECUTE FUNCTION journeys.inject_lost_ack()").execute(&db.migrator).await?;
    assert!(journeys.process(id, 1).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.account_registry")
            .fetch_one(&core_pool)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM core.ledger_accounts WHERE kind='CUSTOMER'"
        )
        .fetch_one(&core_pool)
        .await?,
        "ACTIVE"
    );
    sqlx::query("DROP TRIGGER inject_lost_ack ON journeys.applications")
        .execute(&db.migrator)
        .await?;
    let account = match journeys.process(id, 1).await? {
        Progress::Done {
            account_id: Some(id),
            ..
        } => id,
        _ => panic!("Expected one active account"),
    };
    assert!(
        matches!(journeys.process(id,1).await?,Progress::Done{account_id:Some(other),..} if other==account)
    );
    assert_eq!(core.balance(account).await?.booked.minor_units(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.parties")
            .fetch_one(&core_pool)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.account_registry")
            .fetch_one(&core_pool)
            .await?,
        1
    );
    // Simulate a lost operation completion; the same job records review and then resumes one original operation.
    let lease = journeys
        .operations
        .claim(Uuid::new_v4(), 30)
        .await?
        .unwrap();
    journeys
        .operations
        .complete(&lease, Completion::Review("SYNTHETIC_ACK_RECOVERY".into()))
        .await?;
    assert_eq!(
        journeys
            .operations
            .status(&customer, operation.id)
            .await?
            .state,
        "REVIEW_REQUIRED"
    );
    // A recovered job completes the actual, already activated domain result.
    journeys
        .operations
        .resume_review(
            &customer,
            operation.id,
            bank_platform::operations::JobSpec {
                kind: "journey.onboard".into(),
                step_key: "confirmed-core-result".into(),
                payload: serde_json::json!({"applicationId":id,"generation":1}),
                delay_seconds: 0,
            },
        )
        .await?;
    let retry = journeys
        .operations
        .claim(Uuid::new_v4(), 30)
        .await?
        .unwrap();
    journeys
        .operations
        .complete(
            &retry,
            Completion::Done(serde_json::to_value(journeys.process(id, 1).await?)?),
        )
        .await?;
    assert_eq!(journeys.get(&customer, id).await?.account_id, Some(account));
    assert_eq!(
        journeys
            .operations
            .status(&customer, operation.id)
            .await?
            .state,
        "SUCCEEDED"
    );
    // Independent manual clearance resumes the original operation, without a new application.
    let template = journeys.get(&customer, id).await?.profile.unwrap();
    for (who, role, capability) in [
        (&maker, "case_reviewer", "REVIEW"),
        (&checker, "case_approver", "APPROVE"),
    ] {
        let person = controls.person(who).await?;
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('controls-operations',$1,$2)").bind(person.0).bind(capability).execute(&db.migrator).await?;
    }
    let (review_app, review_operation) = ready(
        &journeys,
        &customer,
        product.id,
        draft.id,
        "manual-review",
        template.clone(),
    )
    .await?;
    assert!(matches!(
        journeys.process(review_app, 1).await?,
        Progress::Pending
    ));
    finish_checks(&checks, &cp, review_app, true).await?;
    assert!(matches!(
        journeys.process(review_app, 1).await?,
        Progress::Review {
            code: "CONTROL_REVIEW"
        }
    ));
    let lease = journeys
        .operations
        .claim(Uuid::new_v4(), 30)
        .await?
        .unwrap();
    assert_eq!(lease.operation_id, review_operation);
    journeys
        .operations
        .complete(&lease, Completion::Review("CONTROL_REVIEW".into()))
        .await?;
    let cases = bank_controls::cases::Cases::new(cp.clone());
    cases.ensure_control_exceptions().await?;
    let case = cases
        .list(&maker, 50)
        .await?
        .into_iter()
        .find(|c| c.resource_id == review_app)
        .unwrap()
        .id;
    cases.claim(&maker, case).await?;
    let proposal = cases
        .propose(
            &maker,
            case,
            bank_controls::cases::Disposition::ClearControl,
        )
        .await?;
    cases.approve(&checker, case, proposal).await?;
    journeys.resume_ready().await?;
    let lease = journeys
        .operations
        .claim(Uuid::new_v4(), 30)
        .await?
        .unwrap();
    assert_eq!(lease.operation_id, review_operation);
    let done = journeys.process(review_app, 1).await?;
    assert!(matches!(
        done,
        Progress::Done {
            account_id: Some(_),
            ..
        }
    ));
    journeys
        .operations
        .complete(&lease, Completion::Done(serde_json::to_value(done)?))
        .await?;
    assert_eq!(
        journeys.get(&customer, review_app).await?.operation_id,
        Some(review_operation)
    );
    // A negative independent disposition also resumes the original workflow to its final business decision.
    let (denial_app, denial_operation) = ready(
        &journeys,
        &customer,
        product.id,
        draft.id,
        "manual-decline",
        template.clone(),
    )
    .await?;
    assert!(matches!(
        journeys.process(denial_app, 1).await?,
        Progress::Pending
    ));
    finish_checks(&checks, &cp, denial_app, true).await?;
    assert!(matches!(
        journeys.process(denial_app, 1).await?,
        Progress::Review { .. }
    ));
    let lease = journeys
        .operations
        .claim(Uuid::new_v4(), 30)
        .await?
        .unwrap();
    assert_eq!(lease.operation_id, denial_operation);
    journeys
        .operations
        .complete(&lease, Completion::Review("CONTROL_REVIEW".into()))
        .await?;
    cases.ensure_control_exceptions().await?;
    let case = cases
        .list(&maker, 50)
        .await?
        .into_iter()
        .find(|c| c.resource_id == denial_app)
        .unwrap()
        .id;
    cases.claim(&maker, case).await?;
    let proposal = cases
        .propose(&maker, case, bank_controls::cases::Disposition::Decline)
        .await?;
    cases.approve(&checker, case, proposal).await?;
    journeys.resume_ready().await?;
    let lease = journeys
        .operations
        .claim(Uuid::new_v4(), 30)
        .await?
        .unwrap();
    assert_eq!(lease.operation_id, denial_operation);
    let decision = journeys.process(denial_app, 1).await?;
    assert!(matches!(decision,Progress::Done{ref state,account_id:None} if state=="DECLINED"));
    journeys
        .operations
        .complete(&lease, Completion::Done(serde_json::to_value(decision)?))
        .await?;
    assert_eq!(journeys.get(&customer, denial_app).await?.state, "DECLINED");
    // Changed terms and profile details invalidate an in-flight generation.
    let (changed_app, _) = ready(
        &journeys,
        &customer,
        product.id,
        draft.id,
        "terms-changed",
        template.clone(),
    )
    .await?;
    let mut next = draft.clone();
    next.id = Uuid::new_v4();
    next.version = 2;
    next.text = "Synthetic terms version two.".into();
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Terms,
            next.id,
            &next,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    controls.approve_change(&checker, proposal).await?;
    terms
        .publish(&core, &controls, &maker, proposal, &next)
        .await?;
    assert!(matches!(
        journeys.process(changed_app, 1).await?,
        Progress::Review {
            code: "CURRENT_TERMS_REQUIRED"
        }
    ));
    let app = journeys.get(&customer, changed_app).await?;
    assert_eq!(app.state, "INFORMATION_REQUIRED");
    let mut revised = template.clone();
    revised.email = "verified-change@example.invalid".into();
    journeys
        .save(&customer, changed_app, app.version, revised)
        .await?;
    assert!(matches!(
        journeys.process(changed_app, 1).await?,
        Progress::Review {
            code: "APPLICATION_DETAILS_CHANGED"
        }
    ));
    journeys
        .accept_terms(&customer, changed_app, next.id)
        .await?;
    assert!(
        journeys
            .submit(&customer, changed_app, "terms-changed-submit")
            .await
            .is_err()
    );
    journeys
        .submit(&customer, changed_app, "renewed-submission")
        .await?;
    assert!(matches!(
        journeys.process(changed_app, 2).await?,
        Progress::Pending
    ));
    finish_checks(&checks, &cp, changed_app, false).await?;
    assert!(matches!(
        journeys.process(changed_app, 2).await?,
        Progress::Done {
            account_id: Some(_),
            ..
        }
    ));
    // Ineligible applicants receive a terminal business decision without an account.
    let mut minor = template.clone();
    minor.date_of_birth = today - Duration::days(3650);
    let (ineligible, _) =
        ready(&journeys, &foreign, product.id, next.id, "underage", minor).await?;
    assert!(
        matches!(journeys.process(ineligible,1).await?,Progress::Done{state,..} if state=="DECLINED")
    );
    assert!(
        journeys
            .get(&foreign, ineligible)
            .await?
            .account_id
            .is_none()
    );
    // Recovery after a restricted Core record exists and terms subsequently advance.
    let (staged, _) = ready(
        &journeys,
        &customer,
        product.id,
        next.id,
        "staged-terms",
        template.clone(),
    )
    .await?;
    assert!(matches!(
        journeys.process(staged, 1).await?,
        Progress::Pending
    ));
    finish_checks(&checks, &cp, staged, false).await?;
    let app = journeys.get(&customer, staged).await?;
    let party = controls.person(&customer).await?;
    let original_account = core
        .stage_account(&bank_core::accounts::OpeningRequest {
            application_id: staged,
            party_id: party,
            product_version_id: product.id,
            acceptance_id: app.acceptance_id.unwrap(),
        })
        .await?;
    let mut newest = next.clone();
    newest.id = Uuid::new_v4();
    newest.version = 3;
    newest.text = "Synthetic terms version three.".into();
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Terms,
            newest.id,
            &newest,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    controls.approve_change(&checker, proposal).await?;
    terms
        .publish(&core, &controls, &maker, proposal, &newest)
        .await?;
    assert!(matches!(
        journeys.process(staged, 1).await?,
        Progress::Review {
            code: "CURRENT_TERMS_REQUIRED"
        }
    ));
    journeys.accept_terms(&customer, staged, newest.id).await?;
    journeys.submit(&customer, staged, "staged-renewal").await?;
    assert!(
        matches!(journeys.process(staged,2).await?,Progress::Done{account_id:Some(account),..} if account==original_account)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core.opening_revisions WHERE account_id=$1"
        )
        .bind(original_account.0)
        .fetch_one(&core_pool)
        .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.parties")
            .fetch_one(&core_pool)
            .await?,
        1
    );
    // A contact change is a separate verified, versioned workflow; it cannot rewrite legal identity.
    let before = core.profile(&customer, &controls).await?;
    assert!(before.email.is_none());
    let change = journeys
        .propose_contact(
            &customer,
            "verified-contact",
            "verified@example.invalid".into(),
        )
        .await?;
    assert!(matches!(
        journeys.process_contact(change.id).await?,
        Progress::Pending
    ));
    assert!(core.profile(&customer, &controls).await?.email.is_none());
    finish_checks(&checks, &cp, change.id, false).await?;
    assert!(
        matches!(journeys.process_contact(change.id).await?,Progress::Done{state,..} if state=="CONTACT_VERIFIED")
    );
    assert_eq!(
        journeys
            .propose_contact(
                &customer,
                "verified-contact",
                "verified@example.invalid".into()
            )
            .await?
            .id,
        change.id
    );
    assert!(
        journeys
            .propose_contact(
                &customer,
                "verified-contact",
                "changed@example.invalid".into()
            )
            .await
            .is_err()
    );
    assert!(journeys.contact_status(&foreign, change.id).await.is_err());
    let current = core.profile(&customer, &controls).await?;
    assert_eq!(current.email.as_deref(), Some("verified@example.invalid"));
    assert_eq!(current.version, before.version + 1);
    let winner = journeys
        .propose_contact(&customer, "contact-winner", "winner@example.invalid".into())
        .await?;
    let stale = journeys
        .propose_contact(&customer, "contact-stale", "stale@example.invalid".into())
        .await?;
    journeys.process_contact(winner.id).await?;
    journeys.process_contact(stale.id).await?;
    finish_checks(&checks, &cp, winner.id, false).await?;
    finish_checks(&checks, &cp, stale.id, false).await?;
    journeys.process_contact(winner.id).await?;
    assert!(matches!(
        journeys.process_contact(stale.id).await?,
        Progress::Review {
            code: "PROFILE_VERSION_CHANGED"
        }
    ));
    assert_eq!(
        core.profile(&customer, &controls).await?.email.as_deref(),
        Some("winner@example.invalid")
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT identity_digest FROM core.parties WHERE id=$1")
            .bind(controls.person(&customer).await?.0)
            .fetch_one(&core_pool)
            .await?,
        template.identity_digest()?
    );
    pool.close().await;
    cp.close().await;
    core_pool.close().await;
    dp.close().await;
    op.close().await;
    db.close().await?;
    std::fs::remove_dir_all(path)?;
    Ok(())
}

async fn ready(
    journeys: &Journeys<FilesystemStore>,
    actor: &ActorContext,
    product: Uuid,
    terms: Uuid,
    key: &str,
    profile: ApplicantProfile,
) -> anyhow::Result<(Uuid, Uuid)> {
    let app = journeys.create(actor, key, product, false, None).await?;
    journeys.save(actor, app.id, app.version, profile).await?;
    journeys
        .attach_evidence(actor, app.id, b"Synthetic identity evidence".to_vec())
        .await?;
    journeys.accept_terms(actor, app.id, terms).await?;
    let operation = journeys
        .submit(actor, app.id, &format!("{key}-submit"))
        .await?;
    Ok((app.id, operation.id))
}
async fn finish_checks(
    checks: &Checks,
    pool: &sqlx::PgPool,
    app: Uuid,
    review: bool,
) -> anyhow::Result<()> {
    let rows=sqlx::query("SELECT r.* FROM controls.check_requests r JOIN controls.check_heads h ON h.request_id=r.id WHERE r.resource_id=$1").bind(app).fetch_all(pool).await?;
    for row in rows {
        let kind = serde_json::from_value(serde_json::json!(row.get::<String, _>("kind")))?;
        let request = CheckRequest {
            id: row.get("id"),
            resource_id: app,
            kind,
            input_digest: row.get("input_digest"),
            policy_version: row.get("policy_version"),
        };
        let result = CheckResult {
            outcome: if review && kind == bank_controls::checks::CheckKind::Sanctions {
                Outcome::Review
            } else {
                Outcome::Allow
            },
            request,
            evidence_id: Uuid::new_v4(),
            expires_at: Utc::now() + Duration::minutes(5),
            reason_code: "SIMULATED_DECISION".into(),
        };
        checks
            .record(&bank_platform::authenticity::SignedDocument::sign(
                simulator_key().as_bytes(),
                &result,
            )?)
            .await?;
    }
    Ok(())
}
