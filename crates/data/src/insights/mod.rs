//! WP-38 customer insights: bank-rule service notices, and marketing suggestions (bank rules or partner
//! recommendations) shown only while the customer permits marketing. Withdrawal hides marketing insights at once;
//! the records stay as evidence. Insights carry provenance and a plain "why" and never execute anything.
use crate::alerts::actor_person;
use anyhow::Result;
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, TrustContext};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Insights {
    pub pool: PgPool,
}
impl Insights {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn list(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Customer).await?;
        Ok(
            sqlx::query_scalar("SELECT analytics.customer_insights($1,$2)")
                .bind(a)
                .bind(p)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn dismiss(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        insight: Uuid,
    ) -> Result<Value> {
        let (a, p) = actor_person(controls, actor, TrustContext::Customer).await?;
        Ok(
            sqlx::query_scalar("SELECT analytics.dismiss_insight($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(insight)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
