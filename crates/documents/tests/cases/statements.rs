//! WP-25: AT-15 statement versions/retrieval and AT-16 retention, legal hold and data requests.
use anyhow::Result;
use bank_controls::entitlements::Entitlements;
use bank_core::Core;
use bank_documents::{
    retention::{DataRequestInput, HoldInput, Restricted},
    statements::{AccessDenied, NotReady, Records},
};
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, Currency, Money, TrustContext},
    testing::TestDatabase,
};
use chrono::NaiveDate;
use serde_json::{Value, json};
use uuid::Uuid;

const SETTLEMENT: &str = "4f1a780b-3e31-48ef-a294-000000000003";
struct F {
    db: TestDatabase,
    core: Core,
    c: Entitlements,
    r: Records,
    alice: ActorContext,
    bob: ActorContext,
    officer: ActorContext,
    approver: ActorContext,
    support: ActorContext,
    nobody: ActorContext,
    account: Uuid,
    alice_person: Uuid,
}
fn actor(subject: &str, trust: TrustContext) -> ActorContext {
    ActorContext {
        issuer: "urn:statement-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "fixture-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
fn day(s: &str) -> NaiveDate {
    s.parse().unwrap()
}
fn id(v: &Value) -> Uuid {
    serde_json::from_value(v.clone()).unwrap()
}
fn denied(e: anyhow::Error) -> String {
    e.downcast_ref::<AccessDenied>()
        .map(|d| d.0.clone())
        .unwrap_or_else(|| format!("unexpected: {e:#}"))
}
fn restricted(e: anyhow::Error) -> String {
    e.downcast_ref::<Restricted>()
        .and_then(|r| r.0["status"].as_str().map(String::from))
        .unwrap_or_else(|| format!("unexpected: {e:#}"))
}
impl F {
    async fn new() -> Result<Self> {
        let db = TestDatabase::new().await?;
        let core = Core::new(db.module(Module::Core).await?);
        let c = Entitlements::new(db.module(Module::Controls).await?);
        let r = Records::new(db.module(Module::Documents).await?);
        let alice = actor("alice", TrustContext::Customer);
        let bob = actor("bob", TrustContext::Customer);
        let officer = actor("officer", TrustContext::Workforce);
        let approver = actor("approver", TrustContext::Workforce);
        let support = actor("support", TrustContext::Workforce);
        let nobody = actor("nobody", TrustContext::Workforce);
        let mut alice_person = Uuid::nil();
        for (who, role) in [
            (&alice, None),
            (&bob, None),
            (&officer, Some("records_officer")),
            (&approver, Some("records_approver")),
            (&support, Some("account_support")),
            (&nobody, None),
        ] {
            let person = Uuid::new_v4();
            if who.subject == "alice" {
                alice_person = person;
            }
            sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
                .bind(&who.issuer)
                .bind(&who.subject)
                .bind(person)
                .execute(&db.migrator)
                .await?;
            if let Some(role) = role {
                sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
            }
        }
        let product = Uuid::new_v4();
        let party = Uuid::new_v4();
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO core.product_versions VALUES($1,'STATEMENT_FIXTURE',1,'2026-01-01','2030-01-01','{\"label\":\"SYNTHETIC\",\"chartVersion\":\"synthetic-chart-v1\"}',$2,$3,$4,now())").bind(product).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(Uuid::new_v4()).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.parties(id,issuer,subject,source_application,identity_digest) VALUES($1,$2,$3,$4,'fixture')").bind(party).bind(&alice.issuer).bind(&alice.subject).bind(Uuid::new_v4()).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'CUSTOMER','ACTIVE',false),($2,'SETTLEMENT','ACTIVE',false) ON CONFLICT DO NOTHING").bind(account).bind(Uuid::parse_str(SETTLEMENT)?).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.account_registry(id,application_id,party_id,product_version_id,acceptance_id,opening_digest,created_at) VALUES($1,$2,$3,$4,$5,'fixture','2026-08-20T10:00:00Z')").bind(account).bind(Uuid::new_v4()).bind(party).bind(product).bind(Uuid::new_v4()).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.account_relationships VALUES($1,$2,'OWNER',NULL)")
            .bind(account)
            .bind(alice_person)
            .execute(&db.migrator)
            .await?;
        Ok(Self {
            db,
            core,
            c,
            r,
            alice,
            bob,
            officer,
            approver,
            support,
            nobody,
            account,
            alice_person,
        })
    }
    async fn clock(&self, at: &str) -> Result<()> {
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE OR REPLACE FUNCTION core.periodic_clock() RETURNS timestamptz LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$ SELECT '{at}T00:00:00Z'::timestamptz $$"))).execute(&self.db.migrator).await?;
        Ok(())
    }
    /// Native runtime funding: Core assigns the booking date from its clock.
    async fn fund(&self, amount: i64) -> Result<()> {
        self.core
            .fund_synthetic(
                Uuid::new_v4(),
                AccountId(self.account),
                AccountId(Uuid::parse_str(SETTLEMENT)?),
                Money::new(amount, Currency::synthetic()),
            )
            .await?;
        Ok(())
    }
    /// Owner-path balanced fixture posting with an explicit booking date (corrections and backdating cases).
    async fn post(
        &self,
        date: &str,
        kind: &str,
        delta: i64,
        original: Option<Uuid>,
    ) -> Result<Uuid> {
        let batch = Uuid::new_v4();
        let mut tx = self.db.migrator.begin().await?;
        sqlx::query("INSERT INTO core.journal_batches(id,operation_id,posting_kind,fingerprint,business_date,original_batch) VALUES($1,$2,$3,'fixture',$4::date,$5)").bind(batch).bind(Uuid::new_v4()).bind(kind).bind(date).bind(original).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO core.journal_entries VALUES($1,$2,$3),($1,$4,$5)")
            .bind(batch)
            .bind(self.account)
            .bind(delta)
            .bind(Uuid::parse_str(SETTLEMENT)?)
            .bind(-delta)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(batch)
    }
    async fn download(&self, who: &ActorContext, version: Uuid) -> Result<Value> {
        let link = self.r.link(&self.c, who, version).await?;
        self.r
            .redeem(&self.c, who, link["token"].as_str().unwrap())
            .await
    }
    async fn count(&self, sql: &'static str) -> Result<i64> {
        Ok(sqlx::query_scalar(sql).fetch_one(&self.db.migrator).await?)
    }
    async fn fail_objects(&self, delete: bool) -> Result<()> {
        sqlx::query("CREATE OR REPLACE FUNCTION documents.fail_fixture() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'object store unavailable'; END $$").execute(&self.db.migrator).await?;
        let trigger = if delete {
            "CREATE TRIGGER fail_fixture BEFORE DELETE ON documents.statement_objects FOR EACH ROW EXECUTE FUNCTION documents.fail_fixture()"
        } else {
            "CREATE TRIGGER fail_fixture BEFORE INSERT ON documents.statement_objects FOR EACH ROW EXECUTE FUNCTION documents.fail_fixture()"
        };
        sqlx::query(trigger).execute(&self.db.migrator).await?;
        Ok(())
    }
    async fn restore_objects(&self) -> Result<()> {
        sqlx::query("DROP TRIGGER fail_fixture ON documents.statement_objects")
            .execute(&self.db.migrator)
            .await?;
        Ok(())
    }
    async fn close(self) -> Result<()> {
        self.core.pool.close().await;
        self.c.pool.close().await;
        self.r.pool.close().await;
        self.db.close().await
    }
}

