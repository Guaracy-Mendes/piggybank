use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_platform::{
    database::Module,
    domain::{ActorContext, EventEnvelope, EventId, Money},
    events::EventStore,
};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductVersion {
    pub id: Uuid,
    pub code: String,
    pub version: i32,
    pub name: String,
    pub label: String,
    pub effective_from: NaiveDate,
    pub effective_until: NaiveDate,
    pub minimum_age: u8,
    pub payment_limit: Money,
    pub daily_limit: Money,
    pub flat_fee: Money,
    pub fee_basis_points: u16,
    pub chart_version: String,
}
impl ProductVersion {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.label == "SYNTHETIC" && self.chart_version == "synthetic-chart-v1",
            "Only explicit synthetic policies are enabled"
        );
        ensure!(
            !self.code.is_empty()
                && self.code.len() <= 32
                && self.code.as_bytes()[0].is_ascii_uppercase()
                && self
                    .code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
            "Invalid product code"
        );
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 100
                && self.version > 0
                && self.effective_until > self.effective_from
                && self.minimum_age <= 130,
            "Invalid product version"
        );
        self.payment_limit.require_positive()?;
        self.daily_limit.require_positive()?;
        ensure!(
            self.daily_limit.minor_units() >= self.payment_limit.minor_units()
                && self.flat_fee.minor_units() >= 0
                && self.fee_basis_points <= 1000,
            "Invalid synthetic pricing or limits"
        );
        Ok(())
    }
    pub fn eligible(&self, age: u8, date: NaiveDate) -> bool {
        age >= self.minimum_age
            && age <= 130
            && date >= self.effective_from
            && date < self.effective_until
    }
    /// Positive fees round half up in exact minor units with a widened intermediate.
    pub fn fee(&self, amount: Money) -> Result<Money> {
        amount.require_positive()?;
        self.validate()?;
        ensure!(
            amount.minor_units() <= self.payment_limit.minor_units(),
            "Product payment limit exceeded"
        );
        let variable =
            (i128::from(amount.minor_units()) * i128::from(self.fee_basis_points) + 5000) / 10000;
        let fee = i64::try_from(variable + i128::from(self.flat_fee.minor_units()))?;
        Ok(Money::new(fee, amount.currency()))
    }
}
impl Core {
    pub async fn publish_product(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        proposal: Uuid,
        product: &ProductVersion,
    ) -> Result<()> {
        product.validate()?;
        let approval = controls
            .approved_change(actor, proposal, ChangeKind::Product, product.id, product)
            .await?;
        let (maker, checker) = approval.people();
        let mut tx = self.pool.begin().await?;
        let created: bool = sqlx::query_scalar("SELECT core.publish_product($1,$2,$3,$4)")
            .bind(serde_json::to_value(product)?)
            .bind(approval.id())
            .bind(maker)
            .bind(checker)
            .fetch_one(&mut *tx)
            .await?;
        if created {
            EventStore::new(self.pool.clone(),Module::Core).stage(&mut tx,&EventEnvelope{event_id:EventId::new(),event_type:"product.published".into(),schema_version:1,producer:"core".into(),aggregate_id:product.id,aggregate_version:1,occurred_at:Utc::now(),correlation_id:proposal,causation_id:proposal.to_string(),data:serde_json::json!({"productVersionId":product.id,"code":product.code,"version":product.version})}).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn product(&self, id: Uuid) -> Result<ProductVersion> {
        let value: serde_json::Value =
            sqlx::query_scalar("SELECT document FROM core.product_versions WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        Ok(serde_json::from_value(value)?)
    }
    pub async fn products(&self, date: NaiveDate) -> Result<Vec<ProductVersion>> {
        let values:Vec<serde_json::Value>=sqlx::query_scalar("SELECT document FROM core.product_versions WHERE effective_from<=$1 AND effective_until>$1 ORDER BY code LIMIT 100").bind(date).fetch_all(&self.pool).await?;
        values
            .into_iter()
            .map(|v| Ok(serde_json::from_value(v)?))
            .collect()
    }
}
