//! Incoming and returned principal: durable signed receipt, Core suspense, current controls, guarded allocation.
use crate::{Payments, types::Progress};
use anyhow::{Result, ensure};
use bank_controls::{
    cases::{CaseKind, Cases, NewCase},
    checks::{Outcome, Purpose},
    entitlements::Action,
};
use bank_core::{credit::CreditPolicy, payments::ExecutionPermit};
use bank_platform::{
    authenticity::SignedDocument,
    database::Module,
    domain::{AccountId, ActorContext, EventEnvelope, EventId, TrustContext, fingerprint},
    events::EventStore,
    operations::JobSpec,
};
use bank_rails::{CreditEvidence, CreditKind};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;
fn actor() -> ActorContext {
    ActorContext {
        issuer: "urn:piggybank:synthetic-rail".into(),
        subject: "provider".into(),
        trust: TrustContext::Service,
        client_id: "provider-simulator".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditStatus {
    pub id: Uuid,
    pub details: CreditEvidence,
    pub target_account_id: Option<AccountId>,
    pub state: String,
    pub reason_code: String,
    pub suspense_batch_id: Option<Uuid>,
    pub allocation_batch_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
struct Gates<'a>(&'a Payments);
impl CreditPolicy for Gates<'_> {
    fn validate<'a>(&'a self, e: &'a CreditEvidence) -> BoxFuture<'a, Result<ExecutionPermit>> {
        Box::pin(async move {
            let digest = fingerprint(e)?;
            let gate = self
                .0
                .checks
                .evaluate(e.movement_id, Purpose::Payment, &digest)
                .await?;
            ensure!(
                gate.outcome == Outcome::Allow,
                "Current inbound controls required"
            );
            Ok(ExecutionPermit {
                request_digest: digest,
                valid_until: gate.valid_until,
            })
        })
    }
}
impl Payments {
    pub async fn accept_credit_evidence(&self, packet: &SignedDocument) -> Result<()> {
        packet.verify(bank_rails::signing_key().as_bytes())?;
        let e: CreditEvidence = match serde_json::from_value(packet.body.clone()) {
            Ok(e) => e,
            Err(error) => {
                self.quarantine_rail(None, packet, "INVALID_TYPED_CREDIT")
                    .await?;
                return Err(error.into());
            }
        };
        if let Err(error) = e.validate() {
            self.quarantine_rail(Some(e.movement_id), packet, "INVALID_CREDIT_SCOPE")
                .await?;
            return Err(error);
        }
        let digest = fingerprint(&e)?;
        let target = self.core.credit_destination(&e).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,152))")
            .bind(e.evidence_id.to_string())
            .execute(&mut *tx)
            .await?;
        let outgoing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM payments.rail_receipts WHERE evidence_id=$1) OR EXISTS(SELECT 1 FROM payments.quotes WHERE instruction->>'operationId'=$2)",
        )
        .bind(e.evidence_id).bind(e.movement_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if outgoing {
            tx.commit().await?;
            self.quarantine_rail(Some(e.movement_id), packet, "CROSS_TYPE_EVIDENCE_IDENTITY")
                .await?;
            anyhow::bail!("Evidence identity already used");
        }
        let created=sqlx::query("INSERT INTO payments.credit_intents(movement_id,evidence_id,document,fingerprint,customer_issuer,customer_subject,target_account_id) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING RETURNING movement_id").bind(e.movement_id).bind(e.evidence_id).bind(serde_json::to_value(packet)?).bind(fingerprint(&e)?).bind(target.as_ref().map(|(_,a)|&a.issuer)).bind(target.as_ref().map(|(_,a)|&a.subject)).bind(target.as_ref().map(|(id,_)|id.0)).fetch_optional(&mut *tx).await?;
        let rows=sqlx::query("SELECT movement_id,evidence_id,fingerprint FROM payments.credit_intents WHERE movement_id=$1 OR evidence_id=$2").bind(e.movement_id).bind(e.evidence_id).fetch_all(&mut *tx).await?;
        let conflict = rows.len() != 1
            || rows.iter().any(|r| {
                r.get::<Uuid, _>("movement_id") != e.movement_id
                    || r.get::<Uuid, _>("evidence_id") != e.evidence_id
                    || r.get::<String, _>("fingerprint") != digest
            });
        if created.is_some() {
            Self::audit_in(&mut tx, &actor(), "credit.received", e.movement_id).await?;
            EventStore::new(self.pool.clone(),Module::Payments).stage(&mut tx,&EventEnvelope{event_id:EventId::new(),event_type:"credit.changed".into(),schema_version:1,producer:"payments".into(),aggregate_id:e.movement_id,aggregate_version:1,occurred_at:Utc::now(),correlation_id:e.movement_id,causation_id:e.movement_id.to_string(),data:serde_json::json!({"movementId":e.movement_id,"state":"RECEIVED","reasonCode":"AUTHENTICATED_RECEIPT"})}).await?;
        }
        tx.commit().await?;
        if conflict {
            self.quarantine_rail(Some(e.movement_id), packet, "IMMUTABLE_CREDIT_CONFLICT")
                .await?;
            if self.credit_document(e.movement_id).await.is_ok() {
                self.own_credit_exception(e.movement_id).await?;
            }
            anyhow::bail!("Credit evidence identity conflict");
        }
        // Durable receipt precedes scheduling; recovery fills a lost scheduling acknowledgment.
        self.schedule_credit(e.movement_id).await?;
        Ok(())
    }
    async fn credit_document(&self, id: Uuid) -> Result<(SignedDocument, CreditEvidence)> {
        let r = sqlx::query(
            "SELECT document,fingerprint FROM payments.credit_intents WHERE movement_id=$1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let packet: SignedDocument = serde_json::from_value(r.get("document"))?;
        packet.verify_integrity(bank_rails::signing_key().as_bytes())?;
        let e: CreditEvidence = serde_json::from_value(packet.body.clone())?;
        e.validate()?;
        ensure!(
            fingerprint(&e)? == r.get::<String, _>("fingerprint"),
            "Immutable credit receipt integrity conflict"
        );
        Ok((packet, e))
    }
    async fn schedule_credit(&self, id: Uuid) -> Result<Uuid> {
        let (_, e) = self.credit_document(id).await?;
        let op = self
            .operations
            .accept(
                &actor(),
                "credit.allocate",
                &id.to_string(),
                &e,
                JobSpec {
                    kind: "credit.allocate".into(),
                    step_key: "initial".into(),
                    payload: serde_json::json!({"movementId":id}),
                    delay_seconds: 0,
                },
            )
            .await?;
        sqlx::query("UPDATE payments.credit_intents SET operation_id=$2 WHERE movement_id=$1 AND (operation_id IS NULL OR operation_id=$2)").bind(id).bind(op.id).execute(&self.pool).await?;
        Ok(op.id)
    }
    async fn credit_state(&self, id: Uuid, state: &str, reason: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let version:Option<i64>=sqlx::query_scalar("UPDATE payments.credit_intents SET state=$2,reason_code=$3,version=version+1,updated_at=now() WHERE movement_id=$1 AND state<>'CREDITED' AND (state<>$2 OR reason_code<>$3) RETURNING version").bind(id).bind(state).bind(reason).fetch_optional(&mut *tx).await?;
        if let Some(version) = version {
            EventStore::new(self.pool.clone(),Module::Payments).stage(&mut tx,&EventEnvelope{event_id:EventId::new(),event_type:"credit.changed".into(),schema_version:1,producer:"payments".into(),aggregate_id:id,aggregate_version:version,occurred_at:Utc::now(),correlation_id:id,causation_id:id.to_string(),data:serde_json::json!({"movementId":id,"state":state,"reasonCode":reason})}).await?;
            Self::audit_in(&mut tx, &actor(), "credit.state-recorded", id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn process_credit(&self, id: Uuid) -> Result<Progress> {
        self.process_credit_with_ack_fault(id, false).await
    }
    pub async fn process_credit_with_ack_fault(
        &self,
        id: Uuid,
        lose_core_ack: bool,
    ) -> Result<Progress> {
        let (packet, e) = self.credit_document(id).await?;
        if e.event_type == CreditKind::ReturnSettled {
            let original = self
                .core
                .return_original_state(e.original_payment_id.expect("validated return"))
                .await?;
            if original == "RESERVED" {
                self.credit_state(id, "CONTROLS_PENDING", "AWAIT_ORIGINAL_FINALITY")
                    .await?;
                return Ok(Progress::Pending);
            }
        }
        let record = match self.core.record_rail_credit(&packet).await {
            Ok(r) => r,
            Err(error) if crate::execution::business_guard(&error) => {
                self.quarantine_rail(Some(e.movement_id), &packet, "FINANCIAL_CREDIT_GUARD")
                    .await?;
                self.credit_state(id, "QUARANTINED", "FINANCIAL_CREDIT_GUARD")
                    .await?;
                self.own_credit_exception(id).await?;
                return Ok(Progress::Review {
                    code: "FINANCIAL_CREDIT_GUARD".into(),
                });
            }
            Err(e) => return Err(e),
        };
        if record.allocation_batch_id.is_some() {
            self.credit_state(id, "CREDITED", "CORE_CREDITED").await?;
            return Ok(Progress::Done {
                state: "CREDITED".into(),
            });
        }
        let target = self.core.credit_destination(&e).await?;
        let Some((account, customer)) = target else {
            self.credit_state(id, "DESTINATION_RESTRICTED", "UNMATCHED_RECEIVER")
                .await?;
            self.own_credit_exception(id).await?;
            return Ok(Progress::Review {
                code: "UNMATCHED_RECEIVER".into(),
            });
        };
        if !self.core.credit_target_ready(account).await?.0 {
            self.credit_state(id, "DESTINATION_RESTRICTED", "CURRENT_CREDIT_RESTRICTION")
                .await?;
            self.own_credit_exception(id).await?;
            return Ok(Progress::Review {
                code: "CURRENT_CREDIT_RESTRICTION".into(),
            });
        }
        let digest = fingerprint(&e)?;
        for kind in Purpose::Payment.required() {
            self.checks.request(&customer, id, *kind, &digest).await?;
        }
        let gate = self.checks.evaluate(id, Purpose::Payment, &digest).await?;
        match gate.outcome {
            Outcome::Pending => {
                self.credit_state(id, "CONTROLS_PENDING", "CURRENT_CONTROLS_PENDING")
                    .await?;
                return Ok(Progress::Pending);
            }
            Outcome::Review | Outcome::Deny => {
                self.credit_state(id, "CONTROL_REVIEW", "FUNDS_REMAIN_IN_SUSPENSE")
                    .await?;
                self.own_credit_exception(id).await?;
                return Ok(Progress::Review {
                    code: "FUNDS_REMAIN_IN_SUSPENSE".into(),
                });
            }
            Outcome::Allow => {}
        }
        match self.core.allocate_rail_credit(&e, &Gates(self)).await {
            Ok(_) => {
                ensure!(
                    !lose_core_ack,
                    "Simulated credit allocation acknowledgement loss after Core commit"
                );
                self.credit_state(id, "CREDITED", "CORE_CREDITED").await?;
                Ok(Progress::Done {
                    state: "CREDITED".into(),
                })
            }
            Err(error) if crate::execution::business_guard(&error) => {
                self.credit_state(id, "DESTINATION_RESTRICTED", "CURRENT_CREDIT_GUARD")
                    .await?;
                self.own_credit_exception(id).await?;
                Ok(Progress::Review {
                    code: "CURRENT_CREDIT_GUARD".into(),
                })
            }
            Err(e) => Err(e),
        }
    }
    pub async fn own_credit_exception(&self, id: Uuid) -> Result<()> {
        let r=sqlx::query("SELECT customer_issuer,customer_subject,target_account_id FROM payments.credit_intents WHERE movement_id=$1").bind(id).fetch_one(&self.pool).await?;
        let customer = r
            .get::<Option<String>, _>("customer_issuer")
            .zip(r.get::<Option<String>, _>("customer_subject"));
        Cases::new(self.controls.pool.clone())
            .ensure(
                &actor(),
                NewCase {
                    kind: CaseKind::Payment,
                    origin_id: id,
                    resource_id: r.get::<Option<Uuid>, _>("target_account_id").unwrap_or(id),
                    evidence_refs: vec![],
                    customer,
                },
            )
            .await?;
        Ok(())
    }
    pub(crate) async fn recover_credits(&self) -> Result<()> {
        let rows = self
            .recovery_page(crate::recovery::RecoveryPage::Credits)
            .await?;
        for row in rows {
            let id: Uuid = row.get("movement_id");
            let op = if let Some(op) = row.get::<Option<Uuid>, _>("operation_id") {
                op
            } else {
                self.schedule_credit(id).await?
            };
            if self.operations.status(&actor(), op).await?.state != "REVIEW_REQUIRED" {
                continue;
            }
            self.own_credit_exception(id).await?;
            let (_, e) = self.credit_document(id).await?;
            let record = self.core.credit_record(id).await?;
            let Some((account, _)) = self.core.credit_destination(&e).await? else {
                continue;
            };
            let (ready, version) = self.core.credit_target_ready(account).await?;
            let gate = self
                .checks
                .evaluate(id, Purpose::Payment, &fingerprint(&e)?)
                .await?;
            let original_ready = e.original_payment_id.is_some()
                && self
                    .core
                    .return_original_state(e.original_payment_id.unwrap())
                    .await?
                    == "SETTLED"
                && row.get::<String, _>("state") == "CONTROLS_PENDING";
            if record
                .as_ref()
                .is_some_and(|r| r.allocation_batch_id.is_some())
                || (ready
                    && (gate.outcome == Outcome::Allow
                        || original_ready
                        || row.get::<String, _>("state") == "DESTINATION_RESTRICTED"))
            {
                self.operations
                    .resume_review(
                        &actor(),
                        op,
                        JobSpec {
                            kind: "credit.allocate".into(),
                            step_key: format!("credit-{}-{}", gate.revision, version),
                            payload: serde_json::json!({"movementId":id}),
                            delay_seconds: 0,
                        },
                    )
                    .await?;
            }
        }
        Ok(())
    }
    pub async fn credit_status(&self, id: Uuid) -> Result<CreditStatus> {
        let r = sqlx::query("SELECT * FROM payments.credit_intents WHERE movement_id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let (_, e) = self.credit_document(id).await?;
        let record = self.core.credit_record(id).await?;
        if let Some(record) = &record {
            ensure!(
                record.request_digest == fingerprint(&e)?,
                "Core credit scope conflicts with orchestration receipt"
            );
        }
        let allocated = record.as_ref().and_then(|r| r.allocation_batch_id);
        ensure!(
            r.get::<String, _>("state") != "CREDITED" || allocated.is_some(),
            "Financial credit evidence missing"
        );
        Ok(CreditStatus {
            id,
            details: e,
            target_account_id: record
                .as_ref()
                .and_then(|r| r.target_account_id)
                .or(r.get::<Option<Uuid>, _>("target_account_id").map(AccountId)),
            state: if allocated.is_some() {
                "CREDITED".into()
            } else {
                r.get("state")
            },
            reason_code: r.get("reason_code"),
            suspense_batch_id: record.map(|r| r.suspense_batch_id),
            allocation_batch_id: allocated,
            operation_id: r.get("operation_id"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
    }
    pub async fn customer_credits(&self, customer: &ActorContext) -> Result<Vec<CreditStatus>> {
        ensure!(
            customer.trust == TrustContext::Customer,
            "Customer required"
        );
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT movement_id FROM payments.credit_intents WHERE customer_issuer=$1 AND customer_subject=$2 ORDER BY created_at DESC LIMIT 100").bind(&customer.issuer).bind(&customer.subject).fetch_all(&self.pool).await?;
        let mut results = vec![];
        for id in ids {
            let s = self.credit_status(id).await?;
            if let Some(account) = s.target_account_id {
                self.authorize(customer, account, Action::ReadAccount)
                    .await?;
                results.push(s);
            }
        }
        Ok(results)
    }
    pub async fn staff_credits(&self, staff: &ActorContext) -> Result<Vec<CreditStatus>> {
        ensure!(
            self.controls.payment_inquiry_authorized(staff).await?,
            "Current payment inquiry role and queue required"
        );
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT movement_id FROM payments.credit_intents ORDER BY created_at DESC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut results = vec![];
        for id in ids {
            results.push(self.credit_status(id).await?);
        }
        Ok(results)
    }
}
