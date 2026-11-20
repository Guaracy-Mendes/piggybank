//! WP-39 routing (template with two simulated providers, user decision 2026-11-18). Route decisions are made in SQL by
//! `payments.route_decide` from the active policy version (and recorded with the shadow policy's choice); measured
//! provider health comes from recorded decisions and attempt-scoped receipts. Fallback happens only after the current
//! provider's signed non-execution and Core's verification of it (see `submission::reroute`). Benefit against a real
//! baseline is not claimed: the shadow report labels its baseline as local simulator only.
use crate::Payments;
use anyhow::Result;
use bank_platform::domain::{ActorContext, TrustContext};
use serde_json::Value;

impl Payments {
    pub(crate) async fn staff_identity(&self, actor: &ActorContext) -> Result<(Value, uuid::Uuid)> {
        anyhow::ensure!(
            actor.trust == TrustContext::Workforce,
            "Workforce context required"
        );
        Ok((
            serde_json::to_value(actor)?,
            self.controls.person(actor).await?.0,
        ))
    }
    /// Policies, head, pending changes, measured health, shadow comparison and recent decisions.
    pub async fn routing_view(&self, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.staff_identity(actor).await?;
        Ok(sqlx::query_scalar("SELECT payments.routing_view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
}
