use crate::{Finance, protocol::valid_code};
use anyhow::{Result, ensure};
use bank_controls::governance::ChangeKind;
use bank_platform::domain::{ActorContext, fingerprint};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;
use uuid::Uuid;
#[derive(Debug)]
pub(crate) struct MappingUnavailable;
impl std::fmt::Display for MappingUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Effective GL mapping unavailable")
    }
}
impl std::error::Error for MappingUnavailable {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartVersion {
    pub id: Uuid,
    pub code: String,
    pub version: i32,
    pub label: String,
    pub effective_from: NaiveDate,
    pub effective_until: NaiveDate,
    pub accounts: BTreeMap<String, String>,
    pub by_kind: BTreeMap<String, String>,
    pub by_account: BTreeMap<Uuid, String>,
}
impl ChartVersion {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_nil()
                && self.code == "synthetic-chart-v1"
                && self.version > 0
                && self.label == "SYNTHETIC"
                && self.effective_until > self.effective_from,
            "Valid versioned synthetic chart required"
        );
        ensure!(
            (2..=64).contains(&self.accounts.len())
                && !self.by_kind.is_empty()
                && self.by_kind.len() <= 3
                && self.by_account.len() <= 64,
            "Bounded chart mappings required"
        );
        for (code, kind) in &self.accounts {
            ensure!(
                valid_code(code)
                    && ["ASSET", "LIABILITY", "REVENUE", "EXPENSE", "EQUITY"]
                        .contains(&kind.as_str()),
                "Explicit valid GL account classification required"
            );
        }
        ensure!(
            self.by_kind
                .keys()
                .all(|k| ["CUSTOMER", "SETTLEMENT", "CONTROL"].contains(&k.as_str()))
                && self
                    .by_kind
                    .values()
                    .chain(self.by_account.values())
                    .all(|v| self.accounts.contains_key(v)),
            "GL mappings must resolve declared accounts"
        );
        for (kind, class) in [("CUSTOMER", "LIABILITY"), ("SETTLEMENT", "ASSET")] {
            if let Some(code) = self.by_kind.get(kind) {
                ensure!(
                    self.accounts.get(code).is_some_and(|c| c == class),
                    "GL account kind classification mismatch"
                );
            }
        }
        Ok(())
    }
}
impl Finance {
    pub async fn publish_chart(
        &self,
        actor: &ActorContext,
        proposal: Uuid,
        chart: &ChartVersion,
    ) -> Result<()> {
        chart.validate()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,164))")
            .bind(&chart.code)
            .execute(&mut *tx)
            .await?;
        let approved = self
            .controls
            .approved_change(actor, proposal, ChangeKind::FinanceChart, chart.id, chart)
            .await?;
        let (maker, checker) = approved.people();
        ensure!(
            self.controls.person(actor).await?.0 == maker,
            "Original canonical chart maker required"
        );
        let inserted=sqlx::query("INSERT INTO finance.chart_versions(id,chart_code,version,document,approval_id,maker_person,checker_person) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(id) DO NOTHING").bind(chart.id).bind(&chart.code).bind(chart.version).bind(serde_json::to_value(chart)?).bind(proposal).bind(maker).bind(checker).execute(&mut *tx).await?.rows_affected()==1;
        let r = sqlx::query("SELECT document,approval_id FROM finance.chart_versions WHERE id=$1")
            .bind(chart.id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            fingerprint(&r.get::<serde_json::Value, _>("document"))? == fingerprint(chart)?
                && r.get::<Uuid, _>("approval_id") == proposal,
            "Chart publication identity conflict"
        );
        if inserted {
            Self::audit_in(&mut tx, actor, "finance.chart.published", chart.id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn chart_at(&self, code: &str, date: NaiveDate) -> Result<ChartVersion> {
        let doc:serde_json::Value=sqlx::query_scalar("SELECT document FROM finance.chart_versions WHERE chart_code=$1 AND (document->>'effectiveFrom')::date<=$2 AND (document->>'effectiveUntil')::date>$2 ORDER BY version DESC LIMIT 1").bind(code).bind(date).fetch_optional(&self.pool).await?.ok_or(MappingUnavailable)?;
        let chart: ChartVersion = serde_json::from_value(doc)?;
        chart.validate()?;
        Ok(chart)
    }
}
