//! WP-28: FR-IAM-06 recovery, FR-CTRL-06 access recertification, SEC-07/10 findings and AT-16 privacy requests.
use anyhow::Result;
use bank_controls::{entitlements::Entitlements, security::Security};
use bank_documents::{retention::DataRequestInput, statements::Records};
use bank_platform::{
    database::Module,
    domain::{ActorContext, TrustContext},
    testing::TestDatabase,
};
use serde_json::{Value, json};
use uuid::Uuid;

struct S {
    db: TestDatabase,
    c: Entitlements,
    sec: Security,
    r: Records,
    alice: ActorContext,
    bob: ActorContext,
    support: ActorContext,
    approver: ActorContext,
    approver2: ActorContext,
    admin: ActorContext,
    officer: ActorContext,
}
fn actor(subject: &str, trust: TrustContext) -> ActorContext {
    ActorContext {
        issuer: "urn:security-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "fixture".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
fn id(v: &Value) -> Uuid {
    serde_json::from_value(v.clone()).unwrap()
}
impl S {
    async fn new() -> Result<Self> {
        let db = TestDatabase::new().await?;
        let c = Entitlements::new(db.module(Module::Controls).await?);
        let sec = Security::new(db.module(Module::Controls).await?);
        let r = Records::new(db.module(Module::Documents).await?);
        let alice = actor("alice", TrustContext::Customer);
        let bob = actor("bob", TrustContext::Customer);
        let support = actor("support", TrustContext::Workforce);
        let approver = actor("approver", TrustContext::Workforce);
        let approver2 = actor("approver2", TrustContext::Workforce);
        let admin = actor("admin", TrustContext::Workforce);
        let officer = actor("officer", TrustContext::Workforce);
        for (who, roles) in [
            (&alice, vec![]),
            (&bob, vec![]),
            (&support, vec!["account_support"]),
            (&approver, vec!["security_approver"]),
            (&approver2, vec!["security_approver"]),
            (&admin, vec!["security_admin"]),
            (&officer, vec!["records_officer"]),
        ] {
            let person = Uuid::new_v4();
            sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
                .bind(&who.issuer)
                .bind(&who.subject)
                .bind(person)
                .execute(&db.migrator)
                .await?;
            for role in roles {
                sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
            }
            if who.trust == TrustContext::Customer {
                sqlx::query("INSERT INTO core.parties(id,issuer,subject,source_application,identity_digest) VALUES($1,$2,$3,$4,'fixture')").bind(Uuid::new_v4()).bind(&who.issuer).bind(&who.subject).bind(Uuid::new_v4()).execute(&db.migrator).await?;
                sqlx::query("INSERT INTO journeys.applications(id,issuer,subject,create_key,create_digest,product_version_id,terms_id,profile,state) VALUES($1,$2,$3,'k','d',$4,$5,$6,'DRAFT')")
                    .bind(Uuid::new_v4()).bind(&who.issuer).bind(&who.subject).bind(Uuid::new_v4()).bind(Uuid::new_v4())
                    .bind(json!({"fullName": format!("Synthetic {}", who.subject), "email": format!("{}@example.invalid", who.subject)}))
                    .execute(&db.migrator).await?;
            }
        }
        Ok(Self {
            db,
            c,
            sec,
            r,
            alice,
            bob,
            support,
            approver,
            approver2,
            admin,
            officer,
        })
    }
    async fn person(&self, who: &ActorContext) -> Result<Uuid> {
        Ok(self.c.person(who).await?.0)
    }
    async fn count(&self, sql: &'static str) -> Result<i64> {
        Ok(sqlx::query_scalar(sql).fetch_one(&self.db.migrator).await?)
    }
    async fn close(self) -> Result<()> {
        self.c.pool.close().await;
        self.sec.pool.close().await;
        self.r.pool.close().await;
        self.db.close().await
    }
}

#[tokio::test]
async fn recovery_verifies_ownership_scores_risk_and_requires_independent_approval() -> Result<()> {
    let s = S::new().await?;
    let alice = s.person(&s.alice).await?;
    // Customer self-service lost device: protective and immediate; replay-safe.
    let key = Uuid::new_v4();
    let own = s
        .sec
        .request_recovery(
            &s.c,
            &s.alice,
            key,
            &json!({"kind":"LOST_DEVICE","reason":"Phone lost"}),
        )
        .await?;
    assert_eq!(
        (own["state"].as_str(), own["channel"].as_str()),
        (Some("APPROVED"), Some("CUSTOMER_SELF"))
    );
    assert!(own.get("risk").is_none());
    assert_eq!(
        s.sec
            .request_recovery(
                &s.c,
                &s.alice,
                key,
                &json!({"kind":"LOST_DEVICE","reason":"Phone lost"})
            )
            .await?["id"],
        own["id"]
    );
    assert!(
        s.sec
            .request_recovery(
                &s.c,
                &s.alice,
                key,
                &json!({"kind":"LOST_DEVICE","reason":"Changed"})
            )
            .await
            .is_err()
    );
    assert!(
        s.sec
            .request_recovery(
                &s.c,
                &s.alice,
                Uuid::new_v4(),
                &json!({"kind":"CREDENTIAL_RESET","reason":"Forgot"})
            )
            .await
            .is_err()
    );
    let bob = s.person(&s.bob).await?;
    assert!(
        s.sec
            .request_recovery(
                &s.c,
                &s.alice,
                Uuid::new_v4(),
                &json!({"kind":"LOST_DEVICE","reason":"Theirs","personId":bob})
            )
            .await
            .is_err()
    );
    // Completion records evidence and a safe security notice.
    let done = s
        .sec
        .complete_recovery(id(&own["id"]), &json!({"issuerSessionsRevoked":true}))
        .await?;
    assert_eq!(done["state"], "EXECUTED");
    assert_eq!(
        s.sec.complete_recovery(id(&own["id"]), &json!({})).await?["state"],
        "EXECUTED"
    );
    let body: String = sqlx::query_scalar("SELECT body FROM data.notifications n CROSS JOIN LATERAL data.render_notification(n.template,n.params) WHERE template='RECOVERY_COMPLETED'").fetch_one(&s.db.migrator).await?;
    assert!(body.contains("signed out") && !body.contains(&alice.to_string()));
    // Staff-assisted credential reset: verification recorded; independent security approval required.
    let reset = json!({"personId":alice,"kind":"CREDENTIAL_RESET","reason":"Verified by phone","verification":"VERIFIED_CONTACT"});
    assert!(
        s.sec
            .request_recovery(
                &s.c,
                &s.support,
                Uuid::new_v4(),
                &json!({"personId":alice,"kind":"CREDENTIAL_RESET","reason":"No check"})
            )
            .await
            .is_err()
    );
    assert!(
        s.sec
            .request_recovery(&s.c, &s.approver, Uuid::new_v4(), &reset)
            .await
            .is_err()
    );
    let staff_person = s.person(&s.admin).await?;
    assert!(s.sec.request_recovery(&s.c, &s.support, Uuid::new_v4(), &json!({"personId":staff_person,"kind":"LOST_DEVICE","reason":"Staff target","verification":"VERIFIED_CONTACT"})).await.is_err());
    let pending = s
        .sec
        .request_recovery(&s.c, &s.support, Uuid::new_v4(), &reset)
        .await?;
    assert_eq!(pending["state"], "PENDING_APPROVAL");
    // The prior executed recovery within 30 days is itself a risk signal.
    assert_eq!(
        pending["risk"]["signals"],
        json!(["RECOVERY_WITHIN_30_DAYS"])
    );
    assert!(
        s.sec
            .complete_recovery(id(&pending["id"]), &json!({}))
            .await
            .is_err()
    );
    assert!(
        s.sec
            .decide_recovery(&s.c, &s.support, id(&pending["id"]), true, "Self approval")
            .await
            .is_err()
    );
    let approved = s
        .sec
        .decide_recovery(
            &s.c,
            &s.approver,
            id(&pending["id"]),
            true,
            "Ownership evidence reviewed",
        )
        .await?;
    assert_eq!(approved["state"], "APPROVED");
    let case_state: String = sqlx::query_scalar("SELECT state FROM controls.cases WHERE id=$1")
        .bind(id(&approved["caseId"]))
        .fetch_one(&s.db.migrator)
        .await?;
    assert_eq!(case_state, "CLOSED");
    assert!(
        s.sec
            .decide_recovery(
                &s.c,
                &s.approver2,
                id(&pending["id"]),
                false,
                "Second opinion"
            )
            .await
            .is_err()
    );
    assert_eq!(
        s.sec
            .complete_recovery(id(&pending["id"]), &json!({"passwordUpdateRequired":true}))
            .await?["state"],
        "EXECUTED"
    );
    // A recent contact change makes even a lost-device request HIGH risk: staff need approval; self opens a review case.
    sqlx::query("INSERT INTO core.contact_changes SELECT gen_random_uuid(),p.id,'f','new@example.invalid',2,'{}',now() FROM core.parties p WHERE p.subject='bob'").execute(&s.db.migrator).await?;
    let risky = s.sec.request_recovery(&s.c, &s.support, Uuid::new_v4(), &json!({"personId":bob,"kind":"LOST_DEVICE","reason":"Called in","verification":"VERIFIED_EVIDENCE_REVIEW"})).await?;
    assert_eq!(
        (risky["state"].as_str(), risky["risk"]["level"].as_str()),
        (Some("PENDING_APPROVAL"), Some("HIGH"))
    );
    let rejected = s
        .sec
        .decide_recovery(
            &s.c,
            &s.approver,
            id(&risky["id"]),
            false,
            "Contact changed yesterday; call back on old number",
        )
        .await?;
    assert_eq!(rejected["state"], "REJECTED");
    let self_risky = s
        .sec
        .request_recovery(
            &s.c,
            &s.bob,
            Uuid::new_v4(),
            &json!({"kind":"LOST_DEVICE","reason":"Lost it"}),
        )
        .await?;
    assert_eq!(self_risky["state"], "APPROVED");
    let next: String = sqlx::query_scalar("SELECT c.next_action FROM controls.cases c JOIN controls.recovery_requests r ON r.case_id=c.id WHERE r.id=$1").bind(id(&self_risky["id"])).fetch_one(&s.db.migrator).await?;
    assert_eq!(next, "REVIEW_SUSPICIOUS_RECOVERY");
    // Customers see only their own requests, without risk detail; staff see the queue.
    assert_eq!(
        s.sec
            .recoveries(&s.c, &s.alice)
            .await?
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        s.sec.recoveries(&s.c, &s.alice).await?[0]
            .get("risk")
            .is_none()
    );
    assert_eq!(
        s.sec
            .recoveries(&s.c, &s.support)
            .await?
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(s.sec.recoveries(&s.c, &s.officer).await.is_err());
    assert!(
        sqlx::query("UPDATE controls.recovery_requests SET reason='rewritten'")
            .execute(&s.db.migrator)
            .await
            .is_err()
    );
    assert!(
        s.count("SELECT count(*) FROM controls.recovery_history")
            .await?
            >= 8
    );
    s.close().await
}

#[tokio::test]
async fn access_review_and_findings_need_independent_attributable_decisions() -> Result<()> {
    let s = S::new().await?;
    assert!(s.sec.open_review(&s.c, &s.approver).await.is_err());
    let opened = s.sec.open_review(&s.c, &s.admin).await?;
    let review = id(&opened["review"]["id"]);
    let items = opened["review"]["items"].as_array().unwrap().clone();
    assert_eq!(
        items.len() as i64,
        s.count("SELECT count(*) FROM controls.role_grants WHERE revoked_at IS NULL")
            .await?
    );
    assert!(s.sec.open_review(&s.c, &s.admin).await.is_err());
    let approver = s.person(&s.approver).await?;
    let support = s.person(&s.support).await?;
    for item in &items {
        let grant = id(&item["grantId"]);
        let person = id(&item["personId"]);
        if person == approver {
            assert!(
                s.sec
                    .decide_item(&s.c, &s.approver, review, grant, "KEEP", "Self")
                    .await
                    .is_err()
            );
            s.sec
                .decide_item(&s.c, &s.approver2, review, grant, "KEEP", "Still required")
                .await?;
        } else if person == support {
            s.sec
                .decide_item(
                    &s.c,
                    &s.approver,
                    review,
                    grant,
                    "REVOKE",
                    "Left the support team",
                )
                .await?;
        } else {
            s.sec
                .decide_item(&s.c, &s.approver, review, grant, "KEEP", "Still required")
                .await?;
        }
    }
    let closed = s.sec.review(&s.c, &s.admin).await?;
    assert!(closed["review"]["closedAt"].is_string());
    let alice = s.person(&s.alice).await?;
    // The revoked grant takes effect at once.
    assert!(s.sec.request_recovery(&s.c, &s.support, Uuid::new_v4(), &json!({"personId":alice,"kind":"LOST_DEVICE","reason":"After revocation","verification":"VERIFIED_CONTACT"})).await.is_err());
    assert!(
        sqlx::query("UPDATE controls.access_review_items SET decision='KEEP'")
            .execute(&s.db.migrator)
            .await
            .is_err()
    );
    // Findings: release-blocking severity must be resolved by someone other than the reporter.
    let reported = s.sec.report_finding(&s.c, &s.admin, &json!({"reference":"SF-001","title":"Server-held tokens at rest","severity":"HIGH","area":"identity","description":"Synthetic review finding."})).await?;
    assert_eq!(reported["releaseBlocking"], 1);
    let finding = id(&reported["findings"][0]["id"]);
    assert!(
        s.sec
            .dispose_finding(&s.c, &s.approver, finding, "ACCEPTED_RISK", "Accept", None)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) SELECT gen_random_uuid(),person_id,'security_approver',now()+interval '1 day' FROM controls.person_links WHERE subject='admin'").execute(&s.db.migrator).await?;
    assert!(
        s.sec
            .dispose_finding(&s.c, &s.admin, finding, "RESOLVED", "Self resolution", None)
            .await
            .is_err()
    );
    let resolved = s
        .sec
        .dispose_finding(
            &s.c,
            &s.approver,
            finding,
            "RESOLVED",
            "Fixed and verified",
            Some("docs/implementation/evidence/WP-28.md"),
        )
        .await?;
    assert_eq!(resolved["releaseBlocking"], 0);
    assert!(
        s.sec
            .dispose_finding(&s.c, &s.approver2, finding, "RESOLVED", "Again", None)
            .await
            .is_err()
    );
    s.sec.report_finding(&s.c, &s.admin, &json!({"reference":"SF-002","title":"Fixed synthetic provider key","severity":"LOW","area":"simulator","description":"Local-only constant."})).await?;
    let low = s.sec.findings(&s.c, &s.approver).await?["findings"][1]["id"].clone();
    assert_eq!(
        s.sec
            .dispose_finding(
                &s.c,
                &s.approver,
                id(&low),
                "ACCEPTED_RISK",
                "Local synthetic profile only",
                None
            )
            .await?["findings"][1]["status"],
        "ACCEPTED_RISK"
    );
    assert!(s.sec.findings(&s.c, &s.support).await.is_err());
    s.close().await
}

#[tokio::test]
async fn privacy_restriction_and_personal_export_honour_purpose_and_subject_boundaries()
-> Result<()> {
    let s = S::new().await?;
    let alice = s.person(&s.alice).await?;
    let restriction =
        s.r.submit_request(
            &s.c,
            &s.alice,
            Uuid::new_v4(),
            &DataRequestInput {
                kind: "RESTRICTION".into(),
                detail: "No marketing or analytics".into(),
            },
        )
        .await?;
    let done =
        s.r.process_request(&s.c, &s.officer, id(&restriction["id"]))
            .await?;
    assert_eq!(done["outcome"], "PARTIALLY_RESTRICTED");
    let statuses: Vec<&str> = done["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        vec![
            "APPLIED",
            "APPLIED",
            "RESTRICTED_LEGAL_BASIS",
            "RESTRICTED_RETENTION"
        ]
    );
    // A later preference cannot re-enable a restricted purpose.
    sqlx::query("SELECT data.set_notification_preference($1,$2,'MARKETING',true)")
        .bind(serde_json::to_value(&s.alice)?)
        .bind(alice)
        .execute(&s.db.migrator)
        .await?;
    let enabled: bool = sqlx::query_scalar("SELECT data.purpose_enabled($1,'MARKETING')")
        .bind(alice)
        .fetch_one(&s.db.migrator)
        .await?;
    assert!(!enabled);
    let export =
        s.r.submit_request(
            &s.c,
            &s.alice,
            Uuid::new_v4(),
            &DataRequestInput {
                kind: "EXPORT".into(),
                detail: String::new(),
            },
        )
        .await?;
    let exported =
        s.r.process_request(&s.c, &s.officer, id(&export["id"]))
            .await?;
    assert_eq!(exported["outcome"], "PARTIALLY_RESTRICTED");
    let document = &exported["export"];
    assert_eq!(document["source"], "DOCUMENTS_PERSONAL_DATA_EXPORT_V1");
    assert_eq!(document["applications"].as_array().unwrap().len(), 1);
    assert_eq!(
        document["applications"][0]["profile"]["fullName"],
        "Synthetic alice"
    );
    assert!(
        !document.to_string().contains("Synthetic bob")
            && !document.to_string().contains("bob@example.invalid")
    );
    assert_eq!(
        document["processingRestrictions"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        document["identityEvidence"]["status"],
        "RESTRICTED_EVIDENCE"
    );
    let mine = s.r.customer_documents(&s.c, &s.alice).await?;
    assert_eq!(mine["dataRequests"].as_array().unwrap().len(), 2);
    assert_eq!(
        s.r.customer_documents(&s.c, &s.bob).await?["dataRequests"],
        json!([])
    );
    s.close().await
}