#[tokio::test]
async fn statements_are_ready_gated_versioned_reproducible_and_retrieved_once_by_the_owner()
-> Result<()> {
    let f = F::new().await?;
    let sep = day("2026-09-01");
    f.clock("2026-09-10").await?;
    f.fund(100000).await?;
    let spend = f
        .post("2026-09-12", "SIMULATED_CARD_PURCHASE", -2500, None)
        .await?;
    f.post("2026-08-31", "SYNTHETIC_FUNDING", 700, None).await?;
    f.post("2026-10-01", "SYNTHETIC_FUNDING", 900, None).await?;
    // Open period: nothing is published; the refusal is evidence.
    f.clock("2026-09-30").await?;
    let early =
        f.r.generate(&f.c, &f.alice, f.account, sep)
            .await
            .unwrap_err();
    let readiness = &early.downcast_ref::<NotReady>().expect("not ready").0;
    assert_eq!(readiness["reasons"], json!(["PERIOD_OPEN"]));
    assert_eq!(
        f.count("SELECT count(*) FROM documents.statement_versions")
            .await?,
        0
    );
    assert_eq!(f.count("SELECT count(*) FROM documents.document_access WHERE action='GENERATE' AND outcome='NOT_READY'").await?, 1);
    assert!(
        f.r.generate(&f.c, &f.alice, f.account, day("2026-09-02"))
            .await
            .is_err()
    );
    assert!(f.r.generate(&f.c, &f.bob, f.account, sep).await.is_err());
    f.clock("2026-10-01").await?;
    let v1 = f.r.generate(&f.c, &f.alice, f.account, sep).await?;
    assert_eq!(
        (
            v1["created"].as_bool(),
            v1["version"].as_i64(),
            v1["reason"].as_str()
        ),
        (Some(true), Some(1), Some("INITIAL"))
    );
    let again = f.r.generate(&f.c, &f.support, f.account, sep).await?;
    assert_eq!(
        (again["id"].clone(), again["created"].as_bool()),
        (v1["id"].clone(), Some(false))
    );
    let v1_id = id(&v1["id"]);
    // Exact totals: opening + signed movements = closing, independently of the Core balance field.
    let d = f.download(&f.alice, v1_id).await?;
    let doc = &d["document"];
    assert_eq!(doc["openingMinor"], "700");
    assert_eq!(doc["closingMinor"], "98200");
    assert_eq!(doc["totals"]["creditsMinor"], "100000");
    assert_eq!(doc["totals"]["debitsMinor"], "2500");
    assert_eq!(doc["totals"]["netMinor"], "97500");
    assert_eq!(doc["items"].as_array().unwrap().len(), 2);
    assert_eq!(doc["label"], "SYNTHETIC_TST_NOT_A_TAX_FORM");
    assert_eq!(doc["retention"]["retainUntil"], "2033-10-01");
    assert!(d["supersededBy"].is_null());
    // Cross-customer, reused, foreign-actor and expired references are denied and recorded.
    assert_eq!(
        denied(f.r.link(&f.c, &f.bob, v1_id).await.unwrap_err()),
        "NOT_AUTHORIZED"
    );
    let link = f.r.link(&f.c, &f.alice, v1_id).await?;
    let token = link["token"].as_str().unwrap();
    assert_eq!(
        denied(f.r.redeem(&f.c, &f.bob, token).await.unwrap_err()),
        "ACTOR_MISMATCH"
    );
    assert_eq!(
        denied(f.r.redeem(&f.c, &f.support, token).await.unwrap_err()),
        "ACTOR_MISMATCH"
    );
    f.r.redeem(&f.c, &f.alice, token).await?;
    assert_eq!(
        denied(f.r.redeem(&f.c, &f.alice, token).await.unwrap_err()),
        "ALREADY_USED"
    );
    assert_eq!(
        denied(
            f.r.redeem(&f.c, &f.alice, &"0".repeat(64))
                .await
                .unwrap_err()
        ),
        "UNKNOWN_REFERENCE"
    );
    let stale = f.r.link(&f.c, &f.alice, v1_id).await?;
    sqlx::query("ALTER TABLE documents.statement_links DISABLE TRIGGER statement_link_guard")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE documents.statement_links SET created_at=created_at-interval '10 minutes',expires_at=expires_at-interval '10 minutes' WHERE id=$1").bind(id(&stale["linkId"])).execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE documents.statement_links ENABLE TRIGGER statement_link_guard")
        .execute(&f.db.migrator)
        .await?;
    assert_eq!(
        denied(
            f.r.redeem(&f.c, &f.alice, stale["token"].as_str().unwrap())
                .await
                .unwrap_err()
        ),
        "EXPIRED"
    );
    assert!(sqlx::query("UPDATE documents.statement_links SET expires_at=expires_at+interval '1 hour' WHERE id=$1").bind(id(&stale["linkId"])).execute(&f.db.migrator).await.is_err());
    assert!(
        sqlx::query("DELETE FROM documents.statement_versions")
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE documents.statement_objects SET digest=digest")
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    assert!(f.r.link(&f.c, &f.nobody, v1_id).await.is_err());
    // Workforce support retrieval is a separate audited grant bound to the staff actor.
    assert_eq!(
        f.download(&f.support, v1_id).await?["contentDigest"],
        d["contentDigest"]
    );
    assert_eq!(
        f.count("SELECT count(*) FROM documents.document_access WHERE outcome='DENIED'")
            .await?,
        6
    );
    // A linked correction booked later supersedes with identical closed-period totals.
    f.clock("2026-10-05").await?;
    let correction = f
        .post("2026-10-05", "CORRECTION", 2500, Some(spend))
        .await?;
    let v2 = f.r.generate(&f.c, &f.alice, f.account, sep).await?;
    assert_eq!(
        (
            v2["version"].as_i64(),
            v2["reason"].as_str(),
            v2["supersedes"].clone()
        ),
        (Some(2), Some("LINKED_CORRECTION"), v1["id"].clone())
    );
    let d2 = f.download(&f.alice, id(&v2["id"])).await?;
    for field in ["openingMinor", "closingMinor", "items"] {
        assert_eq!(d2["document"][field], doc[field]);
    }
    let mut totals = d2["document"]["totals"].clone();
    totals["linkedCorrectionCount"] = json!(0);
    assert_eq!(totals, doc["totals"]);
    assert_eq!(
        d2["document"]["linkedCorrections"][0]["batchId"],
        correction.to_string()
    );
    let old = f.download(&f.alice, v1_id).await?;
    assert_eq!(
        (old["contentDigest"].clone(), old["supersededBy"].clone()),
        (d["contentDigest"].clone(), v2["id"].clone())
    );
    assert_eq!(
        f.r.generate(&f.c, &f.alice, f.account, sep).await?["created"],
        false
    );
    // The next period carries the correction as a movement and opens at the prior closing.
    f.clock("2026-11-01").await?;
    let oct =
        f.r.generate(&f.c, &f.alice, f.account, day("2026-10-01"))
            .await?;
    let od = f.download(&f.alice, id(&oct["id"])).await?;
    assert_eq!(od["document"]["openingMinor"], "98200");
    assert_eq!(od["document"]["totals"]["correctionsMinor"], "2500");
    assert_eq!(od["document"]["closingMinor"], "101600");
    // An owner-path backdated fact is never silently absorbed: it becomes a visible superseding version.
    f.post("2026-09-20", "SYNTHETIC_FUNDING", 100, None).await?;
    let v3 = f.r.generate(&f.c, &f.officer, f.account, sep).await?;
    assert_eq!(
        (v3["version"].as_i64(), v3["reason"].as_str()),
        (Some(3), Some("SOURCE_CHANGED"))
    );
    let list = f.r.customer_documents(&f.c, &f.alice).await?;
    let periods = list["accounts"][0]["periods"].as_array().unwrap();
    assert_eq!(periods[0]["periodStart"], "2026-11-01");
    assert_eq!(periods[0]["readiness"]["reasons"], json!(["PERIOD_OPEN"]));
    let september = periods
        .iter()
        .find(|p| p["periodStart"] == "2026-09-01")
        .unwrap();
    assert_eq!(september["versions"].as_array().unwrap().len(), 3);
    assert!(september["versions"][0].get("legalHold").is_none());
    assert_eq!(
        f.r.customer_documents(&f.c, &f.bob).await?["accounts"],
        json!([])
    );
    let staff = f.r.staff_documents(&f.c, &f.support, f.account).await?;
    assert!(
        staff["access"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["reason"] == "ACTOR_MISMATCH")
    );
    assert!(
        f.r.staff_documents(&f.c, &f.nobody, f.account)
            .await
            .is_err()
    );
    assert!(
        f.r.staff_documents(&f.c, &f.alice, f.account)
            .await
            .is_err()
    );
    f.close().await
}

