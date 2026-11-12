use bank_controls::{
    entitlements::Entitlements, evidence::EvidenceService, governance::ChangeKind,
};
use bank_core::{
    Core,
    accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest},
};
use bank_data::{Data, MasterChange};
use bank_platform::{
    database::Module,
    domain::{
        AccountId, ActorContext, Currency, EventEnvelope, EventId, Money, TrustContext, fingerprint,
    },
    testing::TestDatabase,
};
use chrono::{Duration, Utc};
use futures_util::future::BoxFuture;
use std::sync::Arc;
use uuid::Uuid;
#[tokio::test]
async fn projections_replay_and_governed_aliases_cannot_change_financial_ownership()
-> anyhow::Result<()> {
    let db = TestDatabase::new().await?;
    let core_pool = db.module(Module::Core).await?;
    let pool = db.module(Module::Data).await?;
    let cp = db.module(Module::Controls).await?;
    let core = Core::new(core_pool.clone());
    let data = Data::new(pool.clone(), Arc::new(core.clone()));
    let controls = Entitlements::new(cp.clone());
    let actor = |subject: &str, trust| ActorContext {
        issuer: "urn:data-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "test".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let alice = actor("alice", TrustContext::Customer);
    let bob = actor("bob", TrustContext::Customer);
    let maker = actor("steward", TrustContext::Workforce);
    let checker = actor("approver", TrustContext::Workforce);
    let alice_party = core
        .ensure_party(&alice, Uuid::new_v4(), &fingerprint(&"synthetic-alice")?)
        .await?;
    let bob_party = core
        .ensure_party(&bob, Uuid::new_v4(), &fingerprint(&"synthetic-bob")?)
        .await?;
    controls.link_person(&alice, alice_party).await?;
    controls.link_person(&bob, bob_party).await?;
    for (who, roles) in [
        (
            &maker,
            vec!["data_steward", "case_reviewer", "operations_replay"],
        ),
        (&checker, vec!["data_approver"]),
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
        if who.subject == maker.subject {
            sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('data-operations',$1,'REVIEW')").bind(person).execute(&db.migrator).await?;
        }
    }
    let product = Uuid::new_v4();
    sqlx::query("INSERT INTO core.product_versions(id,code,version,effective_from,effective_until,document,approval_id,maker_person,checker_person) VALUES($1,'DATA_FIXTURE',1,current_date-1,current_date+30,'{\"label\":\"SYNTHETIC\",\"chartVersion\":\"synthetic-chart-v1\"}',gen_random_uuid(),gen_random_uuid(),gen_random_uuid())").bind(product).execute(&db.migrator).await?;
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
                    controls_revision: "synthetic-port-fixture".into(),
                    expires_at: Utc::now() + Duration::minutes(5),
                })
            })
        }
    }
    let mut accounts = vec![];
    for person in [alice_party, bob_party] {
        let request = OpeningRequest {
            application_id: Uuid::new_v4(),
            party_id: person,
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
    let hold = Uuid::new_v4();
    core.reserve(hold, accounts[0], money(200)).await?;
    core.transfer(Uuid::new_v4(), accounts[0], accounts[1], money(150), None)
        .await?;
    core.release(hold).await?;
    let documents: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT document FROM core.outbox ORDER BY aggregate_id,aggregate_version",
    )
    .fetch_all(&core_pool)
    .await?;
    let events: Vec<EventEnvelope> = documents
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()?;
    let gap = events
        .iter()
        .find(|e| e.aggregate_id == hold && e.aggregate_version == 2)
        .unwrap();
    assert!(!data.consume(gap).await?);
    for event in &events {
        if event.event_id != gap.event_id {
            assert!(data.consume(event).await?);
            assert!(data.consume(event).await?);
        }
    }
    assert!(
        data.replay(&alice, &controls, gap.event_id.0)
            .await
            .is_err()
    );
    assert!(data.replay(&maker, &controls, gap.event_id.0).await?);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM data.hold_views WHERE operation_id=$1")
            .bind(hold)
            .fetch_one(&pool)
            .await?,
        "RELEASED"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM data.transaction_views")
            .fetch_one(&pool)
            .await?,
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM data.account_views WHERE state='ACTIVE'"
        )
        .fetch_one(&pool)
        .await?,
        2
    );
    let mut conflict = gap.clone();
    conflict.data["submitted"] = serde_json::json!(true);
    assert!(!data.consume(&conflict).await?);
    let mut unsupported = gap.clone();
    unsupported.event_id = EventId::new();
    unsupported.schema_version = 2;
    assert!(!data.consume(&unsupported).await?);
    let path = std::env::temp_dir().join(format!(
        "piggybank_master_evidence_{}",
        Uuid::new_v4().simple()
    ));
    let evidence = EvidenceService::new(
        cp.clone(),
        Arc::new(bank_adapters::object_store::FilesystemStore::new(&path)?),
    );
    let proof=evidence.put(&maker,"CASE",b"Synthetic evidence for enterprise reference grouping; financial ownership remains unchanged.".to_vec(),alice_party.0).await?;
    let conflict = data
        .conflict(&maker, &controls, alice_party.0, bob_party.0, proof.id)
        .await?;
    let case: Uuid = sqlx::query_scalar("SELECT case_id FROM data.reference_conflicts WHERE id=$1")
        .bind(conflict)
        .fetch_one(&pool)
        .await?;
    let change = MasterChange {
        case_id: case,
        reference_id: alice_party.0,
        target_canonical: bob_party.0,
        expected_revision: 1,
        evidence_ref: proof.id,
    };
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Master,
            alice_party.0,
            &change,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    assert!(
        data.apply_master(&maker, &controls, proposal, &change, &evidence)
            .await
            .is_err()
    );
    controls.approve_change(&checker, proposal).await?;
    data.apply_master(&maker, &controls, proposal, &change, &evidence)
        .await?;
    data.apply_master(&maker, &controls, proposal, &change, &evidence)
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM data.master_history")
            .fetch_one(&pool)
            .await?,
        1
    );
    assert_eq!(
        core.authority(
            accounts[0],
            bank_controls::entitlements::Action::ReadAccount
        )
        .await?
        .owners,
        vec![alice_party]
    );
    assert_eq!(core.balance(accounts[0]).await?.booked.minor_units(), 850);
    assert_eq!(core.balance(accounts[1]).await?.booked.minor_units(), 150);
    assert!(
        sqlx::query("UPDATE core.ledger_accounts SET booked=999999")
            .execute(&pool)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.parties")
            .fetch_one(&core_pool)
            .await?,
        2
    );
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM data.dead_letters WHERE event_id=$1 AND state='RESOLVED')",
    )
    .bind(gap.event_id.0)
    .fetch_one(&pool)
    .await?;
    assert!(held);
    pool.close().await;
    cp.close().await;
    core_pool.close().await;
    db.close().await?;
    std::fs::remove_dir_all(path)?;
    Ok(())
}
