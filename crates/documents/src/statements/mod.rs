//! WP-25 period statements: readiness-gated immutable versions from native Core journals,
//! single-use short-lived retrieval and access evidence. Authority is rechecked inside PostgreSQL.
use anyhow::{Result, ensure};
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, TrustContext};
use chrono::NaiveDate;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// A refused retrieval or link grant. The denial itself is durably recorded as access evidence.
#[derive(Debug)]
pub struct AccessDenied(pub String);
impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Document access denied: {}", self.0)
    }
}
impl std::error::Error for AccessDenied {}

/// A statement whose period or finance basis is not ready. Nothing was published.
#[derive(Debug)]
pub struct NotReady(pub Value);
impl std::fmt::Display for NotReady {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Statement basis is not ready")
    }
}
impl std::error::Error for NotReady {}

#[derive(Clone)]
pub struct Records {
    pub pool: PgPool,
}
pub(crate) fn staff(actor: &ActorContext) -> Result<bool> {
    match actor.trust {
        TrustContext::Customer => Ok(false),
        TrustContext::Workforce => Ok(true),
        _ => anyhow::bail!("Customer or workforce trust required"),
    }
}
fn outcome(value: Value) -> Result<Value> {
    if value["outcome"] == "DENIED" {
        return Err(AccessDenied(value["reason"].as_str().unwrap_or("DENIED").into()).into());
    }
    Ok(value)
}
impl Records {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub(crate) async fn call(&self, sql: &'static str, binds: Vec<Bind>) -> Result<Value> {
        let mut query = sqlx::query_scalar::<_, Value>(sql);
        for bind in binds {
            query = match bind {
                Bind::Json(v) => query.bind(v),
                Bind::Id(v) => query.bind(v),
                Bind::Date(v) => query.bind(v),
                Bind::Text(v) => query.bind(v),
                Bind::Flag(v) => query.bind(v),
            };
        }
        Ok(query.fetch_one(&self.pool).await?)
    }
    /// Idempotent: an unchanged native snapshot returns the existing version.
    pub async fn generate(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        account: Uuid,
        period_start: NaiveDate,
    ) -> Result<Value> {
        let person = controls.person(actor).await?;
        let result = self
            .call(
                "SELECT documents.generate_statement($1,$2,$3,$4,$5)",
                vec![
                    Bind::Json(serde_json::to_value(actor)?),
                    Bind::Id(person.0),
                    Bind::Id(account),
                    Bind::Date(period_start),
                    Bind::Flag(staff(actor)?),
                ],
            )
            .await?;
        if result["state"] == "NOT_READY" {
            return Err(NotReady(result["readiness"].clone()).into());
        }
        Ok(result)
    }
    pub async fn link(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        version: Uuid,
    ) -> Result<Value> {
        let person = controls.person(actor).await?;
        outcome(
            self.call(
                "SELECT documents.statement_link($1,$2,$3,$4)",
                vec![
                    Bind::Json(serde_json::to_value(actor)?),
                    Bind::Id(person.0),
                    Bind::Id(version),
                    Bind::Flag(staff(actor)?),
                ],
            )
            .await?,
        )
    }
    /// Consumes the single-use reference. Expired, reused, foreign or disposed references are denied.
    pub async fn redeem(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        token: &str,
    ) -> Result<Value> {
        ensure!(
            token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
            AccessDenied("UNKNOWN_REFERENCE".into())
        );
        let person = controls.person(actor).await?;
        outcome(
            self.call(
                "SELECT documents.redeem_statement_link($1,$2,$3,$4)",
                vec![
                    Bind::Json(serde_json::to_value(actor)?),
                    Bind::Id(person.0),
                    Bind::Text(token.to_ascii_lowercase()),
                    Bind::Flag(staff(actor)?),
                ],
            )
            .await?,
        )
    }
    pub async fn customer_documents(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<Value> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        let person = controls.person(actor).await?;
        self.call(
            "SELECT documents.customer_documents($1,$2)",
            vec![Bind::Json(serde_json::to_value(actor)?), Bind::Id(person.0)],
        )
        .await
    }
    pub async fn staff_documents(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        account: Uuid,
    ) -> Result<Value> {
        ensure!(actor.trust == TrustContext::Workforce, "Workforce required");
        let person = controls.person(actor).await?;
        self.call(
            "SELECT documents.staff_documents($1,$2,$3)",
            vec![
                Bind::Json(serde_json::to_value(actor)?),
                Bind::Id(person.0),
                Bind::Id(account),
            ],
        )
        .await
    }
    /// Worker cycle for the most recent closed month; not-ready accounts are retried on a later pass.
    pub async fn generate_due(&self, limit: i32) -> Result<i32> {
        Ok(
            sqlx::query_scalar("SELECT documents.generate_due_statements($1)")
                .bind(limit)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
pub(crate) enum Bind {
    Json(Value),
    Id(Uuid),
    Date(NaiveDate),
    Text(String),
    Flag(bool),
}
