//! Authenticated synthetic rail protocol and bounded HTTP adapter. No ledger authority.
use anyhow::{Result, ensure};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{Money, fingerprint},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
mod credit;
mod http;
pub use credit::{CreditEvidence, CreditKind};
pub use http::RailClient;
pub const PROVIDER: &str = "SYNTHETIC_RAIL";
/// WP-39: a second, interchangeable simulated provider for the same rail (template for routing).
pub const PROVIDER_B: &str = "SYNTHETIC_RAIL_B";
pub const PROVIDERS: [&str; 2] = [PROVIDER, PROVIDER_B];
pub const RAIL: &str = "SIMULATED";
pub fn signing_key() -> String {
    std::env::var("BANK_RAIL_SIGNING_KEY")
        .unwrap_or_else(|_| "synthetic-rail-authentication-key-only-2026".into())
}
/// Each provider authenticates with its own key; unknown providers have none.
pub fn provider_key(provider: &str) -> Option<String> {
    match provider {
        PROVIDER => Some(signing_key()),
        PROVIDER_B => Some(
            std::env::var("BANK_RAIL_B_SIGNING_KEY")
                .unwrap_or_else(|_| "synthetic-rail-b-authentication-key-only-2026".into()),
        ),
        _ => None,
    }
}
/// The provider named in a signed evidence body, if it is a known provider.
pub fn evidence_provider(document: &SignedDocument) -> Option<&'static str> {
    let named = document.body.get("provider")?.as_str()?;
    PROVIDERS.into_iter().find(|p| *p == named)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RailInstruction {
    pub instruction_id: Uuid,
    pub payment_id: Uuid,
    pub beneficiary_reference: String,
    pub amount: Money,
    pub requested_date: NaiveDate,
    pub input_digest: String,
    pub rail: String,
    pub authority_until: DateTime<Utc>,
}
impl RailInstruction {
    pub fn validate(&self) -> Result<()> {
        self.amount.require_positive()?;
        ensure!(
            self.instruction_id == self.payment_id
                && self.rail == RAIL
                && self.input_digest.len() == 64
                && self.input_digest.bytes().all(|c| c.is_ascii_hexdigit())
                && valid_recipient_reference(&self.beneficiary_reference),
            "Invalid simulated rail instruction"
        );
        Ok(())
    }
}
pub fn valid_recipient_reference(reference: &str) -> bool {
    reference.len() == 35
        && reference.starts_with("SIM")
        && reference[3..]
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RailStatus {
    Accepted,
    Pending,
    Settled,
    Rejected,
    Cancelled,
    NotFound,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RailEvidence {
    pub evidence_id: Uuid,
    pub provider: String,
    pub instruction_id: Uuid,
    pub payment_id: Uuid,
    pub input_digest: String,
    pub amount: Money,
    pub status: RailStatus,
    pub version: i64,
    pub occurred_at: DateTime<Utc>,
    pub settlement_date: Option<NaiveDate>,
    pub reason_code: String,
}
impl RailEvidence {
    pub fn validate_for(&self, instruction: &RailInstruction) -> Result<()> {
        self.validate_binding(
            instruction.instruction_id,
            &instruction.input_digest,
            instruction.amount,
            instruction.requested_date,
        )
    }
    pub fn validate_binding(
        &self,
        instruction_id: Uuid,
        input_digest: &str,
        amount: Money,
        requested_date: NaiveDate,
    ) -> Result<()> {
        ensure!(
            provider_key(&self.provider).is_some()
                && self.instruction_id == instruction_id
                && self.payment_id == instruction_id
                && self.input_digest == input_digest
                && self.amount == amount
                && self.version > 0
                && self.reason_code.len() <= 80
                && !self.reason_code.is_empty()
                && self
                    .reason_code
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c == b'_'),
            "Rail evidence scope conflict"
        );
        ensure!(
            self.occurred_at <= Utc::now() + chrono::Duration::seconds(30)
                && self
                    .settlement_date
                    .is_none_or(|date| date >= requested_date && date <= Utc::now().date_naive()),
            "Rail financial evidence date is invalid"
        );
        ensure!(
            (self.status == RailStatus::Settled) == self.settlement_date.is_some(),
            "Settlement finality date mismatch"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipientEvidence {
    pub provider: String,
    pub reference: String,
    pub enabled: bool,
    pub currency: bank_platform::domain::Currency,
    pub verified_at: DateTime<Utc>,
}
impl RecipientEvidence {
    pub fn validate(&self, reference: &str) -> Result<()> {
        ensure!(
            self.provider == PROVIDER
                && self.reference == reference
                && self.enabled
                && valid_recipient_reference(reference)
                && Utc::now()
                    .signed_duration_since(self.verified_at)
                    .num_seconds()
                    .unsigned_abs()
                    <= 60,
            "Current simulated recipient verification required"
        );
        Ok(())
    }
}
/// Stored transport result retains the original authenticated packet for domain receipt processing.
pub struct VerifiedEvidence {
    pub document: SignedDocument,
    pub evidence: RailEvidence,
    pub digest: String,
}
impl VerifiedEvidence {
    pub fn verify(
        document: SignedDocument,
        key: &str,
        instruction: &RailInstruction,
    ) -> Result<Self> {
        document.verify(key.as_bytes())?;
        let evidence: RailEvidence = serde_json::from_value(document.body.clone())?;
        ensure!(
            provider_key(&evidence.provider).as_deref() == Some(key),
            "Evidence must be signed by the provider it names"
        );
        evidence.validate_for(instruction)?;
        let digest = fingerprint(&evidence)?;
        Ok(Self {
            document,
            evidence,
            digest,
        })
    }
}
