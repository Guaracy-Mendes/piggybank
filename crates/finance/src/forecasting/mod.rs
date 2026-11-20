//! WP-40 liquidity forecasting. A forecast combines three recorded inputs: the WP-24 liquidity observation (only fresh
//! signed evidence against fresh Core positions counts as confirmed funds), the warehouse watermark, and daily
//! settlement outflow history from WP-37 facts at a fixed cut. `finance.forecast_compute` is pure, so a forecast is
//! reproduced by recomputing it from the stored inputs and the same fact cut. Each forecast is evaluated against the
//! active versioned policy: a breach restricts the policy's restricted actions and opens an owned Finance case; a
//! degraded forecast (no confirmed funds) escalates but can never justify an automated action.
use crate::Finance;
use anyhow::{Result, ensure};
use bank_platform::domain::{ActorContext, TrustContext};
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn forecast_service_actor() -> ActorContext {
    ActorContext {
        issuer: "urn:piggybank:finance".into(),
        subject: "liquidity-forecast".into(),
        client_id: "bank-worker".into(),
        trust: TrustContext::Service,
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}

impl Finance {
    pub(crate) async fn liquidity_identity(&self, actor: &ActorContext) -> Result<(Value, Uuid)> {
        ensure!(
            actor.trust == TrustContext::Workforce,
            "Workforce context required"
        );
        Ok((
            serde_json::to_value(actor)?,
            self.controls.person(actor).await?.0,
        ))
    }
    async fn record_forecast(&self, actor: Value, person: Option<Uuid>) -> Result<Value> {
        let positions = self.core.positions().await?;
        let observation = self.observe_liquidity(&positions).await?.view;
        Ok(
            sqlx::query_scalar("SELECT finance.record_forecast($1,$2,$3)")
                .bind(actor)
                .bind(person)
                .bind(serde_json::to_value(&observation)?)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// A Finance maker or approver runs a forecast now.
    pub async fn run_forecast(&self, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        self.record_forecast(a, Some(p)).await
    }
    /// Worker: records a forecast only when the active policy schedules them and the last one is old enough.
    pub async fn forecast_tick(&self) -> Result<bool> {
        let due: Option<bool> = sqlx::query_scalar("SELECT finance.forecast_due()")
            .fetch_one(&self.pool)
            .await?;
        if !due.unwrap_or(false) {
            return Ok(false);
        }
        self.record_forecast(serde_json::to_value(forecast_service_actor())?, None)
            .await?;
        Ok(true)
    }
    pub async fn forecast(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        Ok(sqlx::query_scalar("SELECT finance.forecast($1,$2,$3)")
            .bind(a)
            .bind(p)
            .bind(id)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn reproduce_forecast(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        Ok(
            sqlx::query_scalar("SELECT finance.reproduce_forecast($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Policies, recent forecasts, restrictions, automated actions and outcomes against actual outflow.
    pub async fn liquidity_view(&self, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        Ok(sqlx::query_scalar("SELECT finance.liquidity_view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn propose_liquidity_policy(
        &self,
        actor: &ActorContext,
        document: &Value,
    ) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        Ok(
            sqlx::query_scalar("SELECT finance.propose_liquidity_policy($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(document)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn decide_liquidity_policy(
        &self,
        actor: &ActorContext,
        version: i32,
        approve: bool,
    ) -> Result<Value> {
        let (a, p) = self.liquidity_identity(actor).await?;
        Ok(
            sqlx::query_scalar("SELECT finance.decide_liquidity_policy($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(version)
                .bind(approve)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
