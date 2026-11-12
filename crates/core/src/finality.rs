//! Authenticated, immutable external finality is required for booking or submitted-hold release.
use crate::{Core, payments::InternalInstruction};
use anyhow::{Result, ensure};
use bank_platform::{authenticity::SignedDocument, domain::fingerprint};
use bank_rails::{RailEvidence, RailStatus};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialFinality {
    pub outcome: String,
    pub batch_id: Option<Uuid>,
    pub evidence_id: Uuid,
}
impl Core {
    pub async fn external_finality(
        &self,
        instruction: &InternalInstruction,
    ) -> Result<Option<FinancialFinality>> {
        let r = sqlx::query("SELECT * FROM core.external_finalities WHERE operation_id=$1")
            .bind(instruction.operation_id)
            .fetch_optional(&self.pool)
            .await?;
        r.map(|r| {
            ensure!(
                r.get::<String, _>("request_digest") == fingerprint(instruction)?,
                "Finality request identity conflict"
            );
            Ok(FinancialFinality {
                outcome: r.get("outcome"),
                batch_id: r.get("batch_id"),
                evidence_id: r.get("evidence_id"),
            })
        })
        .transpose()
    }
    /// Live ingress authenticates freshness before durable receipt. Core revalidates archive integrity and exact financial scope.
    pub async fn finalize_external(
        &self,
        instruction: &InternalInstruction,
        packet: &SignedDocument,
    ) -> Result<FinancialFinality> {
        ensure!(
            instruction.details.rail == "SIMULATED",
            "External rail required"
        );
        packet.verify_integrity(bank_rails::signing_key().as_bytes())?;
        let e: RailEvidence = serde_json::from_value(packet.body.clone())?;
        e.validate_binding(
            instruction.operation_id,
            &fingerprint(instruction)?,
            instruction.details.amount,
            instruction.details.requested_date,
        )?;
        ensure!(
            matches!(
                e.status,
                RailStatus::Settled | RailStatus::Rejected | RailStatus::Cancelled
            ),
            "Acknowledgement is not financial finality"
        );
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("SELECT * FROM core.finalize_external($1,$2,$3,$4)")
            .bind(instruction.operation_id)
            .bind(fingerprint(instruction)?)
            .bind(serde_json::to_value(packet)?)
            .bind(fingerprint(&e)?)
            .fetch_one(&mut *tx)
            .await?;
        if r.get::<bool, _>("created") {
            if let Some(batch) = r.get::<Option<Uuid>, _>("batch_id") {
                self.posted(&mut tx, batch, instruction.operation_id)
                    .await?;
            }
            self.hold_fact(&mut tx, instruction.operation_id).await?;
        }
        let result = FinancialFinality {
            outcome: r.get("outcome"),
            batch_id: r.get("batch_id"),
            evidence_id: e.evidence_id,
        };
        tx.commit().await?;
        Ok(result)
    }
}
