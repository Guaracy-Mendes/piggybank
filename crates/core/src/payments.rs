//! Core payment execution port: same-ledger posting, fee and daily budget are atomic.
use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::entitlements::{Action, Entitlements};
use bank_platform::domain::{AccountId, ActorContext, PaymentDetails, TrustContext, fingerprint};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InternalInstruction {
    pub operation_id: Uuid,
    pub details: PaymentDetails,
    pub payee_account_id: AccountId,
    pub product_version_id: Uuid,
    pub beneficiary_version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rail_policy_version_id: Option<Uuid>,
}
/// Trusted domain evidence, never deserialized from a channel.
pub struct ExecutionPermit {
    pub request_digest: String,
    pub valid_until: DateTime<Utc>,
}
pub trait PaymentPolicy: Send + Sync {
    fn validate<'a>(
        &'a self,
        instruction: &'a InternalInstruction,
    ) -> BoxFuture<'a, Result<ExecutionPermit>>;
}
impl Core {
    pub async fn receiving_reference(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        account: AccountId,
    ) -> Result<String> {
        ensure!(
            actor.trust == TrustContext::Customer,
            "Customer receiving authority required"
        );
        let authority = self.authority(account, Action::ReadAccount).await?;
        ensure!(
            controls
                .authorize(actor, Action::ReadAccount, Some(&authority), account.0)
                .await?,
            "Account unavailable"
        );
        Ok(sqlx::query_scalar("SELECT core.receiving_reference($1)")
            .bind(account.0)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn resolve_receiver(&self, reference: &str) -> Result<AccountId> {
        ensure!(
            reference.len() == 34
                && reference.starts_with("PB")
                && reference[2..]
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()),
            "Invalid receiving reference"
        );
        let id:Uuid=sqlx::query_scalar("SELECT r.account_id FROM core.receiving_references r JOIN core.ledger_accounts a ON a.id=r.account_id WHERE r.reference=$1 AND a.state='ACTIVE' AND NOT a.credit_blocked").bind(reference).fetch_one(&self.pool).await?;
        Ok(AccountId(id))
    }
    pub async fn payment_product(
        &self,
        account: AccountId,
    ) -> Result<crate::products::ProductVersion> {
        let id=sqlx::query_scalar("SELECT r.product_version_id FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.id=$1 AND a.state='ACTIVE' AND NOT a.debit_blocked").bind(account.0).fetch_one(&self.pool).await?;
        self.product(id).await
    }
    pub async fn payment_receipt(&self, instruction: &InternalInstruction) -> Result<Option<Uuid>> {
        let row = sqlx::query(
            "SELECT batch_id,request_digest FROM core.payment_executions WHERE operation_id=$1",
        )
        .bind(instruction.operation_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = row {
            ensure!(
                row.get::<String, _>("request_digest") == fingerprint(instruction)?,
                "Payment receipt scope conflict"
            );
            Ok(Some(row.get("batch_id")))
        } else {
            Ok(None)
        }
    }
    pub async fn execute_internal_payment<P: PaymentPolicy>(
        &self,
        instruction: &InternalInstruction,
        policy: &P,
    ) -> Result<Uuid> {
        // A prior immutable receipt is authoritative even if a response was lost and the quote has since expired.
        if let Some(batch) = self.payment_receipt(instruction).await? {
            return Ok(batch);
        }
        let permit = policy.validate(instruction).await?;
        ensure!(
            permit.request_digest == fingerprint(instruction)? && permit.valid_until > Utc::now(),
            "Payment execution evidence changed or expired"
        );
        instruction.details.amount.require_positive()?;
        ensure!(instruction.details.fee.minor_units() >= 0, "Invalid fee");
        instruction
            .details
            .amount
            .checked_add(instruction.details.fee)?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM core.execute_internal_payment($1,$2,$3)")
            .bind(serde_json::to_value(instruction)?)
            .bind(permit.request_digest)
            .bind(permit.valid_until)
            .fetch_one(&mut *tx)
            .await?;
        let batch = row.get("batch_id");
        if row.get("created") {
            self.posted(&mut tx, batch, instruction.operation_id)
                .await?;
        }
        tx.commit().await?;
        Ok(batch)
    }
}
