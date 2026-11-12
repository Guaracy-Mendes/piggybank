//! Live authenticity precedes durable ingress. Financial completion comes only from Core.
use crate::{Payments, types::Progress};
use anyhow::{Result, ensure};
use bank_platform::{authenticity::SignedDocument, domain::fingerprint};
use bank_rails::{RailEvidence, RailInstruction, RailStatus, VerifiedEvidence};
use uuid::Uuid;
impl Payments {
    pub async fn accept_rail_evidence(&self, packet: &SignedDocument) -> Result<()> {
        packet.verify(bank_rails::signing_key().as_bytes())?;
        if packet.body.get("eventType").is_some() {
            return self.accept_credit_evidence(packet).await;
        }
        let evidence: RailEvidence = match serde_json::from_value(packet.body.clone()) {
            Ok(e) => e,
            Err(e) => {
                self.quarantine_rail(None, packet, "INVALID_TYPED_EVIDENCE")
                    .await?;
                return Err(e.into());
            }
        };
        let work: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT document FROM payments.rail_submissions WHERE payment_id=$1",
        )
        .bind(evidence.payment_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(work) = work else {
            self.quarantine_rail(
                Some(evidence.payment_id),
                packet,
                "UNMATCHED_FINANCIAL_IDENTITY",
            )
            .await?;
            anyhow::bail!("Unknown provider work");
        };
        let instruction: RailInstruction = serde_json::from_value(work)?;
        let verified = match VerifiedEvidence::verify(
            packet.clone(),
            &bank_rails::signing_key(),
            &instruction,
        ) {
            Ok(v) => v,
            Err(e) => {
                self.quarantine_rail(Some(evidence.payment_id), packet, "EVIDENCE_SCOPE_CONFLICT")
                    .await?;
                self.own_exception(evidence.payment_id).await?;
                return Err(e);
            }
        };
        let intent = self.intent(evidence.payment_id).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT id FROM payments.intents WHERE id=$1 FOR UPDATE")
            .bind(intent.id)
            .fetch_one(&mut *tx)
            .await?;
        let state = self
            .record_rail_evidence(&mut tx, intent.id, &verified)
            .await?;
        if state != "CONFLICT" && evidence.status != RailStatus::NotFound {
            self.transition(
                &mut tx,
                &intent.actor,
                intent.id,
                state,
                if state == "RECONCILING" {
                    "FINAL_EVIDENCE_REQUIRES_CORE"
                } else {
                    "PROVIDER_ACKNOWLEDGED"
                },
                None,
            )
            .await?;
        }
        tx.commit().await?;
        if state == "CONFLICT" {
            self.quarantine_rail(Some(intent.id), packet, "IMMUTABLE_PROVIDER_CONFLICT")
                .await?;
            self.own_exception(intent.id).await?;
            anyhow::bail!("Conflicting provider evidence");
        }
        Ok(())
    }
    pub(crate) async fn quarantine_rail(
        &self,
        id: Option<Uuid>,
        packet: &SignedDocument,
        reason: &str,
    ) -> Result<()> {
        let digest = fingerprint(&packet.body)?;
        let mut tx = self.pool.begin().await?;
        let created=sqlx::query("INSERT INTO payments.rail_quarantine VALUES($1,$2,$3,$4,$5,now()) ON CONFLICT(digest,reason_code) DO NOTHING RETURNING id").bind(Uuid::new_v4()).bind(id).bind(&digest).bind(reason).bind(serde_json::to_value(packet)?).fetch_optional(&mut *tx).await?;
        let quarantine: Uuid = sqlx::query_scalar(
            "SELECT id FROM payments.rail_quarantine WHERE digest=$1 AND reason_code=$2",
        )
        .bind(digest)
        .bind(reason)
        .fetch_one(&mut *tx)
        .await?;
        let actor = bank_platform::domain::ActorContext {
            issuer: "urn:piggybank:synthetic-rail".into(),
            subject: "evidence-ingress".into(),
            trust: bank_platform::domain::TrustContext::Service,
            client_id: "provider-simulator".into(),
            person_id: None,
            represented_party: None,
            roles: vec![],
        };
        if created.is_some() {
            Self::audit_in(&mut tx, &actor, "rail.evidence-quarantined", quarantine).await?;
        }
        tx.commit().await?;
        bank_controls::cases::Cases::new(self.controls.pool.clone())
            .ensure(
                &actor,
                bank_controls::cases::NewCase {
                    kind: bank_controls::cases::CaseKind::Payment,
                    origin_id: quarantine,
                    resource_id: id.unwrap_or(quarantine),
                    evidence_refs: vec![],
                    customer: None,
                },
            )
            .await?;
        Ok(())
    }
    pub async fn reconcile_external_with_ack_fault(
        &self,
        id: Uuid,
        lose_core_ack: bool,
    ) -> Result<Option<Progress>> {
        let intent = self.intent(id).await?;
        let q = self.quote_record(intent.quote_id).await?;
        ensure!(
            q.instruction.details.rail == "SIMULATED",
            "External rail required"
        );
        let mut finality = self.core.external_finality(&q.instruction).await?;
        if finality.is_none() {
            let packet:Option<serde_json::Value>=sqlx::query_scalar("SELECT document FROM payments.rail_receipts WHERE payment_id=$1 AND document->'body'->>'status' IN ('SETTLED','REJECTED','CANCELLED') ORDER BY provider_version DESC LIMIT 1").bind(id).fetch_optional(&self.pool).await?;
            let Some(packet) = packet else {
                return Ok(None);
            };
            let packet: SignedDocument = serde_json::from_value(packet)?;
            finality = Some(self.core.finalize_external(&q.instruction, &packet).await?);
            ensure!(
                !lose_core_ack,
                "Simulated finality acknowledgement loss after Core commit"
            );
        }
        let finality = finality.expect("checked financial finality");
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT id FROM payments.intents WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        // Canonical Core facts can repair a contradictory orchestration row. Nonfinancial callbacks cannot do this.
        let reason = if finality.outcome == "SETTLED" {
            "CORE_BOOKED"
        } else {
            "AUTHENTICATED_NONEXECUTION"
        };
        let changed=sqlx::query("UPDATE payments.intents SET state=$2,reason_code=$3,batch_id=$4,version=version+1,updated_at=now() WHERE id=$1 AND (state<>$2 OR reason_code<>$3 OR batch_id IS DISTINCT FROM $4)").bind(id).bind(&finality.outcome).bind(reason).bind(finality.batch_id).execute(&mut *tx).await?;
        if changed.rows_affected() == 1 {
            self.fact(
                &mut tx,
                &intent.actor,
                id,
                "payment.financial-finality-recorded",
            )
            .await?;
        }
        tx.commit().await?;
        Ok(Some(Progress::Done {
            state: finality.outcome,
        }))
    }
    pub async fn staff_quarantine(
        &self,
        actor: &bank_platform::domain::ActorContext,
    ) -> Result<serde_json::Value> {
        ensure!(
            self.controls.payment_inquiry_authorized(actor).await?,
            "Current payment inquiry role and queue required"
        );
        use sqlx::Row;
        let rows=sqlx::query("SELECT id,payment_id,digest,reason_code,recorded_at FROM payments.rail_quarantine ORDER BY recorded_at DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        Ok(
            serde_json::json!({"items":rows.into_iter().map(|r|serde_json::json!({"id":r.get::<Uuid,_>("id"),"paymentId":r.get::<Option<Uuid>,_>("payment_id"),"digest":r.get::<String,_>("digest"),"reasonCode":r.get::<String,_>("reason_code"),"recordedAt":r.get::<chrono::DateTime<chrono::Utc>,_>("recorded_at")})).collect::<Vec<_>>()}),
        )
    }
}
