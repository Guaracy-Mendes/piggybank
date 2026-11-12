//! Governed projections and enterprise aliases. Core financial identities remain unchanged.
use anyhow::{Result, ensure};
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_core::inquiry::Records;
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, EventEnvelope, EventId, fingerprint},
    events::{EventStore, ReplayAuthorizer},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;
pub const CONSUMER: &str = "data-core-v1";
#[derive(Clone)]
pub struct Data {
    pub pool: PgPool,
    source: Arc<dyn Records>,
}
impl Data {
    pub fn new(pool: PgPool, source: Arc<dyn Records>) -> Self {
        Self { pool, source }
    }
    pub async fn consume(&self, event: &EventEnvelope) -> Result<bool> {
        event.validate()?;
        let transport = EventStore::new(self.pool.clone(), Module::Data);
        if event.schema_version != 1
            || event.producer != "core"
            || ![
                "customer.created",
                "profile.changed",
                "product.published",
                "account.changed",
                "hold.changed",
                "journal.batch-posted",
            ]
            .contains(&event.event_type.as_str())
        {
            transport
                .quarantine(CONSUMER, event, "UNREGISTERED_CORE_EVENT")
                .await?;
            return Ok(false);
        }
        let digest = fingerprint(event)?;
        let prior: Option<String> = sqlx::query_scalar(
            "SELECT fingerprint FROM data.inbox WHERE consumer=$1 AND event_id=$2",
        )
        .bind(CONSUMER)
        .bind(event.event_id.0)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(prior) = prior {
            if prior == digest {
                return Ok(true);
            }
            transport
                .quarantine(CONSUMER, event, "CONFLICTING_EVENT_IDENTITY")
                .await?;
            return Ok(false);
        }
        // Only immutable/minimal source records are fetched; no transaction spans a source call.
        let account = if event.event_type == "account.changed" {
            Some(self.source.account(AccountId(event.aggregate_id)).await?)
        } else {
            None
        };
        let journal = if event.event_type == "journal.batch-posted" {
            Some(self.source.journal(event.aggregate_id).await?)
        } else {
            None
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO data.consumer_versions(consumer,producer,aggregate_id) VALUES($1,'core',$2) ON CONFLICT DO NOTHING").bind(CONSUMER).bind(event.aggregate_id).execute(&mut *tx).await?;
        let version:i64=sqlx::query_scalar("SELECT version FROM data.consumer_versions WHERE consumer=$1 AND producer='core' AND aggregate_id=$2 FOR UPDATE").bind(CONSUMER).bind(event.aggregate_id).fetch_one(&mut *tx).await?;
        let prior: Option<String> = sqlx::query_scalar(
            "SELECT fingerprint FROM data.inbox WHERE consumer=$1 AND event_id=$2",
        )
        .bind(CONSUMER)
        .bind(event.event_id.0)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(prior) = prior {
            tx.rollback().await?;
            if prior == digest {
                return Ok(true);
            }
            transport
                .quarantine(CONSUMER, event, "CONFLICTING_EVENT_IDENTITY")
                .await?;
            return Ok(false);
        }
        if event.aggregate_version != version + 1 {
            tx.rollback().await?;
            transport
                .quarantine(CONSUMER, event, "AGGREGATE_VERSION_GAP")
                .await?;
            return Ok(false);
        }
        match event.event_type.as_str() {
            "customer.created" | "profile.changed" => {
                self.reference(&mut tx, event, "PARTY").await?;
            }
            "product.published" => {
                self.reference(&mut tx, event, "PRODUCT").await?;
            }
            "account.changed" => {
                let source = account.expect("Registered account handler fetched its descriptor");
                let state = event
                    .data
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let debit = event.data.get("debitBlocked").and_then(Value::as_bool);
                let credit = event.data.get("creditBlocked").and_then(Value::as_bool);
                if !["PROVISIONING", "ACTIVE", "CLOSING", "CLOSED"].contains(&state)
                    || debit.is_none()
                    || credit.is_none()
                {
                    tx.rollback().await?;
                    transport
                        .quarantine(CONSUMER, event, "INVALID_ACCOUNT_FACT")
                        .await?;
                    return Ok(false);
                }
                sqlx::query("INSERT INTO data.account_views(account_id,party_id,product_version_id,state,debit_blocked,credit_blocked,source_version,source_as_of) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(account_id) DO UPDATE SET state=EXCLUDED.state,debit_blocked=EXCLUDED.debit_blocked,credit_blocked=EXCLUDED.credit_blocked,source_version=EXCLUDED.source_version,source_as_of=EXCLUDED.source_as_of,recorded_at=now() WHERE data.account_views.source_version<EXCLUDED.source_version").bind(source.account_id.0).bind(source.party_id.0).bind(source.product_version_id).bind(state).bind(debit).bind(credit).bind(event.aggregate_version).bind(event.occurred_at).execute(&mut *tx).await?;
                self.reference(&mut tx, event, "ACCOUNT").await?;
            }
            "hold.changed" => {
                let account = event
                    .data
                    .get("accountId")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                let state = event
                    .data
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let submitted = event.data.get("submitted").and_then(Value::as_bool);
                if account.is_none()
                    || submitted.is_none()
                    || !["ACTIVE", "CONSUMED", "RELEASED"].contains(&state)
                {
                    tx.rollback().await?;
                    transport
                        .quarantine(CONSUMER, event, "INVALID_HOLD_FACT")
                        .await?;
                    return Ok(false);
                }
                sqlx::query("INSERT INTO data.hold_views VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(operation_id) DO UPDATE SET state=EXCLUDED.state,submitted=EXCLUDED.submitted,source_version=EXCLUDED.source_version,source_as_of=EXCLUDED.source_as_of WHERE data.hold_views.source_version<EXCLUDED.source_version").bind(event.aggregate_id).bind(account).bind(state).bind(submitted).bind(event.aggregate_version).bind(event.occurred_at).execute(&mut *tx).await?;
            }
            "journal.batch-posted" => {
                let batch = journal.expect("Registered journal handler fetched an immutable batch");
                for entry in &batch.entries {
                    sqlx::query("INSERT INTO data.transaction_views(batch_id,account_id,operation_id,document,origin_event) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(batch.id).bind(entry.account_id.0).bind(batch.operation_id).bind(json!({"batchId":batch.id,"accountId":entry.account_id,"amount":entry.amount,"businessDate":batch.business_date,"postingKind":batch.posting_kind,"originalBatchId":batch.original_batch_id,"status":"POSTED","source":"CORE_JOURNAL"})).bind(event.event_id.0).execute(&mut *tx).await?;
                }
            }
            _ => unreachable!("Registered event types checked before opening transaction"),
        }
        sqlx::query("INSERT INTO data.inbox(consumer,event_id,fingerprint) VALUES($1,$2,$3)")
            .bind(CONSUMER)
            .bind(event.event_id.0)
            .bind(digest)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE data.consumer_versions SET version=$3 WHERE consumer=$1 AND producer='core' AND aggregate_id=$2").bind(CONSUMER).bind(event.aggregate_id).bind(event.aggregate_version).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
    async fn reference(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        event: &EventEnvelope,
        kind: &str,
    ) -> Result<()> {
        let created=sqlx::query("INSERT INTO data.master_refs(id,kind,source_ref,canonical_ref,source_version,origin_event) VALUES($1,$2,$1,$1,$3,$4) ON CONFLICT(source,kind,source_ref) DO NOTHING RETURNING id").bind(event.aggregate_id).bind(kind).bind(event.aggregate_version).bind(event.event_id.0).fetch_optional(&mut **tx).await?;
        if created.is_some() {
            self.master_fact(
                tx,
                event.aggregate_id,
                1,
                event.aggregate_id,
                event.event_id.0,
            )
            .await?;
        }
        sqlx::query("UPDATE data.master_refs SET source_version=$2,origin_event=$3,updated_at=now() WHERE id=$1 AND source_version<$2").bind(event.aggregate_id).bind(event.aggregate_version).bind(event.event_id.0).execute(&mut **tx).await?;
        Ok(())
    }
    async fn master_fact(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        version: i64,
        canonical: Uuid,
        correlation: Uuid,
    ) -> Result<()> {
        EventStore::new(self.pool.clone(), Module::Data)
            .stage(
                tx,
                &EventEnvelope {
                    event_id: EventId::new(),
                    event_type: "master.reference-changed".into(),
                    schema_version: 1,
                    producer: "data".into(),
                    aggregate_id: id,
                    aggregate_version: version,
                    occurred_at: Utc::now(),
                    correlation_id: correlation,
                    causation_id: correlation.to_string(),
                    data: json!({"referenceId":id,"canonicalRef":canonical}),
                },
            )
            .await
    }
    pub async fn replay(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        id: Uuid,
    ) -> Result<bool> {
        ensure!(
            controls.authorize_replay(actor).await?,
            "Data replay denied"
        );
        let document: Value = sqlx::query_scalar(
            "SELECT document FROM data.dead_letters WHERE consumer=$1 AND event_id=$2",
        )
        .bind(CONSUMER)
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let request = Uuid::new_v4();
        sqlx::query("INSERT INTO data.replay_audit VALUES($1,$2,$3,$4,$5,'REQUESTED',now())")
            .bind(request)
            .bind(CONSUMER)
            .bind(id)
            .bind(&actor.issuer)
            .bind(&actor.subject)
            .execute(&self.pool)
            .await?;
        let result = self.consume(&serde_json::from_value(document)?).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE data.replay_audit SET state=$2 WHERE id=$1")
            .bind(request)
            .bind(if result { "APPLIED" } else { "PENDING" })
            .execute(&mut *tx)
            .await?;
        if result {
            sqlx::query(
                "UPDATE data.dead_letters SET state='RESOLVED' WHERE consumer=$1 AND event_id=$2",
            )
            .bind(CONSUMER)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(result)
    }
    pub async fn conflict(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        reference: Uuid,
        target: Uuid,
        evidence: Uuid,
    ) -> Result<Uuid> {
        ensure!(
            actor.trust == bank_platform::domain::TrustContext::Workforce
                && controls
                    .authorize(
                        actor,
                        bank_controls::entitlements::Action::ProposeMaster,
                        None,
                        reference
                    )
                    .await?,
            "Workforce data context required"
        );
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO data.reference_conflicts(id,reference_id,proposed_canonical,evidence_ref) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(id).bind(reference).bind(target).bind(evidence).execute(&self.pool).await?;
        let id:Uuid=sqlx::query_scalar("SELECT id FROM data.reference_conflicts WHERE reference_id=$1 AND proposed_canonical=$2 AND evidence_ref=$3").bind(reference).bind(target).bind(evidence).fetch_one(&self.pool).await?;
        let cases = bank_controls::cases::Cases::new(controls.pool.clone());
        let case = cases
            .ensure(
                actor,
                bank_controls::cases::NewCase {
                    kind: bank_controls::cases::CaseKind::Data,
                    origin_id: id,
                    resource_id: reference,
                    evidence_refs: vec![evidence],
                    customer: None,
                },
            )
            .await?;
        sqlx::query("UPDATE data.reference_conflicts SET case_id=$2 WHERE id=$1")
            .bind(id)
            .bind(case)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }
    pub async fn apply_master<T: bank_controls::evidence::BlobStore>(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        proposal: Uuid,
        change: &MasterChange,
        evidence: &bank_controls::evidence::EvidenceService<T>,
    ) -> Result<()> {
        let cases = bank_controls::cases::Cases::new(controls.pool.clone());
        let case = cases.detail(actor, change.case_id).await?;
        ensure!(
            case.kind == "DATA" && case.resource_id == change.reference_id,
            "Master evidence case scope mismatch"
        );
        evidence
            .get_for_case(actor, change.case_id, change.evidence_ref, &cases)
            .await?;
        let proof = controls
            .approved_change(
                actor,
                proposal,
                ChangeKind::Master,
                change.reference_id,
                change,
            )
            .await?;
        let (maker, checker) = proof.people();
        let mut tx = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT * FROM data.master_refs WHERE id=$1 AND kind='PARTY' FOR UPDATE")
                .bind(change.reference_id)
                .fetch_one(&mut *tx)
                .await?;
        let previous: Uuid = row.get("canonical_ref");
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM data.master_history WHERE approval_id=$1)",
        )
        .bind(proof.id())
        .fetch_one(&mut *tx)
        .await?
        {
            tx.commit().await?;
            return Ok(());
        }
        let target: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM data.master_refs WHERE canonical_ref=$1 AND kind='PARTY')",
        )
        .bind(change.target_canonical)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            target
                && previous != change.target_canonical
                && row.get::<i64, _>("revision") == change.expected_revision,
            "Master reference target/version conflict"
        );
        let version = change.expected_revision + 1;
        sqlx::query("INSERT INTO data.master_history(id,reference_id,previous_canonical,new_canonical,approval_id,maker_person,checker_person,evidence_ref,case_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(Uuid::new_v4()).bind(change.reference_id).bind(previous).bind(change.target_canonical).bind(proof.id()).bind(maker).bind(checker).bind(change.evidence_ref).bind(change.case_id).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE data.master_refs SET canonical_ref=$2,revision=$3,updated_at=now() WHERE id=$1",
        )
        .bind(change.reference_id)
        .bind(change.target_canonical)
        .bind(version)
        .execute(&mut *tx)
        .await?;
        self.master_fact(
            &mut tx,
            change.reference_id,
            version,
            change.target_canonical,
            proposal,
        )
        .await?;
        sqlx::query("UPDATE data.reference_conflicts SET state='RESOLVED' WHERE reference_id=$1 AND proposed_canonical=$2 AND evidence_ref=$3").bind(change.reference_id).bind(change.target_canonical).bind(change.evidence_ref).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MasterChange {
    pub case_id: Uuid,
    pub reference_id: Uuid,
    pub target_canonical: Uuid,
    pub expected_revision: i64,
    pub evidence_ref: Uuid,
}
