use bank_controls::entitlements::{Action, ApprovalRef, Entitlements, ResourceAuthority};
use bank_platform::{
    database::Module,
    domain::{
        AccountId, ActorContext, Currency, Money, PartyId, PaymentDetails, TrustContext,
        fingerprint,
    },
    testing::TestDatabase,
};
use chrono::{Duration, Utc};
use uuid::Uuid;

#[tokio::test]
async fn authority_maker_checker_and_action_proofs_hold() -> anyhow::Result<()> {
    let db = TestDatabase::new().await?;
    let controls = db.module(Module::Controls).await?;
    let identity_pool = db.module(Module::Identity).await?;
    let service = Entitlements::new(controls.clone());
    let maker_person = PartyId::new();
    let checker_person = PartyId::new();
    let customer_person = PartyId::new();
    let actor = |subject: &str, trust| ActorContext {
        issuer: "urn:fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "test".into(),
        person_id: None,
        represented_party: None,
        roles: vec!["security_admin".into()],
    };
    let maker = actor("maker", TrustContext::Workforce);
    let alias = actor("maker-alias", TrustContext::Workforce);
    let checker = actor("checker", TrustContext::Workforce);
    let customer = actor("customer", TrustContext::Customer);
    for (who, person) in [
        (&maker, maker_person),
        (&alias, maker_person),
        (&checker, checker_person),
        (&customer, customer_person),
    ] {
        sqlx::query("INSERT INTO controls.person_links(issuer,subject,person_id) VALUES($1,$2,$3)")
            .bind(&who.issuer)
            .bind(&who.subject)
            .bind(person.0)
            .execute(&db.migrator)
            .await?;
    }
    for (person, role) in [
        (maker_person, "security_admin"),
        (maker_person, "security_approver"),
        (checker_person, "security_approver"),
    ] {
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&db.migrator).await?;
    }
    let account = AccountId::new();
    let authority = ResourceAuthority {
        resource_id: account.0,
        owners: vec![customer_person],
        permitted_staff: vec![],
        action_restricted: false,
    };
    assert!(
        service
            .authorize(
                &customer,
                Action::ReadAccount,
                Some(&authority),
                Uuid::new_v4()
            )
            .await?
    );
    assert!(
        !service
            .authorize(
                &maker,
                Action::ReadAccount,
                Some(&authority),
                Uuid::new_v4()
            )
            .await?
    );
    assert!(
        !service
            .authorize(&customer, Action::ManageRoles, None, Uuid::new_v4())
            .await?
    );
    let proposal = service
        .propose_role(
            &maker,
            checker_person,
            "account_support",
            Some(account.0),
            Utc::now() + Duration::hours(1),
            Uuid::new_v4(),
        )
        .await?;
    assert!(
        service
            .approve_role(&alias, proposal, Uuid::new_v4())
            .await
            .is_err()
    );
    service
        .approve_role(&checker, proposal, Uuid::new_v4())
        .await?;
    let staff_authority = ResourceAuthority {
        resource_id: account.0,
        owners: vec![customer_person],
        permitted_staff: vec![checker_person],
        action_restricted: false,
    };
    assert!(
        service
            .authorize(
                &checker,
                Action::ReadAccount,
                Some(&staff_authority),
                Uuid::new_v4()
            )
            .await?
    );
    let grant: Uuid =
        sqlx::query_scalar("SELECT id FROM controls.role_grants WHERE proposal_id=$1")
            .bind(proposal)
            .fetch_one(&controls)
            .await?;
    assert!(
        service
            .revoke_role(&checker, grant, Uuid::new_v4())
            .await
            .is_err()
    );
    service.revoke_role(&maker, grant, Uuid::new_v4()).await?;
    assert!(
        !service
            .authorize(
                &checker,
                Action::ReadAccount,
                Some(&staff_authority),
                Uuid::new_v4()
            )
            .await?
    );
    let operation = Uuid::new_v4();
    let details = PaymentDetails {
        payer_account_id: account,
        beneficiary_id: "fixture-beneficiary".into(),
        amount: Money::new(2500, Currency::synthetic()),
        fee: Money::new(0, Currency::synthetic()),
        requested_date: Utc::now().date_naive(),
        rail: "SIMULATED".into(),
        quote_id: "fixture-quote".into(),
    };
    let challenge = service
        .challenge(&customer, operation, &details, &authority, Uuid::new_v4())
        .await?;
    assert_eq!(
        challenge.id,
        service
            .challenge(&customer, operation, &details, &authority, Uuid::new_v4())
            .await?
            .id
    );
    let identity = bank_identity::IdentityService {
        verifier: bank_identity::oidc::Verifier::new()?,
        pool: identity_pool.clone(),
        public_base: "http://127.0.0.1:4180".into(),
    };
    assert!(
        service
            .approve_payment(
                &identity,
                &customer,
                ApprovalRef {
                    challenge_id: challenge.id,
                    operation_id: operation
                },
                &details,
                &authority,
                Uuid::new_v4()
            )
            .await
            .is_err()
    );
    // This fixture represents the immutable receipt emitted only by the tested OIDC callback.
    sqlx::query("INSERT INTO identity.subjects(issuer,subject,trust) VALUES($1,$2,'customer')")
        .bind(&customer.issuer)
        .bind(&customer.subject)
        .execute(&db.migrator)
        .await?;
    sqlx::query("INSERT INTO identity.step_up_proofs(id,challenge_id,issuer,subject,intent_digest,authenticated_at,expires_at) VALUES($1,$2,$3,$4,$5,now(),now()+interval '1 minute')")
        .bind(Uuid::new_v4()).bind(challenge.id).bind(&customer.issuer).bind(&customer.subject).bind(fingerprint(&details)?).execute(&db.migrator).await?;
    let mut changed = details.clone();
    changed.amount = Money::new(2501, Currency::synthetic());
    assert!(
        service
            .approve_payment(
                &identity,
                &customer,
                ApprovalRef {
                    challenge_id: challenge.id,
                    operation_id: operation
                },
                &changed,
                &authority,
                Uuid::new_v4()
            )
            .await
            .is_err()
    );
    assert!(
        service
            .approve_payment(
                &identity,
                &customer,
                ApprovalRef {
                    challenge_id: challenge.id,
                    operation_id: Uuid::new_v4()
                },
                &details,
                &authority,
                Uuid::new_v4()
            )
            .await
            .is_err()
    );
    service
        .approve_payment(
            &identity,
            &customer,
            ApprovalRef {
                challenge_id: challenge.id,
                operation_id: operation,
            },
            &details,
            &authority,
            Uuid::new_v4(),
        )
        .await?;
    service
        .approve_payment(
            &identity,
            &customer,
            ApprovalRef {
                challenge_id: challenge.id,
                operation_id: operation,
            },
            &details,
            &authority,
            Uuid::new_v4(),
        )
        .await?;
    let restricted = ResourceAuthority {
        resource_id: account.0,
        owners: vec![customer_person],
        permitted_staff: vec![],
        action_restricted: true,
    };
    assert!(
        service
            .approve_payment(
                &identity,
                &customer,
                ApprovalRef {
                    challenge_id: challenge.id,
                    operation_id: operation
                },
                &details,
                &restricted,
                Uuid::new_v4()
            )
            .await
            .is_err()
    );
    controls.close().await;
    identity_pool.close().await;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn distinct_signed_controls_pending_conflicts_expiry_and_provider_restart()
