//! Selected synthetic GL protocol: strict durable deduplication by original Core batch.
use anyhow::{Result, ensure};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{Currency, Money, fingerprint},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;
pub fn signing_key() -> String {
    std::env::var("BANK_FINANCE_SIGNING_KEY")
        .unwrap_or_else(|_| "synthetic-finance-authentication-key-only-2026".into())
}
pub fn valid_code(s: &str) -> bool {
    (2..=48).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlEntry {
    pub core_account_id: Uuid,
    pub gl_account: String,
    pub delta: Money,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlBatch {
    pub export_id: Uuid,
    pub source_batch_id: Uuid,
    pub original_source_batch_id: Option<Uuid>,
    pub core_operation_id: Uuid,
    pub posting_kind: String,
    pub business_date: NaiveDate,
    pub chart_id: Uuid,
    pub chart_version: i32,
    pub chart_digest: String,
    pub source_digest: String,
    pub debit_total: String,
    pub credit_total: String,
    pub entries: Vec<GlEntry>,
}
impl GlBatch {
    pub fn totals(entries: &[GlEntry]) -> (String, String) {
        let mut debit = 0i128;
        let mut credit = 0i128;
        for entry in entries {
            let n = i128::from(entry.delta.minor_units());
            if n < 0 { debit -= n } else { credit += n }
        }
        (debit.to_string(), credit.to_string())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.export_id.is_nil()
                && !self.source_batch_id.is_nil()
                && !self.core_operation_id.is_nil()
                && !self.chart_id.is_nil()
                && self.chart_version > 0,
            "Stable GL source and export identities required"
        );
        ensure!(
            (2..=16).contains(&self.entries.len())
                && valid_code(&self.posting_kind)
                && self.business_date <= Utc::now().date_naive(),
            "Bounded dated GL batch required"
        );
        ensure!(
            [&self.chart_digest, &self.source_digest]
                .iter()
                .all(|s| s.len() == 64
                    && s.bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())),
            "Exact source and chart digests required"
        );
        let mut accounts = BTreeSet::new();
        for e in &self.entries {
            ensure!(
                !e.core_account_id.is_nil()
                    && accounts.insert(e.core_account_id)
                    && valid_code(&e.gl_account)
                    && e.delta.currency() == Currency::synthetic()
                    && e.delta.minor_units() != 0,
                "Valid exact GL leg required"
            );
        }
        let (debit, credit) = Self::totals(&self.entries);
        ensure!(
            debit == credit
                && debit != "0"
                && debit == self.debit_total
                && credit == self.credit_total,
            "Balanced exact GL control totals required"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GlStatus {
    Booked,
    NotFound,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlEvidence {
    pub evidence_id: Uuid,
    pub provider: String,
    pub export_id: Uuid,
    pub source_batch_id: Uuid,
    pub input_digest: String,
    pub status: GlStatus,
    pub provider_journal_reference: Option<String>,
    pub business_date: NaiveDate,
    pub debit_total: String,
    pub credit_total: String,
    pub occurred_at: DateTime<Utc>,
    pub version: i32,
}
impl GlEvidence {
    pub fn validate(&self, batch: &GlBatch) -> Result<()> {
        batch.validate()?;
        ensure!(
            !self.evidence_id.is_nil()
                && self.provider == "SYNTHETIC_GL"
                && self.export_id == batch.export_id
                && self.source_batch_id == batch.source_batch_id
                && self.input_digest == fingerprint(batch)?
                && self.business_date == batch.business_date
                && self.debit_total == batch.debit_total
                && self.credit_total == batch.credit_total
                && self.version == 1
                && self.occurred_at <= Utc::now() + chrono::Duration::seconds(5),
            "GL evidence scope mismatch"
        );
        ensure!(
            match self.status {
                GlStatus::Booked => self
                    .provider_journal_reference
                    .as_ref()
                    .is_some_and(|s| s == &format!("SIMGL-{}", batch.source_batch_id.simple())),
                GlStatus::NotFound => self.provider_journal_reference.is_none(),
            },
            "Definitive GL booking reference required"
        );
        Ok(())
    }
}
pub struct VerifiedGl {
    pub document: SignedDocument,
    pub evidence: GlEvidence,
}
impl VerifiedGl {
    pub fn verify(packet: SignedDocument, batch: &GlBatch) -> Result<Self> {
        packet.verify(signing_key().as_bytes())?;
        let evidence: GlEvidence = serde_json::from_value(packet.body.clone())?;
        evidence.validate(batch)?;
        Ok(Self {
            document: packet,
            evidence,
        })
    }
}
