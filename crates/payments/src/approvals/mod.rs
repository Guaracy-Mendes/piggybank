//! WP-39 routing-policy approvals: a payment operator proposes a new version or a mode change (ACTIVE, SHADOW,
//! SHADOW_OFF); a different person with the payment approver role decides. A version becomes ACTIVE only after its
//! shadow evaluation reached the policy's minimum decisions, unless it was active before (rollback). Policy documents
//! cannot add providers, widen fallback beyond Core-recognised non-execution, or touch customer approval.
use crate::Payments;
use anyhow::Result;
use bank_platform::domain::ActorContext;
use serde_json::Value;
use uuid::Uuid;

impl Payments {
    pub async fn propose_route_change(
        &self,
        actor: &ActorContext,
        document: Option<&Value>,
        version: Option<i32>,
        mode: &str,
    ) -> Result<Value> {
        let (a, p) = self.staff_identity(actor).await?;
        Ok(
            sqlx::query_scalar("SELECT payments.propose_route_change($1,$2,$3,$4,$5)")
                .bind(a)
                .bind(p)
                .bind(document)
                .bind(version)
                .bind(mode)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn decide_route_change(
        &self,
        actor: &ActorContext,
        change: Uuid,
        approve: bool,
    ) -> Result<Value> {
        let (a, p) = self.staff_identity(actor).await?;
        Ok(
            sqlx::query_scalar("SELECT payments.decide_route_change($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(change)
                .bind(approve)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
