use anyhow::{Result, ensure};
use bank_platform::domain::Money;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CreditKind {
    InboundSettled,
    ReturnSettled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditEvidence {
    pub event_type: CreditKind,
    pub movement_id: Uuid,
    pub evidence_id: Uuid,
    pub provider: String,
    pub rail: String,
    pub original_payment_id: Option<Uuid>,
    pub receiving_reference: Option<String>,
    pub amount: Money,
    pub business_date: NaiveDate,
    pub occurred_at: DateTime<Utc>,
    pub version: i64,
    pub reason_code: String,
}
impl CreditEvidence {
    pub fn validate(&self) -> Result<()> {
        self.amount.require_positive()?;
        ensure!(
            self.provider == crate::PROVIDER
                && self.rail == crate::RAIL
                && self.version == 1
                && self.business_date <= Utc::now().date_naive()
                && self.occurred_at <= Utc::now() + chrono::Duration::seconds(30)
                && !self.reason_code.is_empty()
                && self.reason_code.len() <= 80
                && self
                    .reason_code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b == b'_'),
            "Invalid final credit evidence"
        );
        match self.event_type {
            CreditKind::InboundSettled => ensure!(
                self.original_payment_id.is_none()
                    && self
                        .receiving_reference
                        .as_ref()
                        .is_some_and(|r| r.len() == 34
                            && r.starts_with("PB")
                            && r[2..]
                                .bytes()
                                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_lowercase())),
                "Explicit synthetic receiving reference required"
            ),
            CreditKind::ReturnSettled => ensure!(
                self.receiving_reference.is_none()
                    && self
                        .original_payment_id
                        .is_some_and(|id| id != self.movement_id),
                "Original payment identity required for return"
            ),
        }
        Ok(())
    }
}
