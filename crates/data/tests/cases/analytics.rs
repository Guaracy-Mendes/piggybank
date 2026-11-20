//! WP-37 governed warehouse (one local adapter): lineage, purpose checks, backfill/replay deduplication, corrections,
//! schema upgrade, master-reference conflicts and the guarantee that analytical principals cannot touch Core.
use bank_controls::{
    entitlements::Entitlements, evidence::EvidenceService, governance::ChangeKind,
};
use bank_core::{
    Core,
    accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest},
};
use bank_data::{
    Data, MasterChange,
    analytics::{Backfill, Denied, SchemaVersion, SnapshotRequest, Warehouse},
};
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, Currency, EventEnvelope, Money, TrustContext, fingerprint},
    testing::TestDatabase,
};
use chrono::{Duration, NaiveDate, Utc};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct World {
    pub(crate) db: TestDatabase,
    pub(crate) core: Core,
    pub(crate) data: Data,
    pub(crate) controls: Entitlements,
    pub(crate) wh: Warehouse,
    pub(crate) core_pool: PgPool,
    pub(crate) data_pool: PgPool,
    pub(crate) controls_pool: PgPool,
    pub(crate) steward: ActorContext,
    pub(crate) approver: ActorContext,
    pub(crate) analyst: ActorContext,
    pub(crate) finance: ActorContext,
    pub(crate) alice: Uuid,
    pub(crate) bob: Uuid,
    pub(crate) accounts: Vec<AccountId>,
    pub(crate) transfer_batch: Uuid,
    pub(crate) from: NaiveDate,
    pub(crate) to: NaiveDate,
}
pub(crate) fn actor(subject: &str, trust: TrustContext) -> ActorContext {
    ActorContext {
        issuer: "urn:analytics-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "test".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
struct Gates;
impl OpeningPolicy for Gates {
    fn validate<'a>(
        &'a self,
        r: &'a OpeningRequest,
    ) -> BoxFuture<'a, anyhow::Result<ActivationEvidence>> {
        Box::pin(async move {
            Ok(ActivationEvidence {
                application_id: r.application_id,
                party_id: r.party_id,
                product_version_id: r.product_version_id,
                acceptance_id: r.acceptance_id,
                controls_revision: "synthetic-analytics-fixture".into(),
                expires_at: Utc::now() + Duration::minutes(5),
            })
        })
    }
}
pub(crate) async fn world() -> anyhow::Result<World> {
    let db = TestDatabase::new().await?;
    let core_pool = db.module(Module::Core).await?;
    let data_pool = db.module(Module::Data).await?;
    let controls_pool = db.module(Module::Controls).await?;
    let core = Core::new(core_pool.clone());
    let data = Data::new(data_pool.clone(), Arc::new(core.clone()));
    let controls = Entitlements::new(controls_pool.clone());
    let wh = Warehouse::new(data_pool.clone());
    let (alice_actor, bob_actor) = (
        actor("alice", TrustContext::Customer),
        actor("bob", TrustContext::Customer),
    );
    let alice = core
        .ensure_party(
            &alice_actor,
            Uuid::new_v4(),
            &fingerprint(&"synthetic-alice")?,
        )
        .await?
        .0;
    let bob = core
        .ensure_party(&bob_actor, Uuid::new_v4(), &fingerprint(&"synthetic-bob")?)
        .await?
        .0;
    controls
        .link_person(&alice_actor, bank_platform::domain::PartyId(alice))
        .await?;
    controls
        .link_person(&bob_actor, bank_platform::domain::PartyId(bob))
        .await?;
    let steward = actor("steward", TrustContext::Workforce);
    let approver = actor("approver", TrustContext::Workforce);
    let analyst = actor("analyst", TrustContext::Workforce);
    let finance = actor("finance", TrustContext::Workforce);
    for (who, roles) in [
        (&steward, vec!["data_steward", "case_reviewer"]),
        (&approver, vec!["data_approver"]),
        (&analyst, vec!["data_analyst"]),
        (&finance, vec!["finance_adjuster"]),
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
        if who.subject == "steward" {
            sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('data-operations',$1,'REVIEW')").bind(person).execute(&db.migrator).await?;
        }
    }
    let product = Uuid::new_v4();
    sqlx::query("INSERT INTO core.product_versions(id,code,version,effective_from,effective_until,document,approval_id,maker_person,checker_person) VALUES($1,'ANALYTICS_FIXTURE',1,current_date-1,current_date+30,'{\"label\":\"SYNTHETIC\",\"chartVersion\":\"synthetic-chart-v1\"}',gen_random_uuid(),gen_random_uuid(),gen_random_uuid())").bind(product).execute(&db.migrator).await?;
    let mut accounts = vec![];
    for party in [alice, bob] {
        let request = OpeningRequest {
            application_id: Uuid::new_v4(),
            party_id: bank_platform::domain::PartyId(party),
            product_version_id: product,
            acceptance_id: Uuid::new_v4(),
        };
        let account = core.stage_account(&request).await?;
        core.activate(account, &request, &Gates).await?;
        accounts.push(account);
    }
    let clearing = AccountId::new();
    sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'SETTLEMENT','ACTIVE',false)").bind(clearing.0).execute(&db.migrator).await?;
    let money = |n| Money::new(n, Currency::synthetic());
    core.fund_synthetic(Uuid::new_v4(), accounts[0], clearing, money(1000))
        .await?;
    let transfer_batch = core
        .transfer(Uuid::new_v4(), accounts[0], accounts[1], money(150), None)
        .await?;
    let today: NaiveDate = sqlx::query_scalar("SELECT (now() AT TIME ZONE 'UTC')::date")
        .fetch_one(&db.migrator)
        .await?;
    let w = World {
        db,
        core,
        data,
        controls,
        wh,
        core_pool,
        data_pool,
        controls_pool,
        steward,
        approver,
        analyst,
        finance,
        alice,
        bob,
        accounts,
        transfer_batch,
        from: today - Duration::days(1),
        to: today + Duration::days(1),
    };
    project(&w).await?;
    Ok(w)
}
/// Feed every Core event to the Data projection (idempotent).
pub(crate) async fn project(w: &World) -> anyhow::Result<()> {
    let documents: Vec<Value> = sqlx::query_scalar(
        "SELECT document FROM core.outbox ORDER BY aggregate_id,aggregate_version",
    )
    .fetch_all(&w.core_pool)
    .await?;
    for document in documents {
        let event: EventEnvelope = serde_json::from_value(document)?;
        assert!(w.data.consume(&event).await?);
    }
    Ok(())
}
fn request(w: &World, version: i32, purpose: &str) -> SnapshotRequest {
    SnapshotRequest {
        transform: "daily_ledger_movement".into(),
        version,
        purpose: purpose.into(),
        from: w.from,
        to: w.to,
    }
}
fn flags(snapshot: &Value) -> Vec<String> {
    snapshot["completeness"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap().to_owned())
        .collect()
}
fn party_rows(snapshot: &Value, party: Uuid) -> Vec<Value> {
    snapshot["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["dimensions"]["partyRef"] == party.to_string())
        .cloned()
        .collect()
}
fn denied(error: anyhow::Error) -> String {
    error
        .downcast_ref::<Denied>()
        .map(|d| d.0.clone())
        .unwrap_or_else(|| panic!("expected a recorded denial, got {error:#}"))
}
fn sql_code(error: &anyhow::Error) -> Option<String> {
    match error.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::Database(d)) => d.code().map(|c| c.into_owned()),
        _ => None,
    }
}
pub(crate) async fn close(w: World) -> anyhow::Result<()> {
    w.core_pool.close().await;
    w.data_pool.close().await;
    w.controls_pool.close().await;
    w.db.close().await
}

