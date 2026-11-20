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
        // WP-39: each provider signs with its own key; SQL accepts only the current route's provider.
        let provider = bank_rails::evidence_provider(packet)
            .ok_or_else(|| anyhow::anyhow!("Unknown rail provider"))?;
        packet.verify_integrity(
            bank_rails::provider_key(provider)
                .expect("known provider key")
                .as_bytes(),
        )?;
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
    /// WP-39: record the first route (idempotent; returns any existing route) or, with the current provider's signed
    /// non-execution evidence, re-route to another provider. Returns the current provider and attempt.
    pub async fn route_external(
        &self,
        instruction: &InternalInstruction,
        provider: &str,
        nonexecution: Option<&SignedDocument>,
    ) -> Result<(String, i32)> {
        ensure!(
            instruction.details.rail == "SIMULATED",
            "External rail required"
        );
        let mut digest = None;
        if let Some(packet) = nonexecution {
            let named = bank_rails::evidence_provider(packet)
                .ok_or_else(|| anyhow::anyhow!("Unknown rail provider"))?;
            packet.verify_integrity(
                bank_rails::provider_key(named)
                    .expect("known provider key")
                    .as_bytes(),
            )?;
            let e: RailEvidence = serde_json::from_value(packet.body.clone())?;
            e.validate_binding(
                instruction.operation_id,
                &fingerprint(instruction)?,
                instruction.details.amount,
                instruction.details.requested_date,
            )?;
            ensure!(
                e.status == RailStatus::Rejected,
                "Non-execution evidence required"
            );
            digest = Some(fingerprint(&e)?);
        }
        let row = sqlx::query("SELECT * FROM core.route_external($1,$2,$3,$4,$5)")
            .bind(instruction.operation_id)
            .bind(fingerprint(instruction)?)
            .bind(provider)
            .bind(nonexecution.map(serde_json::to_value).transpose()?)
            .bind(digest)
            .fetch_one(&self.pool)
            .await?;
        Ok((row.get("provider"), row.get("attempt")))
    }
}
