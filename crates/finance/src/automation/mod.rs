//! WP-40 controlled finance automation. A confirmed breach forecast proposes one settlement-prefunding action, capped by
//! the approved per-action limit. A Finance maker confirms it and a different Finance approver approves it, each while
//! the proposing forecast is still the latest, confirmed and within the policy's age limit. Core then books it once
//! (`core.execute_liquidity_action` re-checks the approval and the per-action and daily limits). A restriction is lifted
//! only by maker/checker on a current confirmed within-limits forecast.
use crate::Finance;
use anyhow::Result;
use bank_platform::domain::ActorContext;
use serde_json::{Value, json};
use uuid::Uuid;

impl Finance {
    async fn liquidity_call(&self, sql: &str, actor: &ActorContext, id: Uuid) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .bind(a)
            .bind(p)
            .bind(id)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn confirm_liquidity_action(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        self.liquidity_call(
            "SELECT finance.confirm_liquidity_action($1,$2,$3)",
            actor,
            id,
        )
        .await
    }
    /// The approver's decision; an approval is booked by Core in the same request.
    pub async fn decide_liquidity_action(
        &self,
        actor: &ActorContext,
        id: Uuid,
        approve: bool,
    ) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        let decision: Value =
            sqlx::query_scalar("SELECT finance.decide_liquidity_action($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(id)
                .bind(approve)
                .fetch_one(&self.pool)
                .await?;
        if decision["state"] != "APPROVED" {
            return Ok(decision);
        }
        self.execute_liquidity_action(actor, id).await
    }
    /// Books an approved action (also the retry after an interrupted booking); replays return the same batch.
    pub async fn execute_liquidity_action(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        let command = self
            .liquidity_call(
                "SELECT finance.liquidity_action_command($1,$2,$3)",
                actor,
                id,
            )
            .await?;
        let maker: Uuid = serde_json::from_value(command["maker"].clone())?;
        let checker: Uuid = serde_json::from_value(command["checker"].clone())?;
        let (batch, created) = self
            .corrections
            .execute_liquidity_action(
                &command["details"],
                command["digest"].as_str().unwrap_or_default(),
                maker,
                checker,
            )
            .await?;
        let mut recorded = self
            .liquidity_call(
                "SELECT finance.record_liquidity_execution($1,$2,$3)",
                actor,
                id,
            )
            .await?;
        recorded["created"] = json!(created);
        recorded["batchId"] = json!(batch);
        Ok(recorded)
    }
    pub async fn propose_restriction_lift(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        self.liquidity_call(
            "SELECT finance.propose_restriction_lift($1,$2,$3)",
            actor,
            id,
        )
        .await
    }
    pub async fn decide_restriction_lift(
        &self,
        actor: &ActorContext,
        id: Uuid,
        approve: bool,
    ) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        Ok(
            sqlx::query_scalar("SELECT finance.decide_restriction_lift($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(id)
                .bind(approve)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
