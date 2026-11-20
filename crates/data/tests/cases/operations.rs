//! WP-26: AT-30 monitoring, FR-DATA-05 seeded defects, FR-CTRL-05 snapshots, FR-OPS-02 trace and safe notifications.
use anyhow::Result;
use bank_controls::{cases::Cases, entitlements::Entitlements, monitoring::Monitoring};
use bank_core::{
    Core,
    accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest},
};
use bank_data::{Data, alerts::Alerts};
use bank_platform::{
    database::Module,
    domain::{
        AccountId, ActorContext, Currency, EventEnvelope, EventId, Money, PartyId, TrustContext,
        fingerprint,
    },
    testing::TestDatabase,
};
use chrono::{Duration, Utc};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

struct O {
    db: TestDatabase,
    core: Core,
    data: Data,
    c: Entitlements,
    m: Monitoring,
    a: Alerts,
    alice: ActorContext,
    bob: ActorContext,
    reviewer: ActorContext,
    approver: ActorContext,
    steward: ActorContext,
    operator: ActorContext,
    accounts: Vec<AccountId>,
    parties: Vec<PartyId>,
    clearing: AccountId,
}
fn actor(subject: &str, trust: TrustContext) -> ActorContext {
    ActorContext {
        issuer: "urn:operations-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "test".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
fn id(v: &Value) -> Uuid {
    serde_json::from_value(v.clone()).unwrap()
}
struct Gates;
impl OpeningPolicy for Gates {
    fn validate<'a>(&'a self, r: &'a OpeningRequest) -> BoxFuture<'a, Result<ActivationEvidence>> {
        Box::pin(async move {
            Ok(ActivationEvidence {
                application_id: r.application_id,
                party_id: r.party_id,
                product_version_id: r.product_version_id,
                acceptance_id: r.acceptance_id,
                controls_revision: "synthetic-port-fixture".into(),
                expires_at: Utc::now() + Duration::minutes(5),
            })
        })
    }
}
impl O {
    async fn new() -> Result<Self> {
        let db = TestDatabase::new().await?;
        let core = Core::new(db.module(Module::Core).await?);
        let data_pool = db.module(Module::Data).await?;
        let data = Data::new(data_pool.clone(), Arc::new(core.clone()));
        let controls_pool = db.module(Module::Controls).await?;
        let c = Entitlements::new(controls_pool.clone());
        let m = Monitoring::new(controls_pool);
        let a = Alerts::new(data_pool);
        let alice = actor("alice", TrustContext::Customer);
        let bob = actor("bob", TrustContext::Customer);
        let mut parties = vec![];
        for who in [&alice, &bob] {
            let party = core
                .ensure_party(who, Uuid::new_v4(), &fingerprint(&who.subject)?)
                .await?;
            c.link_person(who, party).await?;
            parties.push(party);
        }
        let reviewer = actor("reviewer", TrustContext::Workforce);
        let approver = actor("approver", TrustContext::Workforce);
        let steward = actor("steward", TrustContext::Workforce);
        let operator = actor("operator", TrustContext::Workforce);
        for (who, roles, queue) in [
            (&reviewer, vec!["case_reviewer"], Some("REVIEW")),
            (&approver, vec!["case_approver"], Some("APPROVE")),
            (&steward, vec!["data_steward"], None),
            (&operator, vec!["operations_replay"], None),
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
            if let Some(capability) = queue {
                sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('controls-operations',$1,$2)").bind(person).bind(capability).execute(&db.migrator).await?;
            }
        }
        let product = Uuid::new_v4();
        sqlx::query("INSERT INTO core.product_versions(id,code,version,effective_from,effective_until,document,approval_id,maker_person,checker_person) VALUES($1,'OPS_FIXTURE',1,current_date-1,current_date+30,'{\"label\":\"SYNTHETIC\",\"chartVersion\":\"synthetic-chart-v1\"}',gen_random_uuid(),gen_random_uuid(),gen_random_uuid())").bind(product).execute(&db.migrator).await?;
        let mut accounts = vec![];
        for party in &parties {
            let request = OpeningRequest {
                application_id: Uuid::new_v4(),
                party_id: *party,
                product_version_id: product,
                acceptance_id: Uuid::new_v4(),
            };
            let account = core.stage_account(&request).await?;
            core.activate(account, &request, &Gates).await?;
            accounts.push(account);
        }
        for (n, account) in accounts.iter().enumerate() {
            sqlx::query("INSERT INTO core.account_relationships VALUES($1,(SELECT person_id FROM controls.person_links WHERE subject=$2),'OWNER',NULL) ON CONFLICT DO NOTHING")
                .bind(account.0)
                .bind(if n == 0 { "alice" } else { "bob" })
                .execute(&db.migrator)
                .await?;
        }
        let clearing = AccountId::new();
        sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'SETTLEMENT','ACTIVE',false)").bind(clearing.0).execute(&db.migrator).await?;
        // Disposable database: thresholds at zero so seeded defects are visible immediately.
        sqlx::query("UPDATE data.quality_thresholds SET seconds=0")
            .execute(&db.migrator)
            .await?;
        Ok(Self {
            db,
            core,
            data,
            c,
            m,
            a,
            alice,
            bob,
            reviewer,
            approver,
            steward,
            operator,
            accounts,
            parties,
            clearing,
        })
    }
    async fn fund(&self, n: usize, minor: i64) -> Result<Uuid> {
        let op = Uuid::new_v4();
        self.core
            .fund_synthetic(
                op,
                self.accounts[n],
                self.clearing,
                Money::new(minor, Currency::synthetic()),
            )
            .await?;
        Ok(op)
    }
    async fn events(&self) -> Result<Vec<EventEnvelope>> {
        let docs: Vec<Value> = sqlx::query_scalar(
            "SELECT document FROM core.outbox ORDER BY created_at,aggregate_id,aggregate_version",
        )
        .fetch_all(&self.db.migrator)
        .await?;
        Ok(docs
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()?)
    }
    /// Deliver every Core event (twice) to the Data consumer, optionally skipping a type.
    async fn project(&self, skip: &[&str]) -> Result<()> {
        for event in self.events().await? {
            if skip.contains(&event.event_type.as_str()) {
                continue;
            }
            self.data.consume(&event).await?;
            self.data.consume(&event).await?;
        }
        Ok(())
    }
    async fn count(&self, sql: &'static str) -> Result<i64> {
        Ok(sqlx::query_scalar(sql).fetch_one(&self.db.migrator).await?)
    }
    async fn person(&self, who: &ActorContext) -> Result<Uuid> {
        Ok(self.c.person(who).await?.0)
    }
    async fn close(self) -> Result<()> {
        self.core.pool.close().await;
        self.c.pool.close().await;
        self.a.pool.close().await;
        self.db.close().await
    }
}

#[tokio::test]
async fn duplicate_durable_events_and_replay_produce_one_economic_alert_with_independent_disposition()
-> Result<()> {
    let o = O::new().await?;
    let mut ops = vec![];
    for _ in 0..3 {
        ops.push(o.fund(0, 9500).await?);
    }
    o.fund(1, 9500).await?;
    o.project(&[]).await?;
    // A conflicting duplicate of an original journal event is quarantined, never projected twice.
    let original = o
        .events()
        .await?
        .into_iter()
        .find(|e| e.event_type == "journal.batch-posted")
        .unwrap();
    let mut duplicate = original.clone();
    duplicate.data = json!({"batchId":original.aggregate_id,"operationId":Uuid::new_v4()});
    assert!(!o.data.consume(&duplicate).await?);
    let mut replayed = original.clone();
    replayed.event_id = EventId::new();
    assert!(!o.data.consume(&replayed).await?);
    assert_eq!(
        o.count("SELECT count(*) FROM data.transaction_views WHERE account_id IN (SELECT id FROM core.account_registry)")
            .await?,
        4
    );
    assert_eq!(o.m.tick(500).await?, 1);
    assert_eq!(o.m.tick(500).await?, 0);
    let scan =
        o.m.scan(
            &o.c,
            &o.reviewer,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::minutes(1),
        )
        .await?;
    // Only the third credit completes the pattern; re-evaluating it finds the existing episode.
    assert_eq!(
        (scan["created"].as_i64(), scan["existing"].as_i64()),
        (Some(0), Some(1))
    );
    assert!(
        o.m.scan(
            &o.c,
            &o.steward,
            Utc::now() - Duration::hours(1),
            Utc::now()
        )
        .await
        .is_err()
    );
    let view = o.m.alerts(&o.c, &o.reviewer).await?;
    let alerts = view["alerts"].as_array().unwrap();
    assert_eq!(alerts.len(), 1);
    let alert = &alerts[0];
    assert_eq!(alert["ruleCode"], "STRUCTURED_CREDITS");
    assert_eq!(alert["accountId"], json!(o.accounts[0].0));
    assert_eq!(alert["anchorOperation"], json!(ops[0]));
    assert_eq!(alert["evidence"]["facts"].as_array().unwrap().len(), 3);
    assert_eq!(alert["caseState"], "OPEN");
    assert_eq!(
        o.count("SELECT count(*) FROM controls.cases WHERE kind='MONITORING'")
            .await?,
        1
    );
    // Monitoring cases never reach the customer (no tipping off).
    assert_eq!(
        o.count("SELECT count(*) FROM controls.cases WHERE kind='MONITORING' AND customer_subject IS NOT NULL")
            .await?,
        0
    );
    let cases = Cases::new(o.c.pool.clone())
        .customer_cases(&o.alice)
        .await?;
    assert_eq!(cases["items"], json!([]));
    assert!(o.m.alerts(&o.c, &o.alice).await.is_err());
    // Independent disposition with attributable people.
    let alert_id = id(&alert["id"]);
    o.m.propose(
        &o.c,
        &o.reviewer,
        alert_id,
        "NO_FURTHER_ACTION",
        "Synthetic payroll pattern explained",
    )
    .await?;
    assert!(
        o.m.propose(
            &o.c,
            &o.reviewer,
            alert_id,
            "REFER_FINANCIAL_CRIME",
            "changed"
        )
        .await
        .is_err()
    );
    assert!(o.m.approve(&o.c, &o.reviewer, alert_id).await.is_err());
    let approved = o.m.approve(&o.c, &o.approver, alert_id).await?;
    assert_eq!(approved["disposition"]["disposition"], "NO_FURTHER_ACTION");
    assert_eq!(
        approved["disposition"]["checkerPerson"],
        json!(o.person(&o.approver).await?)
    );
    let closed: String = sqlx::query_scalar("SELECT state FROM controls.cases WHERE id=$1")
        .bind(id(&alert["caseId"]))
        .fetch_one(&o.db.migrator)
        .await?;
    assert_eq!(closed, "CLOSED");
    assert_eq!(
        o.m.approve(&o.c, &o.approver, alert_id).await?["id"],
        alert["id"]
    );
    assert!(
        sqlx::query("UPDATE controls.monitoring_dispositions SET rationale='rewritten'")
            .execute(&o.db.migrator)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM controls.monitoring_alerts")
            .execute(&o.db.migrator)
            .await
            .is_err()
    );
    o.close().await
}

#[tokio::test]
async fn seeded_data_defects_open_owned_alerts_and_repair_cannot_write_balances() -> Result<()> {
    let o = O::new().await?;
    let funded = o.fund(0, 1000).await?;
    // Project everything except journal and account facts: journal batches are missing from the read model.
    o.project(&["journal.batch-posted", "account.changed"])
        .await?;
    let scan = o.a.quality_scan().await?;
    assert!(scan["opened"].as_i64().unwrap() >= 3);
    let view = o.a.quality_view(&o.c, &o.steward).await?;
    let open = |code: &str| {
        view["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["checkCode"] == code && a["state"] == "OPEN")
            .cloned()
            .collect::<Vec<_>>()
    };
    let missing = open("MISSING_FINANCIAL_EVENT");
    assert_eq!(missing.len(), 1);
    assert_eq!(
        missing[0]["nextAction"],
        "CHECK_RELAY_THEN_REPAIR_PROJECTION"
    );
    assert_eq!(open("STALE_PROJECTION").len(), 1);
    assert!(
        open("UNDELIVERED_EVENTS")
            .iter()
            .any(|a| a["subject"] == "core")
    );
    assert_eq!(view["freshness"]["status"], "STALE");
    // Rescanning the same defects opens nothing new. (With a zero threshold, the scan's own undelivered
    // case events may legitimately open an UNDELIVERED_EVENTS alert for the controls outbox.)
    let defects = "SELECT count(*) FROM data.quality_alerts WHERE check_code<>'UNDELIVERED_EVENTS'";
    let before_rescan = o.count(defects).await?;
    o.a.quality_scan().await?;
    assert_eq!(o.count(defects).await?, before_rescan);
    assert_eq!(
        o.count("SELECT count(*) FROM controls.cases WHERE kind='DATA'")
            .await?,
        o.count("SELECT count(*) FROM data.quality_alerts").await?
    );
    // Repair re-projects from the immutable journal into Data only; the Data role cannot touch balances.
    let batch = id(&missing[0]["observation"]["batchId"]);
    let before: i64 = sqlx::query_scalar("SELECT booked FROM core.ledger_accounts WHERE id=$1")
        .bind(o.accounts[0].0)
        .fetch_one(&o.db.migrator)
        .await?;
    assert!(o.a.repair(&o.c, &o.reviewer, batch).await.is_err());
    let repaired = o.a.repair(&o.c, &o.steward, batch).await?;
    assert_eq!(repaired["rowsInserted"], 2);
    assert_eq!(
        o.a.repair(&o.c, &o.steward, batch).await?["rowsInserted"],
        0
    );
    assert!(
        sqlx::query("UPDATE core.ledger_accounts SET booked=booked+1 WHERE id=$1")
            .bind(o.accounts[0].0)
            .execute(&o.a.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("INSERT INTO core.journal_entries VALUES($1,$2,1)")
            .bind(batch)
            .bind(o.accounts[1].0)
            .execute(&o.a.pool)
            .await
            .is_err()
    );
    let after: i64 = sqlx::query_scalar("SELECT booked FROM core.ledger_accounts WHERE id=$1")
        .bind(o.accounts[0].0)
        .fetch_one(&o.db.migrator)
        .await?;
    assert_eq!(before, after);
    // The repaired transaction lacks its account view: a broken reference, cleared by replaying account facts.
    o.a.quality_scan().await?;
    let view = o.a.quality_view(&o.c, &o.steward).await?;
    assert!(
        view["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["checkCode"] == "BROKEN_REFERENCE" && a["state"] == "OPEN")
    );
    o.project(&[]).await?;
    assert_eq!(
        o.count("SELECT count(*) FROM data.transaction_views")
            .await?,
        2
    );
    // Relay restored for every module; later case events must not count as an outage.
    sqlx::query("DO $$ DECLARE s text; BEGIN FOREACH s IN ARRAY ARRAY['platform','identity','core','controls','journeys','payments','finance','documents','data','simulator'] LOOP EXECUTE format('UPDATE %I.outbox SET delivered_at=now() WHERE delivered_at IS NULL',s); END LOOP; END $$")
        .execute(&o.db.migrator)
        .await?;
    sqlx::query(
        "UPDATE data.quality_thresholds SET seconds=3600 WHERE check_code='UNDELIVERED_EVENTS'",
    )
    .execute(&o.db.migrator)
    .await?;
    o.a.quality_scan().await?;
    let view = o.a.quality_view(&o.c, &o.steward).await?;
    assert_eq!(view["freshness"]["status"], "CURRENT");
    assert!(
        view["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["state"] == "CLEARED")
    );
    let alert = id(&missing[0]["id"]);
    assert!(
        o.a.acknowledge(&o.c, &o.reviewer, alert, "Relay restored")
            .await
            .is_err()
    );
    assert_eq!(
        o.a.acknowledge(
            &o.c,
            &o.steward,
            alert,
            "Relay restored; projection repaired"
        )
        .await?["acknowledged"],
        true
    );
    let state: String = sqlx::query_scalar("SELECT state FROM controls.cases WHERE id=$1")
        .bind(id(&missing[0]["caseId"]))
        .fetch_one(&o.db.migrator)
        .await?;
    assert_eq!(state, "CLOSED");
    // A quarantined (conflicting) event is a separate owned defect.
    let mut forged = o
        .events()
        .await?
        .into_iter()
        .find(|e| e.event_type == "journal.batch-posted")
        .unwrap();
    forged.data = json!({"batchId":forged.aggregate_id,"operationId":Uuid::new_v4()});
    assert!(!o.data.consume(&forged).await?);
    assert_eq!(o.a.quality_scan().await?["opened"], 1);
    // Fixed-snapshot operations report reconciles to the underlying counts.
    let snapshot = o.m.snapshot(&o.c, &o.reviewer).await?;
    let counts = &snapshot["counts"];
    assert_eq!(
        counts["cases"]["openByKind"]["DATA"].as_i64().unwrap_or(0),
        o.count("SELECT count(*) FROM controls.cases WHERE kind='DATA' AND state<>'CLOSED'")
            .await?
    );
    assert_eq!(
        counts["dataQuality"]["openAlerts"].as_i64().unwrap(),
        o.count("SELECT count(*) FROM data.quality_alerts WHERE state='OPEN'")
            .await?
    );
    assert_eq!(
        counts["integrations"]["deadLetters"]["data"]
            .as_i64()
            .unwrap(),
        o.count("SELECT count(*) FROM data.dead_letters WHERE state='PENDING'")
            .await?
    );
    assert_eq!(snapshot["digest"].as_str().unwrap().len(), 64);
    assert!(o.m.snapshot(&o.c, &o.alice).await.is_err());
    // End-to-end trace by durable identifiers only.
    let trace = o.m.trace(&o.c, &o.reviewer, funded).await?;
    assert_eq!(trace["journal"][0]["operationId"], json!(funded));
    assert_eq!(trace["journal"][0]["projected"], true);
    assert!(
        trace["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["eventType"] == "journal.batch-posted")
    );
    assert!(!trace.to_string().contains("alice"));
    o.close().await
}

#[tokio::test]
async fn notifications_are_deduplicated_purpose_scoped_redacted_and_retried_through_outage()
-> Result<()> {
    let o = O::new().await?;
    let alice = o.person(&o.alice).await?;
    let bob = o.person(&o.bob).await?;
    sqlx::query("INSERT INTO core.contacts(party_id,email,version,verification_resource,evidence) VALUES($1,'alice.synthetic@example.invalid',1,gen_random_uuid(),'{}')")
        .bind(o.parties[0].0)
        .execute(&o.db.migrator)
        .await?;
    // Sources: a developer token (security), and an outgoing payment projected from a durable event.
    sqlx::query("INSERT INTO core.consumer_tokens(id,person_id,actor_context,label,scope,secret_hash) VALUES(gen_random_uuid(),$1,'{}','Read','accounts:read',repeat('a',64))")
        .bind(alice)
        .execute(&o.db.migrator)
        .await?;
    o.fund(0, 50000).await?;
    let batch = Uuid::new_v4();
    let mut tx = o.db.migrator.begin().await?;
    sqlx::query("INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint) VALUES($1,$1,'INTERNAL_PAYMENT','fixture')").bind(batch).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO core.journal_entries VALUES($1,$2,-12345),($1,$3,12345)")
        .bind(batch)
        .bind(o.accounts[0].0)
        .bind(o.accounts[1].0)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO data.transaction_views(batch_id,account_id,operation_id,document,origin_event) VALUES($1,$2,$1,'{}',gen_random_uuid()),($1,$3,$1,'{}',gen_random_uuid())")
        .bind(batch)
        .bind(o.accounts[0].0)
        .bind(o.accounts[1].0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Bob turns off activity notices; mandatory purposes cannot be turned off.
    o.a.set_preference(&o.c, &o.bob, "ACTIVITY", false).await?;
    assert!(
        o.a.set_preference(&o.c, &o.bob, "SERVICE", false)
            .await
            .is_err()
    );
    assert!(
        o.a.set_preference(&o.c, &o.steward, "ACTIVITY", false)
            .await
            .is_err()
    );
    // Bob receives the credit, not a debit: no PAYMENT_SENT for him regardless.
    assert_eq!(o.a.derive(100).await?, 2);
    assert_eq!(o.a.derive(100).await?, 0);
    let payment: Value = sqlx::query_scalar(
        "SELECT to_jsonb(n) FROM data.notifications n WHERE template='PAYMENT_SENT'",
    )
    .fetch_one(&o.db.migrator)
    .await?;
    assert_eq!(
        (
            payment["person_id"].clone(),
            payment["channel"].as_str(),
            payment["recipient"].as_str()
        ),
        (
            json!(alice),
            Some("SYNTHETIC_EMAIL"),
            Some("a***@example.invalid")
        )
    );
    // Outage: the email channel fails, backs off, and recovers without duplicate delivery.
    assert!(
        o.a.set_channel(
            &o.c,
            &o.steward,
            "SYNTHETIC_EMAIL",
            false,
            "Synthetic outage"
        )
        .await
        .is_err()
    );
    o.a.set_channel(
        &o.c,
        &o.operator,
        "SYNTHETIC_EMAIL",
        false,
        "Synthetic outage drill",
    )
    .await?;
    assert_eq!(o.a.deliver(100).await?["failed"], 2);
    assert_eq!(o.a.deliver(100).await?["failed"], 0);
    o.a.set_channel(
        &o.c,
        &o.operator,
        "SYNTHETIC_EMAIL",
        true,
        "Outage resolved",
    )
    .await?;
    sqlx::query("UPDATE data.notifications SET next_attempt_at=now() WHERE state='PENDING'")
        .execute(&o.db.migrator)
        .await?;
    assert_eq!(o.a.deliver(100).await?["delivered"], 2);
    assert_eq!(o.a.deliver(100).await?["delivered"], 0);
    assert_eq!(
        o.count("SELECT count(*) FROM data.notification_attempts WHERE outcome='DELIVERED'")
            .await?,
        2
    );
    // Captured content is safe: no amounts, balances, full identifiers or token material.
    let bodies: Vec<String> =
        sqlx::query_scalar("SELECT subject||' '||body FROM data.notification_attempts")
            .fetch_all(&o.db.migrator)
            .await?;
    for body in &bodies {
        assert!(!body.contains("123.45") && !body.contains("12345") && !body.contains("500.00"));
        assert!(!body.contains(&o.accounts[0].0.to_string()) && !body.contains("aaaa"));
    }
    // A preference-disabled activity notice is recorded as suppressed, never delivered.
    let other = Uuid::new_v4();
    let mut tx = o.db.migrator.begin().await?;
    sqlx::query("INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint) VALUES($1,$1,'INTERNAL_PAYMENT','fixture')").bind(other).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO core.journal_entries VALUES($1,$2,-100),($1,$3,100)")
        .bind(other)
        .bind(o.accounts[1].0)
        .bind(o.accounts[0].0)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO data.transaction_views(batch_id,account_id,operation_id,document,origin_event) VALUES($1,$2,$1,'{}',gen_random_uuid())").bind(other).bind(o.accounts[1].0).execute(&mut *tx).await?;
    tx.commit().await?;
    assert_eq!(o.a.derive(100).await?, 1);
    let suppressed: (String, Option<String>) = sqlx::query_as(
        "SELECT state,suppression_reason FROM data.notifications WHERE person_id=$1",
    )
    .bind(bob)
    .fetch_one(&o.db.migrator)
    .await?;
    assert_eq!(
        suppressed,
        (
            "SUPPRESSED".into(),
            Some("PURPOSE_PREFERENCE_DISABLED".into())
        )
    );
    assert_eq!(o.a.deliver(100).await?["delivered"], 0);
    // Customers only see their own messages.
    let mine = o.a.messages(&o.c, &o.alice).await?;
    assert_eq!(mine["messages"].as_array().unwrap().len(), 2);
    assert!(
        o.a.messages(&o.c, &o.bob).await?["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Six failed attempts become an owned NOTIFICATION_FAILED alert until acknowledged.
    o.a.set_channel(&o.c, &o.operator, "IN_APP", false, "Inbox outage drill")
        .await?;
    sqlx::query("INSERT INTO data.notifications(id,dedupe_key,person_id,purpose,template,params,source_ref,channel,recipient,state,next_attempt_at) VALUES(gen_random_uuid(),'fixture:failing',$1,'SERVICE','DATA_REQUEST_COMPLETED','{\"kind\":\"ACCESS\"}',gen_random_uuid(),'IN_APP','in-app inbox','PENDING',now())")
        .bind(bob)
        .execute(&o.db.migrator)
        .await?;
    for _ in 0..6 {
        sqlx::query("UPDATE data.notifications SET next_attempt_at=now() WHERE state='PENDING'")
            .execute(&o.db.migrator)
            .await?;
        o.a.deliver(100).await?;
    }
    let failed: String = sqlx::query_scalar(
        "SELECT state FROM data.notifications WHERE dedupe_key='fixture:failing'",
    )
    .fetch_one(&o.db.migrator)
    .await?;
    assert_eq!(failed, "FAILED");
    o.a.quality_scan().await?;
    let view = o.a.quality_view(&o.c, &o.steward).await?;
    let alert = view["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["checkCode"] == "NOTIFICATION_FAILED")
        .unwrap()
        .clone();
    o.a.acknowledge(
        &o.c,
        &o.steward,
        id(&alert["id"]),
        "Customer informed through support",
    )
    .await?;
    let staff = o.a.notifications(&o.c, &o.operator).await?;
    assert_eq!(staff["counts"]["FAILED"], 1);
    o.close().await
}

#[tokio::test]
async fn critical_alerts_page_the_primary_escalate_to_the_secondary_and_need_the_paged_person()
-> Result<()> {
    let o = O::new().await?;
    let reviewer = o.person(&o.reviewer).await?;
    let approver = o.person(&o.approver).await?;
    assert!(
        o.m.set_oncall(&o.c, &o.steward, reviewer, "PRIMARY", true)
            .await
            .is_err()
    );
    o.m.set_oncall(&o.c, &o.operator, reviewer, "PRIMARY", true)
        .await?;
    o.m.set_oncall(&o.c, &o.operator, approver, "SECONDARY", true)
        .await?;
    // A projection gap (high severity) is a critical source.
    o.fund(0, 1000).await?;
    o.a.quality_scan().await?;
    let opened = o.m.page_tick().await?;
    assert!(opened["opened"].as_i64().unwrap() >= 1);
    assert_eq!(o.m.page_tick().await?["opened"], 0);
    let view = o.m.pages(&o.c, &o.reviewer).await?;
    let page = view["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| {
            p["sourceKind"] == "DATA_QUALITY"
                && p["summary"]
                    .as_str()
                    .unwrap()
                    .contains("missing financial event")
        })
        .unwrap()
        .clone();
    assert_eq!(page["pagedYou"], true);
    assert!(page["deliverySeconds"].as_f64().unwrap() <= 60.0);
    let delivered: i64 = sqlx::query_scalar("SELECT count(*) FROM controls.page_deliveries WHERE tier='PRIMARY' AND channel='SYNTHETIC_PAGER'").fetch_one(&o.db.migrator).await?;
    assert!(delivered >= 1);
    // Unacknowledged past the policy window: escalate once to the secondary, who may then acknowledge.
    sqlx::query("UPDATE controls.paging_policy SET ack_seconds=0")
        .execute(&o.db.migrator)
        .await?;
    assert!(o.m.page_tick().await?["escalated"].as_i64().unwrap() >= 1);
    assert_eq!(o.m.page_tick().await?["escalated"], 0);
    let id: Uuid = serde_json::from_value(page["id"].clone())?;
    assert!(o.m.acknowledge(&o.c, &o.steward, id).await.is_err());
    assert_eq!(
        o.m.acknowledge(&o.c, &o.approver, id).await?["acknowledged"],
        true
    );
    assert_eq!(
        o.m.acknowledge(&o.c, &o.approver, id).await?["acknowledged"],
        false
    );
    assert!(o.m.acknowledge(&o.c, &o.reviewer, id).await.is_err());
    assert!(
        sqlx::query("UPDATE controls.pages SET summary='rewritten'")
            .execute(&o.db.migrator)
            .await
            .is_err()
    );
    let metrics = o.m.metrics_snapshot().await?;
    assert_eq!(metrics["unbalancedBatches"], 0);
    assert!(metrics["qualityAlertsOpen"].as_i64().unwrap() >= 1);
    assert!(metrics["outbox"]["core"]["undelivered"].as_i64().unwrap() >= 1);
    o.close().await
}
