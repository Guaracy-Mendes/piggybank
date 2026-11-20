//! WP-26 transaction monitoring and operations reporting. Patterns are evaluated over durable projected
//! journal facts; alerts are scoped to one economic episode and dispositioned by independent people.
use crate::entitlements::Entitlements;
use anyhow::{Result, ensure};
use bank_platform::domain::{ActorContext, TrustContext};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Monitoring {
    pub pool: PgPool,
}
async fn staff(controls: &Entitlements, actor: &ActorContext) -> Result<(Value, Uuid)> {
    ensure!(actor.trust == TrustContext::Workforce, "Workforce required");
    Ok((
        serde_json::to_value(actor)?,
        controls.person(actor).await?.0,
    ))
}
impl Monitoring {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// Worker: evaluate each newly projected fact once per current rule. Returns alerts created.
    pub async fn tick(&self, limit: i32) -> Result<i32> {
        Ok(sqlx::query_scalar("SELECT controls.monitoring_tick($1)")
            .bind(limit)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn scan(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.monitoring_scan($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(from)
                .bind(to)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn alerts(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.monitoring_alerts_view($1,$2)")
                .bind(a)
                .bind(p)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn propose(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        alert: Uuid,
        disposition: &str,
        rationale: &str,
    ) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.propose_monitoring_disposition($1,$2,$3,$4,$5)")
                .bind(a)
                .bind(p)
                .bind(alert)
                .bind(disposition)
                .bind(rationale)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn approve(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        alert: Uuid,
    ) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.approve_monitoring_disposition($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(alert)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// One REPEATABLE READ transaction: every count observes the same committed snapshot.
    pub async fn snapshot(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;
        let value = sqlx::query_scalar("SELECT controls.operations_snapshot($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(value)
    }
    pub async fn trace(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.operation_trace($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Worker: open pages for critical sources and escalate unacknowledged pages to the secondary.
    pub async fn page_tick(&self) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT controls.page_tick()")
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn pages(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT controls.pages_view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn acknowledge(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        page: Uuid,
    ) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.acknowledge_page($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(page)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn set_oncall(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        person: Uuid,
        tier: &str,
        active: bool,
    ) -> Result<Value> {
        let (a, p) = staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.set_oncall($1,$2,$3,$4,$5)")
                .bind(a)
                .bind(p)
                .bind(person)
                .bind(tier)
                .bind(active)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Counts only, for the token-protected metrics endpoint.
    pub async fn metrics_snapshot(&self) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT controls.metrics_snapshot()")
            .fetch_one(&self.pool)
            .await?)
    }
}
