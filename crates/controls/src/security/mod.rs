//! WP-28 recovery decisions, access recertification and the security findings register.
//! Every authority check is repeated inside PostgreSQL; this module only resolves the canonical person.
use crate::entitlements::Entitlements;
use anyhow::Result;
use bank_platform::domain::ActorContext;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Security {
    pub pool: PgPool,
}
impl Security {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn who(&self, controls: &Entitlements, actor: &ActorContext) -> Result<(Value, Uuid)> {
        Ok((
            serde_json::to_value(actor)?,
            controls.person(actor).await?.0,
        ))
    }
    pub async fn request_recovery(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        key: Uuid,
        input: &Value,
    ) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.request_recovery($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(key)
                .bind(input)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn decide_recovery(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        approve: bool,
        reason: &str,
    ) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.decide_recovery($1,$2,$3,$4,$5)")
                .bind(a)
                .bind(p)
                .bind(id)
                .bind(approve)
                .bind(reason)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Issuer target of an approved recovery (internal executor use only).
    pub async fn recovery_target(&self, id: Uuid) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT controls.recovery_target($1)")
            .bind(id)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn complete_recovery(&self, id: Uuid, outcome: &Value) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT controls.complete_recovery($1,$2)")
                .bind(id)
                .bind(outcome)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn recoveries(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT controls.recovery_view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn open_review(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.open_access_review($1,$2)")
                .bind(a)
                .bind(p)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn decide_item(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        review: Uuid,
        grant: Uuid,
        decision: &str,
        reason: &str,
    ) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.decide_access_item($1,$2,$3,$4,$5,$6)")
                .bind(a)
                .bind(p)
                .bind(review)
                .bind(grant)
                .bind(decision)
                .bind(reason)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn review(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.access_review_view($1,$2)")
                .bind(a)
                .bind(p)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn report_finding(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        input: &Value,
    ) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.report_finding($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(input)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn dispose_finding(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        status: &str,
        disposition: &str,
        evidence: Option<&str>,
    ) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT controls.dispose_finding($1,$2,$3,$4,$5,$6)")
                .bind(a)
                .bind(p)
                .bind(id)
                .bind(status)
                .bind(disposition)
                .bind(evidence)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn findings(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.who(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT controls.findings_view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
}