#[tokio::test]
async fn warehouse_snapshots_trace_to_source_schema_transform_and_authorized_purpose()
-> anyhow::Result<()> {
    let w = world().await?;
    let fr = request(&w, 1, "FINANCIAL_REPORTING");
    // Nothing ingested yet: missing sources, uningested projection rows and a Core control-total gap are all flagged.
    let empty = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    assert_eq!(empty["status"], "INCOMPLETE");
    let codes = flags(&empty);
    for code in ["MISSING_SOURCE", "STALE_SOURCE", "CONTROL_TOTAL_MISMATCH"] {
        assert!(codes.contains(&code.to_owned()), "{code} in {codes:?}");
    }
    assert_eq!(empty["control"]["reconciled"], false);
    // Incremental ingestion; a second pass finds nothing new and records no run.
    let first = w.wh.ingest_due(0.0).await?;
    let by = |r: &Value, d: &str| {
        r.as_array()
            .unwrap()
            .iter()
            .find(|x| x["dataset"] == d)
            .unwrap()
            .clone()
    };
    assert_eq!(by(&first, "ledger_entries")["inserted"], 4);
    assert_eq!(by(&first, "account_states")["inserted"], 2);
    let again = w.wh.ingest_due(0.0).await?;
    assert_eq!(by(&again, "ledger_entries")["inserted"], 0);
    assert!(by(&again, "ledger_entries")["runId"].is_null());
    // Complete snapshot: Core = input = output, lineage to schema/run/transform/purpose/adapter.
    let snap = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    assert_eq!(snap["status"], "COMPLETE", "{snap:#}");
    assert_eq!(snap["control"]["reconciled"], true);
    assert_eq!(snap["control"]["core"]["entries"], 4);
    assert_eq!(snap["control"]["output"]["creditMinor"], "1150");
    assert_eq!(snap["purpose"], "FINANCIAL_REPORTING");
    assert_eq!(snap["adapter"], "LOCAL_WAREHOUSE");
    assert_eq!(snap["supersedes"], empty["id"]);
    assert_eq!(snap["explanation"]["newFacts"], 4);
    let ledger = snap["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["dataset"] == "ledger_entries")
        .unwrap();
    assert_eq!(ledger["schemaVersions"], json!([1]));
    assert_eq!(ledger["runs"].as_array().unwrap().len(), 1);
    assert_eq!(snap["transformDigest"].as_str().unwrap().len(), 64);
    let alice_rows = party_rows(&snap, w.alice);
    assert_eq!(alice_rows.len(), 2, "funding and transfer rows for alice");
    assert!(
        snap["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["dimensions"]["partyRef"] == "INTERNAL:SETTLEMENT")
    );
    let id: Uuid = serde_json::from_value(snap["id"].clone())?;
    // Reproducible from immutable facts; one row traces to its source projection rows and ingestion run.
    let reproduced = w.wh.reproduce(&w.controls, &w.steward, id).await?;
    assert_eq!(reproduced["reproduced"], true);
    let row = alice_rows[0]["rowNo"].as_i64().unwrap() as i32;
    let trace = w.wh.trace(&w.controls, &w.steward, id, row).await?;
    let facts = trace["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 1);
    assert!(
        facts[0]["sourceRef"]
            .as_str()
            .unwrap()
            .starts_with("transaction_views:")
    );
    assert_eq!(facts[0]["schemaVersion"], 1);
    assert_eq!(facts[0]["runId"], ledger["runs"][0]);
    // The snapshot is published once, without personal data.
    let event: Value = sqlx::query_scalar(
        "SELECT document FROM data.outbox WHERE aggregate_id=$1 AND document->>'eventType'='analytics.snapshot-published'",
    )
    .bind(id)
    .fetch_one(&w.db.migrator)
    .await?;
    assert_eq!(event["producer"], "data");
    assert_eq!(event["data"]["resultDigest"], snap["resultDigest"]);
    let contract: Value = serde_json::from_str(include_str!(
        "../../../../contracts/events/analytics/snapshot-published.v1.schema.json"
    ))?;
    let declared: Vec<&str> = contract["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for key in event["data"].as_object().unwrap().keys() {
        assert!(declared.contains(&key.as_str()), "{key} not in contract");
    }
    // Purpose denials are recorded: no dataset permits MARKETING; finance roles may not run operations analytics.
    assert_eq!(
        denied(
            w.wh.snapshot(&w.controls, &w.analyst, &request(&w, 1, "MARKETING"))
                .await
                .unwrap_err()
        ),
        "PURPOSE_NOT_PERMITTED_FOR_DATASET"
    );
    assert_eq!(
        denied(
            w.wh.snapshot(
                &w.controls,
                &w.finance,
                &request(&w, 1, "OPERATIONS_ANALYTICS")
            )
            .await
            .unwrap_err()
        ),
        "ROLE_NOT_PERMITTED_FOR_PURPOSE"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM analytics.access_denials")
            .fetch_one(&w.db.migrator)
            .await?,
        2
    );
    // Reading a snapshot re-checks its purpose: the analyst holds no financial-reporting role.
    let refused = w.wh.detail(&w.controls, &w.analyst, id).await.unwrap_err();
    assert_eq!(sql_code(&refused).as_deref(), Some("42501"));
    assert!(w.wh.detail(&w.controls, &w.finance, id).await.is_ok());
    // A WP-28 ANALYTICS restriction excludes alice from optional analytics but never from mandatory reporting.
    sqlx::query("INSERT INTO data.processing_restrictions(person_id,purpose,request_id) VALUES($1,'ANALYTICS',gen_random_uuid())")
        .bind(w.alice)
        .execute(&w.db.migrator)
        .await?;
    let ops =
        w.wh.snapshot(
            &w.controls,
            &w.analyst,
            &request(&w, 1, "OPERATIONS_ANALYTICS"),
        )
        .await?;
    assert!(party_rows(&ops, w.alice).is_empty());
    assert_eq!(ops["control"]["excluded"]["entries"], 2);
    assert_eq!(ops["control"]["excluded"]["parties"], 1);
    assert_eq!(ops["control"]["reconciled"], true);
    let mandatory = w.wh.snapshot(&w.controls, &w.finance, &fr).await?;
    assert_eq!(party_rows(&mandatory, w.alice).len(), 2);
    assert_eq!(mandatory["control"]["excluded"]["entries"], 0);
    // Non-staff contexts cannot use the warehouse at all.
    let customer = actor("alice", TrustContext::Customer);
    assert!(w.wh.view(&w.controls, &customer).await.is_err());
    let view = w.wh.view(&w.controls, &w.steward).await?;
    assert_eq!(view["adapters"][0]["status"], "LOCAL_TEMPLATE");
    assert_eq!(view["denials"].as_array().unwrap().len(), 2);
    // Registered v1 dataset schemas match the published contracts.
    for (dataset, contract) in [
        (
            "ledger_entries",
            include_str!("../../../../contracts/events/analytics/ledger_entries.v1.json"),
        ),
        (
            "account_states",
            include_str!("../../../../contracts/events/analytics/account_states.v1.json"),
        ),
    ] {
        let contract: Value = serde_json::from_str(contract)?;
        let (fields, key): (Value, Vec<String>) = sqlx::query_as("SELECT s.fields,d.economic_key FROM analytics.dataset_schemas s JOIN analytics.datasets d ON d.code=s.dataset WHERE s.dataset=$1 AND s.version=1")
            .bind(dataset)
            .fetch_one(&w.db.migrator)
            .await?;
        assert_eq!(fields, contract["fields"], "{dataset} fields");
        assert_eq!(
            json!(key),
            contract["economicKey"],
            "{dataset} economic key"
        );
    }
    close(w).await
}

#[tokio::test]
async fn backfill_correction_schema_upgrade_and_master_conflicts_preserve_financial_history()
-> anyhow::Result<()> {
    let w = world().await?;
    w.wh.ingest_due(0.0).await?;
    let fr = request(&w, 1, "FINANCIAL_REPORTING");
    let original = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    assert_eq!(original["status"], "COMPLETE");
    let original_id: Uuid = serde_json::from_value(original["id"].clone())?;
    // A Core correction reverses the transfer as a new linked batch; its projection is not yet ingested.
    let correction: Uuid = sqlx::query_scalar("SELECT batch_id FROM core.post(gen_random_uuid(),'CORRECTION',(SELECT jsonb_agg(jsonb_build_object('accountId',account_id,'delta',-delta)) FROM core.journal_entries WHERE batch_id=$1),NULL,$1)")
        .bind(w.transfer_batch)
        .fetch_one(&w.db.migrator)
        .await?;
    sqlx::query("INSERT INTO data.transaction_views(batch_id,account_id,operation_id,document,origin_event) SELECT j.id,e.account_id,j.operation_id,jsonb_build_object('batchId',j.id,'accountId',e.account_id,'amount',jsonb_build_object('minorUnits',e.delta::text,'currency',jsonb_build_object('code','TST','scale',2)),'businessDate',j.business_date,'postingKind',j.posting_kind,'originalBatchId',j.original_batch,'status','POSTED','source','CORE_JOURNAL'),gen_random_uuid() FROM core.journal_batches j JOIN core.journal_entries e ON e.batch_id=j.id WHERE j.id=$1")
        .bind(correction)
        .execute(&w.db.migrator)
        .await?;
    let stale = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    assert_eq!(stale["status"], "INCOMPLETE");
    assert!(flags(&stale).contains(&"STALE_SOURCE".to_owned()));
    assert_eq!(stale["completeness"][0]["uningestedRows"], 2);
    // Backfill re-reads the whole window: the two new entries are inserted once, earlier facts are duplicates.
    let window = Backfill {
        dataset: "ledger_entries".into(),
        from: Utc::now() - Duration::hours(1),
        to: Utc::now() + Duration::minutes(1),
    };
    assert!(
        w.wh.backfill(&w.controls, &w.analyst, &window)
            .await
            .is_err(),
        "backfill is a steward action"
    );
    let run = w.wh.backfill(&w.controls, &w.steward, &window).await?;
    assert_eq!(
        (
            run["mode"].as_str(),
            run["inserted"].as_i64(),
            run["duplicates"].as_i64()
        ),
        (Some("BACKFILL"), Some(2), Some(4))
    );
    let rerun = w.wh.backfill(&w.controls, &w.steward, &window).await?;
    assert_eq!(
        (rerun["inserted"].as_i64(), rerun["duplicates"].as_i64()),
        (Some(0), Some(6))
    );
    assert!(rerun["runId"].is_string(), "every backfill is evidenced");
    let corrected = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    assert_eq!(corrected["status"], "COMPLETE", "{corrected:#}");
    assert_eq!(corrected["supersedes"], stale["id"]);
    let corrections = corrected["explanation"]["corrections"].as_array().unwrap();
    assert_eq!(corrections.len(), 1);
    assert_eq!(
        corrections[0]["originalBatchId"],
        w.transfer_batch.to_string()
    );
    assert_eq!(corrections[0]["postingKind"], "CORRECTION");
    // The original snapshot and the original facts are untouched and still reproduce.
    let still = w.wh.detail(&w.controls, &w.steward, original_id).await?;
    assert_eq!(still["rows"], original["rows"]);
    assert_eq!(still["supersededBy"], stale["id"]);
    assert_eq!(
        w.wh.reproduce(&w.controls, &w.steward, original_id).await?["reproduced"],
        true
    );
    assert!(
        sqlx::query("UPDATE analytics.facts SET record='{}' WHERE dataset='ledger_entries'")
            .execute(&w.db.migrator)
            .await
            .is_err()
    );

    // Schema upgrade: incompatible proposals are refused; an optional additive field is accepted.
    let v1: Value = sqlx::query_scalar(
        "SELECT fields FROM analytics.dataset_schemas WHERE dataset='ledger_entries' AND version=1",
    )
    .fetch_one(&w.db.migrator)
    .await?;
    let with = |extra: Value| {
        let mut f = v1.as_array().unwrap().clone();
        f.push(extra);
        Value::Array(f)
    };
    let without_currency = Value::Array(
        v1.as_array()
            .unwrap()
            .iter()
            .filter(|f| f["name"] != "currency")
            .cloned()
            .collect(),
    );
    for (version, fields, why) in [
        (2, without_currency, "field currency removed"),
        (
            2,
            with(json!({"name":"productVersionId","type":"uuid","required":true})),
            "must be optional",
        ),
        (
            2,
            with(json!({"name":"merchantName","type":"text","required":false})),
            "not supplied by source",
        ),
        (
            2,
            with(json!({"name":"productVersionId","type":"text","required":false})),
            "source type",
        ),
        (
            3,
            with(json!({"name":"productVersionId","type":"uuid","required":false})),
            "version must be 2",
        ),
    ] {
        let error =
            w.wh.register_schema(
                &w.controls,
                &w.steward,
                &SchemaVersion {
                    dataset: "ledger_entries".into(),
                    version,
                    fields,
                },
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains(why), "{why}: {error:#}");
    }
    let upgrade = SchemaVersion {
        dataset: "ledger_entries".into(),
        version: 2,
        fields: with(json!({"name":"productVersionId","type":"uuid","required":false})),
    };
    assert!(
        w.wh.register_schema(&w.controls, &w.analyst, &upgrade)
            .await
            .is_err()
    );
    assert_eq!(
        w.wh.register_schema(&w.controls, &w.steward, &upgrade)
            .await?["compatibility"],
        "BACKWARD"
    );
    w.core
        .transfer(
            Uuid::new_v4(),
            w.accounts[0],
            w.accounts[1],
            Money::new(25, Currency::synthetic()),
            None,
        )
        .await?;
    project(&w).await?;
    w.wh.ingest_due(0.0).await?;
    let versions: Vec<(i32, i64)> = sqlx::query_as("SELECT schema_version,count(*) FROM analytics.facts WHERE dataset='ledger_entries' GROUP BY 1 ORDER BY 1")
        .fetch_all(&w.db.migrator)
        .await?;
    assert_eq!(
        versions,
        vec![(1, 6), (2, 2)],
        "older facts keep their schema version"
    );
    let v2 =
        w.wh.snapshot(
            &w.controls,
            &w.steward,
            &request(&w, 2, "FINANCIAL_REPORTING"),
        )
        .await?;
    assert_eq!(v2["status"], "COMPLETE", "{v2:#}");
    let ledger = v2["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["dataset"] == "ledger_entries")
        .unwrap()
        .clone();
    assert_eq!(ledger["schemaVersions"], json!([1, 2]));
    assert!(
        v2["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["dimensions"].get("productVersionId").is_some())
    );
    assert!(
        party_rows(&v2, w.alice)
            .iter()
            .all(|r| r["dimensions"]["productVersionId"] != "UNRESOLVED"),
        "v1 facts resolve product through the observed account state"
    );

    // Master conflict: flagged, never merged automatically; an approved merge changes later snapshots only.
    let path = std::env::temp_dir().join(format!(
        "piggybank_analytics_evidence_{}",
        Uuid::new_v4().simple()
    ));
    let evidence = EvidenceService::new(
        w.controls_pool.clone(),
        Arc::new(bank_adapters::object_store::FilesystemStore::new(&path)?),
    );
    let proof = evidence
        .put(
            &w.steward,
            "CASE",
            b"Synthetic evidence for analytical reference grouping; financial ownership unchanged."
                .to_vec(),
            w.alice,
        )
        .await?;
    let conflict = w
        .data
        .conflict(&w.steward, &w.controls, w.alice, w.bob, proof.id)
        .await?;
    let flagged = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    let master = flagged["completeness"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"] == "UNRESOLVED_MASTER_CONFLICT")
        .expect("conflict flagged")
        .clone();
    assert_eq!(master["handling"], "NO_AUTOMATIC_MERGE");
    assert_eq!(master["references"][0]["conflictId"], conflict.to_string());
    assert!(
        !party_rows(&flagged, w.alice).is_empty(),
        "alice stays separate until approval"
    );
    let flagged_id: Uuid = serde_json::from_value(flagged["id"].clone())?;
    let case: Uuid = sqlx::query_scalar("SELECT case_id FROM data.reference_conflicts WHERE id=$1")
        .bind(conflict)
        .fetch_one(&w.db.migrator)
        .await?;
    let change = MasterChange {
        case_id: case,
        reference_id: w.alice,
        target_canonical: w.bob,
        expected_revision: 1,
        evidence_ref: proof.id,
    };
    let proposal = w
        .controls
        .propose_change(
            &w.steward,
            ChangeKind::Master,
            w.alice,
            &change,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    w.controls.approve_change(&w.approver, proposal).await?;
    w.data
        .apply_master(&w.steward, &w.controls, proposal, &change, &evidence)
        .await?;
    let merged = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    assert_eq!(merged["status"], "COMPLETE", "{merged:#}");
    assert!(party_rows(&merged, w.alice).is_empty());
    assert!(!party_rows(&merged, w.bob).is_empty());
    assert_eq!(merged["explanation"]["masterChanges"], 1);
    // The pre-merge snapshot reproduces against its own master cut; Core ownership never moved.
    assert_eq!(
        w.wh.reproduce(&w.controls, &w.steward, flagged_id).await?["reproduced"],
        true
    );
    assert_eq!(
        w.core
            .authority(
                w.accounts[0],
                bank_controls::entitlements::Action::ReadAccount
            )
            .await?
            .owners,
        vec![bank_platform::domain::PartyId(w.alice)]
    );

    // Replay under a new batch identity is an economic duplicate; different content for the same identity is quarantined.
    let src: (Uuid, Uuid, Value) = sqlx::query_as("SELECT account_id,operation_id,document FROM data.transaction_views WHERE batch_id=$1 LIMIT 1")
        .bind(w.transfer_batch)
        .fetch_one(&w.db.migrator)
        .await?;
    let replayed = Uuid::new_v4();
    let mut altered = src.2.clone();
    altered["amount"]["minorUnits"] = json!("-999");
    for (batch, document) in [(replayed, src.2.clone()), (Uuid::new_v4(), altered)] {
        sqlx::query("INSERT INTO data.transaction_views(batch_id,account_id,operation_id,document,origin_event) VALUES($1,$2,$3,$4,gen_random_uuid())")
            .bind(batch).bind(src.0).bind(src.1).bind(document)
            .execute(&w.db.migrator).await?;
    }
    let replay = w.wh.backfill(&w.controls, &w.steward, &window).await?;
    assert_eq!(
        (replay["inserted"].as_i64(), replay["conflicts"].as_i64()),
        (Some(0), Some(1)),
        "{replay:#}"
    );
    let quarantined = w.wh.snapshot(&w.controls, &w.steward, &fr).await?;
    assert!(flags(&quarantined).contains(&"ECONOMIC_CONFLICT".to_owned()));
    assert_eq!(
        quarantined["control"]["reconciled"], true,
        "conflicting content never reaches the report"
    );
    assert_eq!(
        quarantined["control"]["output"],
        merged["control"]["output"]
    );
    std::fs::remove_dir_all(path)?;
    close(w).await
}

#[tokio::test]
async fn analytical_principals_cannot_post_or_mutate_core_balances() -> anyhow::Result<()> {
    let w = world().await?;
    let m = &w.db.migrator;
    // The Data role (the only role the warehouse uses) has no Core schema usage, table or function privilege.
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT has_schema_privilege('bank_data','core','USAGE')")
            .fetch_one(m)
            .await?
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core' AND c.relkind IN ('r','p','v','m') AND has_table_privilege('bank_data',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE')").fetch_one(m).await?, 0);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='core' AND has_function_privilege('bank_data',p.oid,'EXECUTE')").fetch_one(m).await?, 0);
    // Nor any direct analytics table access: the definer API is the only path.
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='analytics' AND c.relkind='r' AND has_table_privilege('bank_data',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE')").fetch_one(m).await?, 0);
    // Every analytics definer body: no Core DML, and only non-volatile Core helpers.
    let bodies: Vec<(String, String)> = sqlx::query_as("SELECT p.proname::text,p.prosrc FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='analytics'").fetch_all(m).await?;
    assert!(bodies.len() >= 15);
    for (name, body) in &bodies {
        let lower = body.to_lowercase();
        for verb in [
            "insert into core.",
            "update core.",
            "delete from core.",
            "truncate core.",
        ] {
            assert!(!lower.contains(verb), "{name} contains {verb}");
        }
        let mut rest = lower.as_str();
        while let Some(at) = rest.find("core.") {
            let tail = &rest[at + 5..];
            let ident: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if tail[ident.len()..].trim_start().starts_with('(') {
                let volatile: Option<bool> = sqlx::query_scalar("SELECT bool_or(p.provolatile='v') FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='core' AND p.proname=$1")
                    .bind(&ident).fetch_one(m).await?;
                assert!(
                    !volatile.unwrap_or(false),
                    "{name} calls volatile core.{ident}"
                );
            }
            rest = &tail[ident.len()..];
        }
    }
    // Attempts through the Data pool fail outright, and every warehouse operation leaves balances unchanged.
    let before: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT id,booked FROM core.ledger_accounts ORDER BY id")
            .fetch_all(m)
            .await?;
    assert!(sqlx::query("SELECT * FROM core.post(gen_random_uuid(),'ANALYTICS',jsonb_build_array(jsonb_build_object('accountId',$1::uuid,'delta',1),jsonb_build_object('accountId',$2::uuid,'delta',-1)))")
        .bind(w.accounts[0].0).bind(w.accounts[1].0).execute(&w.data_pool).await.is_err());
    assert!(
        sqlx::query("UPDATE core.ledger_accounts SET booked=booked+1")
            .execute(&w.data_pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO analytics.facts(dataset,schema_version,economic_key,economic_digest,account_id,record,source_ref,source_recorded_at,run_id) VALUES('ledger_entries',1,'x','x',gen_random_uuid(),'{}','x',now(),gen_random_uuid())").execute(&w.data_pool).await.is_err());
    w.wh.ingest_due(0.0).await?;
    let snap =
        w.wh.snapshot(
            &w.controls,
            &w.steward,
            &request(&w, 1, "FINANCIAL_REPORTING"),
        )
        .await?;
    let id: Uuid = serde_json::from_value(snap["id"].clone())?;
    w.wh.reproduce(&w.controls, &w.steward, id).await?;
    w.wh.backfill(
        &w.controls,
        &w.steward,
        &Backfill {
            dataset: "account_states".into(),
            from: Utc::now() - Duration::hours(1),
            to: Utc::now(),
        },
    )
    .await?;
    let after: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT id,booked FROM core.ledger_accounts ORDER BY id")
            .fetch_all(m)
            .await?;
    assert_eq!(before, after);
    assert_eq!(
        w.core.balance(w.accounts[0]).await?.booked.minor_units(),
        850
    );
    close(w).await
}
