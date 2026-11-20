//! WP-41 outcome KPIs (local capacity template, user decision 2026-11-19). `analytics.kpi_snapshot` computes KPI-01..08
//! with explicit numerators, denominators, exclusions and segments from local synthetic records and stores an immutable,
//! digested snapshot. Every snapshot is labelled local synthetic: production outcomes and ratified targets need DEC-07
//! and DEC-12. KPI-07 is a sampled human review and is reported as `MANUAL_SAMPLE_REQUIRED`.
use crate::analytics::Warehouse;
use anyhow::Result;
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::ActorContext;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KpiWindow {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl Warehouse {
    pub async fn kpi_snapshot(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        window: &KpiWindow,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT analytics.kpi_snapshot($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(window.from)
                .bind(window.to)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn kpi_snapshots(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(
            sqlx::query_scalar("SELECT analytics.kpi_snapshot_list($1,$2)")
                .bind(a)
                .bind(p)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
