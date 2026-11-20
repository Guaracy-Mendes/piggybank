//! WP-37 governed warehouse with one local adapter (user decision 2026-11-18), the template for later analytical
//! storage/processing adapters (DEC-05/06/08 pending). Everything runs through `analytics.*` definer functions under the
//! Data role: versioned dataset schemas, append-only facts keyed by economic identity, versioned transforms and immutable
//! snapshots with lineage, purpose checks, source watermarks and Core control totals. Nothing here can reach Core writes:
//! this service holds only the Data pool, and the Data role has no Core table or function privileges.
use crate::alerts::actor_person;
use anyhow::{Result, bail};
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, TrustContext};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// The single adapter built locally. A selected product registers its own code and implements the same functions.
pub const LOCAL_ADAPTER: &str = "LOCAL_WAREHOUSE";

/// A purpose or role refusal recorded in `analytics.access_denials`.
#[derive(Debug)]
pub struct Denied(pub String);
impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "analytics request denied: {}", self.0)
    }
}
impl std::error::Error for Denied {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Backfill {
    pub dataset: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaVersion {
    pub dataset: String,
    pub version: i32,
    pub fields: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotRequest {
    pub transform: String,
    pub version: i32,
    pub purpose: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Clone)]
pub struct Warehouse {
    pub pool: PgPool,
}
impl Warehouse {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub(crate) async fn staff(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<(Value, Uuid)> {
        actor_person(controls, actor, TrustContext::Workforce).await
    }
    /// Worker: one incremental pass per dataset; rows younger than `safe_lag_seconds` wait for the next pass.
    pub async fn ingest_due(&self, safe_lag_seconds: f64) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT analytics.ingest_due(make_interval(secs => $1),5000)")
                .bind(safe_lag_seconds)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn view(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT analytics.view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn backfill(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        request: &Backfill,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT analytics.backfill($1,$2,$3,$4,$5)")
                .bind(a)
                .bind(p)
                .bind(&request.dataset)
                .bind(request.from)
                .bind(request.to)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn register_schema(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        request: &SchemaVersion,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT analytics.register_schema($1,$2,$3,$4,$5)")
                .bind(a)
                .bind(p)
                .bind(&request.dataset)
                .bind(request.version)
                .bind(&request.fields)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Purpose and role are checked first; a refusal is recorded and returned as [`Denied`].
    pub async fn snapshot(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        request: &SnapshotRequest,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        let result: Value =
            sqlx::query_scalar("SELECT analytics.create_snapshot($1,$2,$3,$4,$5,$6,$7)")
                .bind(a)
                .bind(p)
                .bind(&request.transform)
                .bind(request.version)
                .bind(&request.purpose)
                .bind(request.from)
                .bind(request.to)
                .fetch_one(&self.pool)
                .await?;
        if result["outcome"] == "DENIED" {
            bail!(Denied(
                result["reason"].as_str().unwrap_or("DENIED").to_owned()
            ));
        }
        Ok(result)
    }
    pub async fn detail(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT analytics.snapshot($1,$2,$3)")
            .bind(a)
            .bind(p)
            .bind(id)
            .fetch_one(&self.pool)
            .await?)
    }
    /// Recomputes the snapshot from immutable facts at its stored cuts and records whether the digest matched.
    pub async fn reproduce(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT analytics.reproduce($1,$2,$3)")
            .bind(a)
            .bind(p)
            .bind(id)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn trace(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        row: i32,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT analytics.trace($1,$2,$3,$4)")
            .bind(a)
            .bind(p)
            .bind(id)
            .bind(row)
            .fetch_one(&self.pool)
            .await?)
    }
}
