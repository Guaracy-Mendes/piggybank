//! WP-33 delegated account authority (FR-ACC-08). An owner grants a delegate VIEW, or PAY up to a per-payment
//! limit, for a fixed term. Grants are immutable; revocation (by the owner) or renunciation (by the delegate) is
//! prospective: Core's account authority stops listing the delegate at once, while every past action keeps the
//! delegate actor recorded on its own intent and audit records.
use crate::Core;
use anyhow::Result;
use bank_platform::domain::{AccountId, PartyId};
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegatedAccount {
    pub account_id: AccountId,
    pub delegation_id: Uuid,
    pub authority: String,
    pub payment_limit_minor: Option<i64>,
}

impl Core {
    /// A short-lived reference the delegate shares with an owner (24 hours; reused while more than an hour remains).
    pub async fn delegate_reference(&self, person: PartyId) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT core.delegate_reference($1)")
            .bind(person.0)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn grant_delegation(&self, owner: PartyId, key: Uuid, data: &Value) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT core.grant_delegation($1,$2,$3)")
            .bind(owner.0)
            .bind(key)
            .bind(data)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn revoke_delegation(
        &self,
        person: PartyId,
        id: Uuid,
        reason: &str,
    ) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT core.revoke_delegation($1,$2,$3)")
                .bind(person.0)
                .bind(id)
                .bind(reason)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn delegations_for(&self, person: PartyId) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT core.delegations_for($1)")
            .bind(person.0)
            .fetch_one(&self.pool)
            .await?)
    }
    /// Accounts the person may currently act on as a delegate (never accounts they own).
    pub async fn delegated_accounts(&self, person: PartyId) -> Result<Vec<DelegatedAccount>> {
        Ok(sqlx::query("SELECT d.account_id,d.id,d.authority,d.payment_limit_minor FROM core.account_delegations d JOIN core.ledger_accounts a ON a.id=d.account_id WHERE d.delegate_person=$1 AND d.revoked_at IS NULL AND d.expires_at>clock_timestamp() ORDER BY d.created_at,d.id LIMIT 100")
            .bind(person.0)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|r| DelegatedAccount {
                account_id: AccountId(r.get("account_id")),
                delegation_id: r.get("id"),
                authority: r.get("authority"),
                payment_limit_minor: r.get("payment_limit_minor"),
            })
            .collect())
    }
}
