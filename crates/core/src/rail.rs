use crate::{
    Core,
    payments::{InternalInstruction, PaymentPolicy},
};
use anyhow::{Result, ensure};
use bank_platform::domain::{AccountId, Currency, Money, fingerprint};
use chrono::Utc;
use serde::Serialize;
use sqlx::Row;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reservation {
    pub account_id: AccountId,
    pub amount: Money,
    pub state: String,
    pub submitted: bool,
    pub authority_until: chrono::DateTime<Utc>,
}
impl Core {
    pub async fn external_reservation(
        &self,
        instruction: &InternalInstruction,
    ) -> Result<Option<Reservation>> {
        let row=sqlx::query("SELECT r.request_digest,r.authority_until,h.* FROM core.external_reservations r JOIN core.holds h ON h.operation_id=r.operation_id WHERE r.operation_id=$1").bind(instruction.operation_id).fetch_optional(&self.pool).await?;
        if let Some(row) = row {
            ensure!(
                row.get::<String, _>("request_digest") == fingerprint(instruction)?,
                "Reservation identity conflict"
            );
            Ok(Some(Reservation {
                account_id: AccountId(row.get("account_id")),
                amount: Money::new(row.get("amount"), Currency::synthetic()),
                state: row.get("state"),
                submitted: row.get("submitted"),
                authority_until: row.get("authority_until"),
            }))
        } else {
            Ok(None)
        }
    }
    pub async fn reserve_external_payment<P: PaymentPolicy>(
        &self,
        instruction: &InternalInstruction,
        policy: &P,
    ) -> Result<Reservation> {
        if let Some(prior) = self.external_reservation(instruction).await? {
            ensure!(
                prior.state != "RELEASED",
                "Original reservation was released"
            );
            return Ok(prior);
        }
        let permit = policy.validate(instruction).await?;
        ensure!(
            permit.request_digest == fingerprint(instruction)? && permit.valid_until > Utc::now(),
            "Current bound execution permit required"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.reserve_external_payment($1,$2,$3)")
            .bind(serde_json::to_value(instruction)?)
            .bind(permit.request_digest)
            .bind(permit.valid_until)
            .execute(&mut *tx)
            .await?;
        self.hold_fact(&mut tx, instruction.operation_id).await?;
        tx.commit().await?;
        self.external_reservation(instruction)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Reservation acknowledgement missing"))
    }
    pub async fn mark_external_submitted(
        &self,
        instruction: &InternalInstruction,
        until: chrono::DateTime<Utc>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.mark_external_submitted($1,$2,$3)")
            .bind(instruction.operation_id)
            .bind(fingerprint(instruction)?)
            .bind(until)
            .execute(&mut *tx)
            .await?;
        self.hold_fact(&mut tx, instruction.operation_id).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn release_external_unsubmitted(
        &self,
        instruction: &InternalInstruction,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.release_external_unsubmitted($1,$2)")
            .bind(instruction.operation_id)
            .bind(fingerprint(instruction)?)
            .execute(&mut *tx)
            .await?;
        self.hold_fact(&mut tx, instruction.operation_id).await?;
        tx.commit().await?;
        Ok(())
    }
}