#[tokio::test]
async fn retention_legal_holds_disposal_and_data_requests_produce_traceable_outcomes() -> Result<()>
{
    let f = F::new().await?;
    f.clock("2026-09-10").await?;
    f.fund(5000).await?;
    f.clock("2026-11-01").await?;
    let sep =
        f.r.generate(&f.c, &f.alice, f.account, day("2026-09-01"))
            .await?;
    let oct =
        f.r.generate(&f.c, &f.alice, f.account, day("2026-10-01"))
            .await?;
    let sep_id = id(&sep["id"]);
    // Retention has not ended: disposal is refused, recorded and leaves the object intact.
    assert_eq!(
        restricted(
            f.r.dispose(&f.c, &f.officer, sep_id, "Scheduled disposal")
                .await
                .unwrap_err()
        ),
        "RESTRICTED_RETENTION"
    );
    assert!(
        f.r.dispose(&f.c, &f.support, sep_id, "Scheduled disposal")
            .await
            .is_err()
    );
    let key = Uuid::new_v4();
    let hold = HoldInput {
        scope: "ACCOUNT".into(),
        account_id: Some(f.account),
        person_id: None,
        reason: "Synthetic litigation hold".into(),
        reference: "CASE-25-1".into(),
    };
    let placed = f.r.place_hold(&f.c, &f.officer, key, &hold).await?;
    assert_eq!(
        f.r.place_hold(&f.c, &f.officer, key, &hold).await?["id"],
        placed["id"]
    );
    let other = HoldInput {
        reference: "OTHER".into(),
        ..hold.clone()
    };
    assert!(f.r.place_hold(&f.c, &f.officer, key, &other).await.is_err());
    assert!(
        f.r.place_hold(&f.c, &f.approver, Uuid::new_v4(), &hold)
            .await
            .is_err()
    );
    let hold_id = id(&placed["id"]);
    f.clock("2034-01-02").await?;
    assert_eq!(
        restricted(
            f.r.dispose(&f.c, &f.officer, sep_id, "Retention ended")
                .await
                .unwrap_err()
        ),
        "RESTRICTED_LEGAL_HOLD"
    );
    assert!(sqlx::query("INSERT INTO documents.statement_disposals VALUES($1,$2,'{}','bypass',NULL,'2034-01-02','{}',repeat('0',64),now())").bind(sep_id).bind(f.alice_person).execute(&f.db.migrator).await.is_err());
    assert!(
        sqlx::query("DELETE FROM documents.statement_objects WHERE version_id=$1")
            .bind(sep_id)
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    // Release is independent of placement, even when the placer also holds the approver role.
    let officer_person = f.c.person(&f.officer).await?.0;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'records_approver',now()+interval '1 day')").bind(Uuid::new_v4()).bind(officer_person).execute(&f.db.migrator).await?;
    assert!(
        f.r.release_hold(&f.c, &f.officer, hold_id, "Matter closed")
            .await
            .is_err()
    );
    assert_eq!(
        f.r.release_hold(&f.c, &f.approver, hold_id, "Matter closed")
            .await?["released"],
        true
    );
    assert_eq!(
        f.r.release_hold(&f.c, &f.approver, hold_id, "Matter closed")
            .await?["released"],
        false
    );
    // Object/metadata partial failure: a failed object removal leaves no disposal evidence behind.
    f.fail_objects(true).await?;
    assert!(
        f.r.dispose(&f.c, &f.officer, sep_id, "Retention ended")
            .await
            .is_err()
    );
    assert_eq!(
        f.count("SELECT count(*) FROM documents.statement_disposals")
            .await?,
        0
    );
    assert_eq!(
        f.count("SELECT count(*) FROM documents.statement_objects")
            .await?,
        2
    );
    f.restore_objects().await?;
    let disposed =
        f.r.dispose(&f.c, &f.officer, sep_id, "Retention ended")
            .await?;
    assert_eq!(disposed["status"], "DISPOSED");
    assert_eq!(
        f.r.dispose(&f.c, &f.officer, sep_id, "Retention ended")
            .await?["status"],
        "ALREADY_DISPOSED"
    );
    assert_eq!(
        f.count("SELECT count(*) FROM documents.statement_versions")
            .await?,
        2
    );
    let evidence: Value = sqlx::query_scalar(
        "SELECT evidence FROM documents.statement_disposals WHERE version_id=$1",
    )
    .bind(sep_id)
    .fetch_one(&f.db.migrator)
    .await?;
    assert_eq!(
        (
            evidence["contentDigest"].clone(),
            evidence["holdCheck"].as_str()
        ),
        (sep["contentDigest"].clone(), Some("NO_ACTIVE_HOLD"))
    );
    assert_eq!(
        denied(f.r.link(&f.c, &f.alice, sep_id).await.unwrap_err()),
        "DISPOSED"
    );
    assert!(
        f.r.generate(&f.c, &f.alice, f.account, day("2026-09-01"))
            .await
            .is_err()
    );
    let view = f.r.staff_documents(&f.c, &f.officer, f.account).await?;
    assert!(
        view["legalHolds"][0]["active"] == false && view["legalHolds"][0]["releasedBy"].is_string()
    );
    // AT-16: a verified deletion request conflicting with a person hold is restricted, item by item.
    let person_hold = HoldInput {
        scope: "PERSON".into(),
        account_id: None,
        person_id: Some(f.alice_person),
        reason: "Synthetic regulator inquiry".into(),
        reference: "REG-25-2".into(),
    };
    let ph =
        f.r.place_hold(&f.c, &f.officer, Uuid::new_v4(), &person_hold)
            .await?;
    let rk = Uuid::new_v4();
    let deletion = DataRequestInput {
        kind: "DELETION".into(),
        detail: "Please erase my statements".into(),
    };
    let access_input = DataRequestInput {
        kind: "ACCESS".into(),
        detail: String::new(),
    };
    let req = f.r.submit_request(&f.c, &f.alice, rk, &deletion).await?;
    assert_eq!(
        f.r.submit_request(&f.c, &f.alice, rk, &deletion).await?["id"],
        req["id"]
    );
    assert!(
        f.r.submit_request(&f.c, &f.alice, rk, &access_input)
            .await
            .is_err()
    );
    assert!(
        f.r.submit_request(&f.c, &f.officer, Uuid::new_v4(), &deletion)
            .await
            .is_err()
    );
    assert!(
        f.r.process_request(&f.c, &f.support, id(&req["id"]))
            .await
            .is_err()
    );
    let done =
        f.r.process_request(&f.c, &f.officer, id(&req["id"]))
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
            "ALREADY_DISPOSED",
            "RESTRICTED_LEGAL_HOLD",
            "RESTRICTED_RETENTION"
        ]
    );
    assert_eq!(
        f.r.process_request(&f.c, &f.officer, id(&req["id"]))
            .await?["items"],
        done["items"]
    );
    assert_eq!(
        f.count("SELECT count(*) FROM documents.statement_objects")
            .await?,
        1
    );
    // Access produces a retained export manifest under its own policy; correction is a restricted outcome.
    let access =
        f.r.submit_request(&f.c, &f.alice, Uuid::new_v4(), &access_input)
            .await?;
    let exported =
        f.r.process_request(&f.c, &f.officer, id(&access["id"]))
            .await?;
    assert_eq!(exported["outcome"], "ALLOWED");
    assert_eq!(exported["export"]["statements"][0]["versionId"], oct["id"]);
    let fix = DataRequestInput {
        kind: "CORRECTION".into(),
        detail: "Wrong amount".into(),
    };
    let correction =
        f.r.submit_request(&f.c, &f.alice, Uuid::new_v4(), &fix)
            .await?;
    assert_eq!(
        f.r.process_request(&f.c, &f.officer, id(&correction["id"]))
            .await?["outcome"],
        "RESTRICTED"
    );
    assert_eq!(
        restricted(
            f.r.dispose_export(&f.c, &f.officer, id(&access["id"]), "Export expired")
                .await
                .unwrap_err()
        ),
        "RESTRICTED_RETENTION"
    );
    f.clock("2034-02-05").await?;
    assert_eq!(
        restricted(
            f.r.dispose_export(&f.c, &f.officer, id(&access["id"]), "Export expired")
                .await
                .unwrap_err()
        ),
        "RESTRICTED_LEGAL_HOLD"
    );
    f.r.release_hold(&f.c, &f.approver, id(&ph["id"]), "Inquiry closed")
        .await?;
    assert_eq!(
        f.r.dispose_export(&f.c, &f.officer, id(&access["id"]), "Export expired")
            .await?["status"],
        "DISPOSED"
    );
    let mine = f.r.customer_documents(&f.c, &f.alice).await?;
    let mine_access = mine["dataRequests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == access["id"])
        .unwrap();
    assert!(mine_access["export"].is_null() && mine_access["exportDisposed"] == true);
    assert!(mine["dataRequests"][0].get("personId").is_none());
    assert_eq!(
        f.r.data_requests(&f.c, &f.officer)
            .await?
            .as_array()
            .unwrap()
            .len(),
        3
    );
    f.close().await
}

