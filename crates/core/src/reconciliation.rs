//! Read-only financial positions for reconciliation. Orchestration never writes these facts.
use crate::Core;
use anyhow::Result;
use bank_platform::domain::{AccountId, Currency, Money};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalPosition {
    pub payment_id: Uuid,
    pub request_digest: String,
    pub account_id: AccountId,
    pub total: Money,
    pub submitted: bool,
    pub hold_state: String,
    pub held_since: DateTime<Utc>,
    pub hold_updated_at: DateTime<Utc>,
    pub final_outcome: Option<String>,
    pub batch_id: Option<Uuid>,
    pub daily_usage_state: String,
    pub account_held: Money,
    pub computed_held: Money,
}
impl Core {
    pub async fn external_positions(&self, after: Uuid) -> Result<Vec<ExternalPosition>> {
        let rows = sqlx::query(
            "SELECT r.operation_id,r.request_digest,h.account_id,h.amount,h.submitted,h.state,h.created_at,h.updated_at,
             f.outcome,x.batch_id,u.state AS usage_state,a.held,
             (SELECT coalesce(sum(v.amount),0)::bigint FROM core.holds v WHERE v.account_id=h.account_id AND v.state='ACTIVE') AS computed_held
             FROM core.external_reservations r JOIN core.holds h USING(operation_id)
             JOIN core.ledger_accounts a ON a.id=h.account_id
             JOIN core.payment_limit_usage u USING(operation_id)
             LEFT JOIN core.external_finalities f USING(operation_id)
             LEFT JOIN core.payment_executions x USING(operation_id)
             WHERE r.operation_id>$1 ORDER BY r.operation_id LIMIT 50",
        )
        .bind(after)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ExternalPosition {
                payment_id: r.get("operation_id"),
                request_digest: r.get("request_digest"),
                account_id: AccountId(r.get("account_id")),
                total: Money::new(r.get("amount"), Currency::synthetic()),
                submitted: r.get("submitted"),
                hold_state: r.get("state"),
                held_since: r.get("created_at"),
                hold_updated_at: r.get("updated_at"),
                final_outcome: r.get("outcome"),
                batch_id: r.get("batch_id"),
                daily_usage_state: r.get("usage_state"),
                account_held: Money::new(r.get("held"), Currency::synthetic()),
                computed_held: Money::new(r.get("computed_held"), Currency::synthetic()),
            })
            .collect())
    }
}