-> anyhow::Result<()> {
    use bank_controls::checks::{CheckKind, CheckResult, Checks, Outcome, Purpose, simulator_key};
    use bank_platform::{authenticity::SignedDocument, domain::fingerprint};
    use serde_json::json;
    use std::process::{Child, Command, Stdio};
    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Controls).await?;
    let checks = Checks::new(pool.clone())?;
    // A separate actual provider process uses only its own runtime database principal.
    // The configured test URL is unique per integration-test process, so no API port is occupied.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/provider-simulator");
    let spawn = || -> anyhow::Result<Process> {
        Ok(Process(
            Command::new(&binary)
                .env("BANK_BIND", address.to_string())
                .env(
                    "BANK_SIMULATOR_DATABASE_URL",
                    db.module_url(Module::Simulator),
                )
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        ))
    };
    let mut server = spawn()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;
    let base = format!("http://{address}");
    for _ in 0..100 {
        if client
            .get(format!("{base}/health/live"))
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // Explicit endpoint injection avoids changing process-global environment during concurrent tests.
    let checks = checks.with_provider_base(base.clone())?;
    let scenario = |id, mode: &str| {
        client
            .post(format!("{base}/admin/scenarios"))
            .header("X-Simulator-Key", simulator_key())
            .json(&json!({"requestId":id,"mode":mode}))
    };
    let actor = ActorContext {
        issuer: "urn:control-fixture".into(),
        subject: "applicant".into(),
        trust: TrustContext::Customer,
        client_id: "customer-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let resource = Uuid::new_v4();
    let digest = fingerprint(&json!({"syntheticLegalData":"fixture-v1"}))?;
    let identity = checks
        .request(&actor, resource, CheckKind::Identity, &digest)
        .await?;
    assert_eq!(checks.poll(identity.id).await?, Outcome::Allow);
    assert_eq!(checks.poll(identity.id).await?, Outcome::Allow);
    assert_eq!(
        checks
            .evaluate(resource, Purpose::Onboarding, &digest)
            .await?
            .outcome,
        Outcome::Pending
    );
    let sanctions = checks
        .request(&actor, resource, CheckKind::Sanctions, &digest)
        .await?;
    scenario(sanctions.id, "REVIEW")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(checks.poll(sanctions.id).await?, Outcome::Review);
    let risk = checks
        .request(&actor, resource, CheckKind::Risk, &digest)
        .await?;
    assert_eq!(checks.poll(risk.id).await?, Outcome::Allow);
    assert_eq!(
        checks
            .evaluate(resource, Purpose::Onboarding, &digest)
            .await?
            .outcome,
        Outcome::Review
    );
    assert_eq!(
        checks
            .evaluate(resource, Purpose::Onboarding, &"a".repeat(64))
            .await?
            .outcome,
        Outcome::Pending
    );
    let packet: SignedDocument = client
        .get(format!("{base}/checks/{}", identity.id))
        .header("X-Simulator-Key", simulator_key())
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut forged = packet.clone();
    forged.body["outcome"] = json!("DENY");
    assert!(checks.record(&forged).await.is_err());
    let mut changed: CheckResult = serde_json::from_value(packet.body.clone())?;
    changed.request.resource_id = Uuid::new_v4();
    assert!(
        checks
            .record(&SignedDocument::sign(simulator_key().as_bytes(), &changed)?)
            .await
            .is_err()
    );
    changed = serde_json::from_value(packet.body.clone())?;
    changed.outcome = Outcome::Deny;
    assert!(
        checks
            .record(&SignedDocument::sign(simulator_key().as_bytes(), &changed)?)
            .await
            .is_err()
    );
    assert_eq!(
        checks
            .evaluate(resource, Purpose::Onboarding, &digest)
            .await?
            .outcome,
        Outcome::Review
    );
    let timeout = checks
        .request(&actor, Uuid::new_v4(), CheckKind::Identity, &digest)
        .await?;
    scenario(timeout.id, "TIMEOUT")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(checks.poll(timeout.id).await?, Outcome::Pending);
    scenario(timeout.id, "ALLOW")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(checks.poll(timeout.id).await?, Outcome::Allow);
    let lost = checks
        .request(&actor, Uuid::new_v4(), CheckKind::Risk, &digest)
        .await?;
    scenario(lost.id, "LOST_RESPONSE")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(checks.poll(lost.id).await?, Outcome::Pending);
    scenario(lost.id, "ALLOW")
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(checks.poll(lost.id).await?, Outcome::Allow);
    let expiring = checks
        .request(&actor, Uuid::new_v4(), CheckKind::Identity, &digest)
        .await?;
    let mut expires = CheckResult {
        request: expiring.clone(),
        outcome: Outcome::Allow,
        evidence_id: Uuid::new_v4(),
        expires_at: Utc::now() + Duration::seconds(2),
        reason_code: "SYNTHETIC_EXPIRY".into(),
    };
    checks
        .record(&SignedDocument::sign(simulator_key().as_bytes(), &expires)?)
        .await?;
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    let replacement = checks
        .request(&actor, expiring.resource_id, CheckKind::Identity, &digest)
        .await?;
    assert_ne!(replacement.id, expiring.id);
    expires.expires_at = Utc::now() + Duration::minutes(5);
    assert!(
        checks
            .record(&SignedDocument::sign(simulator_key().as_bytes(), &expires)?)
            .await
            .is_err()
    );
    let good = Uuid::new_v4();
    for kind in Purpose::Onboarding.required() {
        let request = checks.request(&actor, good, *kind, &digest).await?;
        assert_eq!(checks.poll(request.id).await?, Outcome::Allow);
    }
    assert_eq!(
        checks
            .evaluate(good, Purpose::Onboarding, &digest)
            .await?
            .outcome,
        Outcome::Allow
    );
    assert_eq!(
        checks
            .evaluate(good, Purpose::Payment, &digest)
            .await?
            .outcome,
        Outcome::Pending
    );
    let late = checks
        .request(&actor, Uuid::new_v4(), CheckKind::Sanctions, &digest)
        .await?;
    scenario(late.id, "LATE").send().await?.error_for_status()?;
    assert!(checks.poll(late.id).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM controls.check_results WHERE request_id=$1"
        )
        .bind(identity.id)
        .fetch_one(&pool)
        .await?,
        1
    );
    let original: serde_json::Value =
        sqlx::query_scalar("SELECT document FROM controls.check_results WHERE request_id=$1")
            .bind(lost.id)
            .fetch_one(&pool)
            .await?;
    server.0.kill()?;
    server.0.wait()?;
    server = spawn()?;
    for _ in 0..100 {
        if client
            .get(format!("{base}/health/live"))
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(checks.poll(lost.id).await?, Outcome::Allow);
    assert_eq!(
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT document FROM controls.check_results WHERE request_id=$1"
        )
        .bind(lost.id)
        .fetch_one(&pool)
        .await?,
        original
    );
    assert!(
        bank_controls::audit::AuditLog::new(pool.clone())
            .verify("controls")
            .await?
    );
    drop(server);
    pool.close().await;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn case_ownership_information_independent_disposition_and_escalation() -> anyhow::Result<()> {
    use bank_controls::{
        cases::{CaseKind, Cases, Disposition, NewCase},
        checks::{CheckKind, CheckResult, Checks, Outcome, Purpose, simulator_key},
    };
    use bank_platform::authenticity::SignedDocument;
    use serde_json::json;
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Controls).await?;
    let cases = Cases::new(pool.clone());
    let checks = Checks::new(pool.clone())?;
    let actor = |subject: &str, trust| ActorContext {
        issuer: "urn:cases-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "test".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let customer = actor("customer", TrustContext::Customer);
    let foreign = actor("foreign", TrustContext::Customer);
    let maker = actor("maker", TrustContext::Workforce);
    let alias = actor("maker-alias", TrustContext::Workforce);
    let checker = actor("checker", TrustContext::Workforce);
    let rogue = actor("unrelated", TrustContext::Workforce);
    let maker_id = Uuid::new_v4();
    let checker_id = Uuid::new_v4();
    let rogue_id = Uuid::new_v4();
    for (who, id) in [
        (&maker, maker_id),
        (&alias, maker_id),
        (&checker, checker_id),
        (&rogue, rogue_id),
    ] {
        sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
            .bind(&who.issuer)
            .bind(&who.subject)
            .bind(id)
            .execute(&db.migrator)
            .await?;
    }
    for person in [maker_id, checker_id, rogue_id] {
        for role in ["case_reviewer", "case_approver"] {
            sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
        }
    }
    for queue in ["controls-operations", "support", "payments-operations"] {
        for person in [maker_id, checker_id] {
            for capability in ["REVIEW", "APPROVE"] {
                sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES($1,$2,$3)").bind(queue).bind(person).bind(capability).execute(&db.migrator).await?;
            }
        }
    }
    let resource = Uuid::new_v4();
    let digest = fingerprint(&json!({"fixture":"case-controlled-review"}))?;
    for kind in Purpose::Onboarding.required() {
        let request = checks.request(&customer, resource, *kind, &digest).await?;
        let result = CheckResult {
            request,
            outcome: if *kind == CheckKind::Sanctions {
                Outcome::Review
            } else {
                Outcome::Allow
            },
            evidence_id: Uuid::new_v4(),
            expires_at: Utc::now() + Duration::minutes(5),
            reason_code: "SIMULATED_DECISION".into(),
        };
        checks
            .record(&SignedDocument::sign(simulator_key().as_bytes(), &result)?)
            .await?;
    }
    assert_eq!(
        checks
            .evaluate(resource, Purpose::Onboarding, &digest)
            .await?
            .outcome,
        Outcome::Review
    );
    cases.ensure_control_exceptions().await?;
    cases.ensure_control_exceptions().await?;
    let queue = cases.list(&maker, 10).await?;
    assert_eq!(queue.len(), 1);
    let id = queue[0].id;
    assert!(cases.detail(&rogue, id).await.is_err());
    assert!(cases.detail(&customer, id).await.is_err());
    assert_eq!(
        cases.customer_cases(&foreign).await?["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        cases.customer_cases(&customer).await?["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!serde_json::to_string(&cases.customer_cases(&customer).await?)?.contains("SANCTIONS"));
    cases.claim(&maker, id).await?;
    cases.claim(&maker, id).await?;
    assert!(cases.claim(&checker, id).await.is_err());
    let proposal = cases.propose(&maker, id, Disposition::ClearControl).await?;
    assert!(cases.approve(&alias, id, proposal).await.is_err());
    cases.approve(&checker, id, proposal).await?;
    cases.approve(&checker, id, proposal).await?;
    assert_eq!(
        checks
            .evaluate(resource, Purpose::Onboarding, &digest)
            .await?
            .outcome,
        Outcome::Allow
    );
    assert_eq!(cases.detail(&checker, id).await?.state, "CLOSED");
    let support_source = |origin| NewCase {
        kind: CaseKind::Support,
        origin_id: origin,
        resource_id: resource,
        evidence_refs: vec![],
        customer: Some((customer.issuer.clone(), customer.subject.clone())),
    };
    let origin = Uuid::new_v4();
    let support = cases.ensure(&customer, support_source(origin)).await?;
    assert_eq!(
        support,
        cases.ensure(&customer, support_source(origin)).await?
    );
    assert!(
        cases
            .ensure(&foreign, support_source(origin))
            .await
            .is_err()
    );
    cases.claim(&maker, support).await?;
    cases
        .request_information(&maker, support, "IDENTITY_EVIDENCE_REQUIRED")
        .await?;
    assert_eq!(
        cases.detail(&maker, support).await?.state,
        "INFORMATION_REQUIRED"
    );
    sqlx::query("UPDATE controls.cases SET deadline=now()-interval '1 second' WHERE id=$1")
        .bind(support)
        .execute(&db.migrator)
        .await?;
    cases.escalate_overdue().await?;
    cases.escalate_overdue().await?;
    assert_eq!(cases.detail(&checker, support).await?.severity, "CRITICAL");
    assert!(cases.detail(&maker, support).await?.escalated_at.is_some());
    let payment = cases
        .ensure(
            &customer,
            NewCase {
                kind: CaseKind::Payment,
                origin_id: Uuid::new_v4(),
                resource_id: resource,
                evidence_refs: vec![],
                customer: Some((customer.issuer.clone(), customer.subject.clone())),
            },
        )
        .await?;
    cases.claim(&maker, payment).await?;
    assert!(
        cases
            .propose(&maker, payment, Disposition::ResolveService)
            .await
            .is_err()
    );
    assert!(
        cases
            .propose(&maker, payment, Disposition::Decline)
            .await
            .is_err()
    );
    let escalation = cases
        .propose(&maker, payment, Disposition::Escalate)
        .await?;
    cases.approve(&checker, payment, escalation).await?;
    assert_eq!(
        cases.detail(&maker, payment).await?.next_action,
        "ESCALATED_REVIEW"
    );
    let denied = checks
        .request(&customer, Uuid::new_v4(), CheckKind::Sanctions, &digest)
        .await?;
    checks
        .record(&SignedDocument::sign(
            simulator_key().as_bytes(),
            &CheckResult {
                request: denied,
                outcome: Outcome::Deny,
                evidence_id: Uuid::new_v4(),
                expires_at: Utc::now() + Duration::minutes(5),
                reason_code: "SIMULATED_DENY".into(),
            },
        )?)
        .await?;
    cases.ensure_control_exceptions().await?;
    let hard_case = cases
        .list(&maker, 10)
        .await?
        .into_iter()
        .find(|c| c.kind == "CONTROL")
        .unwrap()
        .id;
    cases.claim(&maker, hard_case).await?;
    let unsupported = cases
        .propose(&maker, hard_case, Disposition::ClearControl)
        .await?;
    assert!(
        cases
            .approve(&checker, hard_case, unsupported)
            .await
            .is_err()
    );
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='case_approver'").bind(checker_id).execute(&db.migrator).await?;
    assert!(
        cases
            .approve(&checker, hard_case, unsupported)
            .await
            .is_err()
    );
    assert!(
        bank_controls::audit::AuditLog::new(pool.clone())
            .verify("cases")
            .await?
    );
    pool.close().await;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn configuration_drafts_deduplicate_preserve_details_and_recheck_authority()
-> anyhow::Result<()> {
    use bank_controls::governance::ChangeKind;
    use serde_json::json;
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Controls).await?;
    let controls = Entitlements::new(pool.clone());
    let actor = |subject: &str| ActorContext {
        issuer: "urn:administration-fixture".into(),
        subject: subject.into(),
        trust: TrustContext::Workforce,
        client_id: "staff-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let maker = actor("maker");
    let alias = actor("alias");
    let checker = actor("checker");
    let rogue = actor("rogue");
    let maker_id = Uuid::new_v4();
    let checker_id = Uuid::new_v4();
    for (a, person) in [
        (&maker, maker_id),
        (&alias, maker_id),
        (&checker, checker_id),
    ] {
        sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
            .bind(&a.issuer)
            .bind(&a.subject)
            .bind(person)
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
    let resource = Uuid::new_v4();
    let details = json!({"id":resource,"immutable":"exact version"});
    let (one, two) = tokio::join!(
        controls.stage_configuration(&maker, ChangeKind::Product, resource, &details),
        controls.stage_configuration(&maker, ChangeKind::Product, resource, &details)
    );
    let id = one?;
    assert_eq!(id, two?);
    assert!(
        controls
            .stage_configuration(
                &maker,
                ChangeKind::Product,
                resource,
                &json!({"id":resource,"immutable":"altered"})
            )
            .await
            .is_err()
    );
    assert!(controls.approve_change(&alias, id).await.is_err());
    assert!(controls.configurations(&rogue).await.is_err());
    controls.approve_change(&checker, id).await?;
    assert!(
        controls
            .approved_change(&maker, id, ChangeKind::Product, resource, &details)
            .await
            .is_ok()
    );
    assert!(
        sqlx::query("UPDATE controls.configuration_drafts SET details='{}' WHERE proposal_id=$1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("TRUNCATE controls.configuration_drafts")
            .execute(&db.migrator)
            .await
            .is_err()
    );
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='product_approver'").bind(checker_id).execute(&db.migrator).await?;
    assert!(
        controls
            .approved_change(&maker, id, ChangeKind::Product, resource, &details)
            .await
            .is_err()
    );
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1")
        .bind(maker_id)
        .execute(&db.migrator)
        .await?;
    assert!(controls.configurations(&maker).await.is_err());
    assert!(
        controls
            .stage_configuration(&maker, ChangeKind::Product, Uuid::new_v4(), &details)
            .await
            .is_err()
    );
    pool.close().await;
    db.close().await
}
