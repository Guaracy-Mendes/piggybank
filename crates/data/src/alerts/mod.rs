//! WP-26 data-quality alerts, projection repair and safe notifications. Everything here writes only the
//! Data schema (and owned cases); balances and journals stay Core-only.
use anyhow::{Result, ensure};
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, TrustContext};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Alerts {
    pub pool: PgPool,
}
pub(crate) async fn actor_person(
    controls: &Entitlements,
    actor: &ActorContext,
    trust: TrustContext,
) -> Result<(Value, Uuid)> {
    ensure!(actor.trust == trust, "Trust context mismatch");
    Ok((
        serde_json::to_value(actor)?,
        controls.person(actor).await?.0,
    ))
}
impl Alerts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn scalar(&self, sql: &'static str) -> Result<Value> {
        Ok(sqlx::query_scalar(sql).fetch_one(&self.pool).await?)
    }
    /// Worker: open owned alerts/cases for current findings and clear resolved ones.
    pub async fn quality_scan(&self) -> Result<Value> {
        self.scalar("SELECT data.quality_scan()").await
    }
    /// Worker: queue deduplicated notifications from durable sources. Returns the number queued.
    pub async fn derive(&self, limit: i32) -> Result<i32> {
        Ok(sqlx::query_scalar("SELECT data.derive_notifications($1)")
            .bind(limit)
            .fetch_one(&self.pool)
            .await?)
    }
    /// Worker: synthetic capture with bounded retries.
    pub async fn deliver(&self, limit: i32) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT data.deliver_notifications($1)")
            .bind(limit)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn quality_view(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Workforce).await?;
        Ok(sqlx::query_scalar("SELECT data.quality_view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn repair(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        batch: Uuid,
    ) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Workforce).await?;
        Ok(
            sqlx::query_scalar("SELECT data.repair_projection($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(batch)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn acknowledge(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        alert: Uuid,
        note: &str,
    ) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Workforce).await?;
        Ok(
            sqlx::query_scalar("SELECT data.acknowledge_quality_alert($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(alert)
                .bind(note)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn messages(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Customer).await?;
        Ok(sqlx::query_scalar("SELECT data.customer_messages($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn set_preference(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        purpose: &str,
        enabled: bool,
    ) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Customer).await?;
        Ok(
            sqlx::query_scalar("SELECT data.set_notification_preference($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(purpose)
                .bind(enabled)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn notifications(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Workforce).await?;
        Ok(sqlx::query_scalar("SELECT data.staff_notifications($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn set_channel(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        channel: &str,
        available: bool,
        reason: &str,
    ) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Workforce).await?;
        Ok(
            sqlx::query_scalar("SELECT data.set_notification_channel($1,$2,$3,$4,$5)")
                .bind(a)
                .bind(p)
                .bind(channel)
                .bind(available)
                .bind(reason)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
