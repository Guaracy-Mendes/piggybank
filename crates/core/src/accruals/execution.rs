//! Narrow financial execution capability. Inputs contain identities/periods, never amounts or clocks.
use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, Currency, Money, fingerprint};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone)]
pub struct PeriodicCommands {
    pub(crate) pool: PgPool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationDetails {
    pub id: Uuid,
    pub consent_id: Uuid,
    pub consent_digest: String,
    pub counter_account_id: Uuid,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationReceipt {
    pub activation: Value,
    pub digest: String,
    pub stopped: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodReceipt {
    pub period: Value,
    pub digest: String,
    pub batch_id: Option<Uuid>,
    pub posted: bool,
    pub amount_minor: String,
}
impl Core {
    pub fn periodic_commands(&self) -> PeriodicCommands {
        PeriodicCommands {
            pool: self.pool.clone(),
        }
    }
}
impl PeriodicCommands {
    pub async fn source(&self, id: Uuid) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT core.periodic_consent_source($1)")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn activation(&self, id: Uuid) -> Result<ActivationReceipt> {
        let raw: Value = sqlx::query_scalar("SELECT core.periodic_activation_receipt($1)")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let r: ActivationReceipt = serde_json::from_value(raw)?;
        ensure!(
            fingerprint(&r.activation)? == r.digest && r.activation["id"] == serde_json::json!(id),
            "Activation receipt integrity conflict"
        );
        Ok(r)
    }
    pub async fn activate(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        d: &ActivationDetails,
    ) -> Result<ActivationReceipt> {
        ensure!(
            controls.finance_authorized(actor, true).await?,
            "Current Finance maker required"
        );
        let person = controls.person(actor).await?;
        sqlx::query("SELECT core.activate_periodic($1,$2,$3)")
            .bind(serde_json::to_value(d)?)
            .bind(serde_json::to_value(actor)?)
            .bind(person.0)
            .execute(&self.pool)
            .await?;
        self.activation(d.id).await
    }
    pub async fn stop(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        reason: &str,
    ) -> Result<ActivationReceipt> {
        ensure!(
            controls.finance_authorized(actor, true).await?,
            "Current Finance maker required"
        );
        let person = controls.person(actor).await?;
        sqlx::query("SELECT core.stop_periodic_activation($1,$2,$3,$4)")
            .bind(id)
            .bind(serde_json::to_value(actor)?)
            .bind(person.0)
            .bind(reason)
            .execute(&self.pool)
            .await?;
        self.activation(id).await
    }
    pub async fn period(
        &self,
        activation: Uuid,
        start: NaiveDate,
    ) -> Result<Option<PeriodReceipt>> {
        let id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM core.periodic_finalizations WHERE activation_id=$1 AND period_start=$2",
        )
        .bind(activation)
        .bind(start)
        .fetch_optional(&self.pool)
        .await?;
        let Some(id) = id else { return Ok(None) };
        let raw: Value = sqlx::query_scalar("SELECT core.periodic_period_receipt($1)")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let r: PeriodReceipt = serde_json::from_value(raw)?;
        ensure!(
            fingerprint(&r.period)? == r.digest && r.period["id"] == serde_json::json!(id),
            "Period receipt integrity conflict"
        );
        Ok(Some(r))
    }
    pub async fn next_due(&self, id: Uuid) -> Result<Option<NaiveDate>> {
        let a = self.activation(id).await?;
        if a.stopped {
            return Ok(None);
        }
        let p: super::PeriodicPolicy = serde_json::from_value(a.activation["policy"].clone())?;
        p.validate()?;
        let first: NaiveDate = serde_json::from_value(a.activation["firstPeriodStart"].clone())?;
        let pending:Option<NaiveDate>=sqlx::query_scalar("SELECT f.period_start FROM core.periodic_finalizations f LEFT JOIN core.periodic_postings p ON p.finalization_id=f.id WHERE f.activation_id=$1 AND p.finalization_id IS NULL ORDER BY f.period_start LIMIT 1").bind(id).fetch_optional(&self.pool).await?;
        let after: Option<NaiveDate> = sqlx::query_scalar(
            "SELECT max(period_end) FROM core.periodic_finalizations WHERE activation_id=$1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let start = pending.or(after).unwrap_or(first);
        if start >= p.effective_until {
            return Ok(None);
        }
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT core.periodic_clock()")
            .fetch_one(&self.pool)
            .await?;
        let preview = p.preview(&super::PreviewInput {
            period_start: start,
            closing_booked: match p.calculation {
                super::Calculation::DailyCreditAccrual { .. } => {
                    Some(Money::new(0, Currency::synthetic()))
                }
                _ => None,
            },
            evaluated_at: now,
        })?;
        Ok(preview.is_due.then_some(start))
    }
    pub async fn finalize(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        start: NaiveDate,
    ) -> Result<PeriodReceipt> {
        ensure!(
            controls.finance_authorized(actor, true).await?,
            "Current Finance maker required"
        );
        let person = controls.person(actor).await?;
        sqlx::query("SELECT core.finalize_periodic_period($1,$2,$3,$4)")
            .bind(id)
            .bind(start)
            .bind(serde_json::to_value(actor)?)
            .bind(person.0)
            .execute(&self.pool)
            .await?;
        self.period(id, start)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Native period receipt required"))
    }
    pub async fn post(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        start: NaiveDate,
    ) -> Result<PeriodReceipt> {
        let r = self.finalize(controls, actor, id, start).await?;
        let period_id: Uuid = serde_json::from_value(r.period["id"].clone())?;
        let person = controls.person(actor).await?;
        sqlx::query("SELECT core.post_periodic_period($1,$2,$3)")
            .bind(period_id)
            .bind(serde_json::to_value(actor)?)
            .bind(person.0)
            .execute(&self.pool)
            .await?;
        self.period(id, start)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Posted period receipt required"))
    }
    pub async fn active_page(&self, after: Uuid) -> Result<Vec<(Uuid, ActorContext)>> {
        let rows=sqlx::query("SELECT id,actor_context FROM core.periodic_activations a WHERE id>$1 AND NOT EXISTS(SELECT 1 FROM core.periodic_activation_stops s WHERE s.activation_id=a.id) ORDER BY id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| Ok((r.get("id"), serde_json::from_value(r.get("actor_context"))?)))
            .collect()
    }
}