#[tokio::test]
async fn failed_object_writes_publish_nothing_and_the_worker_cycle_retries_once() -> Result<()> {
    let f = F::new().await?;
    f.clock("2026-09-03").await?;
    f.fund(4200).await?;
    f.clock("2026-10-01").await?;
    f.fail_objects(false).await?;
    assert!(
        f.r.generate(&f.c, &f.alice, f.account, day("2026-09-01"))
            .await
            .is_err()
    );
    assert_eq!(f.r.generate_due(10).await?, 0);
    assert_eq!(
        f.count("SELECT count(*) FROM documents.statement_versions")
            .await?,
        0
    );
    f.restore_objects().await?;
    // Retried on a later pass once the object store succeeds; replays create nothing new.
    assert_eq!(f.r.generate_due(10).await?, 1);
    assert_eq!(f.r.generate_due(10).await?, 0);
    let list = f.r.customer_documents(&f.c, &f.alice).await?;
    let september = list["accounts"][0]["periods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["periodStart"] == "2026-09-01")
        .unwrap()
        .clone();
    assert_eq!(september["versions"][0]["closingMinor"], "4200");
    assert_eq!(september["readiness"]["periodic"], json!([]));
    assert_eq!(
        f.count("SELECT count(*) FROM documents.document_access WHERE action='GENERATE' AND outcome='NOT_READY' AND reason LIKE '%object store unavailable%'")
            .await?,
        1
    );
    f.close().await
}

#[tokio::test]
async fn concurrent_generation_converges_on_one_version() -> Result<()> {
    let f = F::new().await?;
    f.clock("2026-09-04").await?;
    f.fund(1200).await?;
    f.clock("2026-10-01").await?;
    let mut tasks = Vec::new();
    for n in 0..6 {
        let (r, c) = (f.r.clone(), f.c.clone());
        let who = if n % 2 == 0 {
            f.alice.clone()
        } else {
            f.support.clone()
        };
        let account = f.account;
        tasks.push(tokio::spawn(async move {
            r.generate(&c, &who, account, day("2026-09-01")).await
        }));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut created = 0;
    for task in tasks {
        let v = task.await??;
        ids.insert(v["id"].to_string());
        created += usize::from(v["created"] == true);
    }
    assert_eq!((ids.len(), created), (1, 1));
    assert_eq!(
        f.count("SELECT count(*) FROM documents.statement_versions")
            .await?,
        1
    );
    f.close().await
}
