//! Independent synthetic clearing files. Bank fees and provider fees have distinct meanings.
use anyhow::{Result, ensure};
use bank_core::finance_snapshot::ReportRange;
use bank_platform::authenticity::SignedDocument;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// Read-only matching view; grouping does not invent a signed aggregate file.
pub(crate) struct ClearingRows<'a, R> {
    pub rows: &'a [R],
    pub range: ReportRange,
    pub generated_at: DateTime<Utc>,
}
#[derive(Debug)]
pub struct SettlementUnavailable;
impl std::fmt::Display for SettlementUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Authenticated settlement file unavailable")
    }
}
impl std::error::Error for SettlementUnavailable {}
pub const PROVIDER: &str = "SYNTHETIC_RAIL_SETTLEMENT";
pub const FEE_POLICY: &str = "SYNTHETIC_PROVIDER_ZERO_FEE_V1";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementRow {
    pub line_id: Uuid,
    pub payment_id: Uuid,
    pub evidence_id: Uuid,
    pub provider_version: i64,
    pub principal_minor: String,
    pub provider_fee_minor: String,
    pub currency_code: String,
    pub currency_scale: u8,
    pub business_date: NaiveDate,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementFile {
    pub file_id: Uuid,
    pub format_version: i32,
    pub provider: String,
    pub fee_policy: String,
    pub range: ReportRange,
    pub generated_at: DateTime<Utc>,
    pub rows: Vec<SettlementRow>,
}
impl SettlementFile {
    /// Structural bounds only: authentic duplicate/currency/value/date differences remain classifiable.
    pub fn validate(&self) -> Result<()> {
        self.range.validate()?;
        ensure!(
            !self.file_id.is_nil()
                && self.format_version == 1
                && self.provider == PROVIDER
                && self.fee_policy == FEE_POLICY
                && self.generated_at <= Utc::now() + chrono::Duration::seconds(5)
                && self.rows.len() <= 100,
            "Invalid bounded synthetic settlement file"
        );
        for row in &self.rows {
            ensure!(
                !row.line_id.is_nil()
                    && !row.payment_id.is_nil()
                    && !row.evidence_id.is_nil()
                    && row.provider_version > 0
                    && row.currency_code.len() == 3
                    && row.currency_code.bytes().all(|b| b.is_ascii_uppercase())
                    && row.currency_scale <= 18,
                "Invalid settlement row structure"
            );
            for amount in [&row.principal_minor, &row.provider_fee_minor] {
                ensure!(
                    amount
                        .parse::<i64>()
                        .is_ok_and(|n| n.to_string() == *amount),
                    "Canonical signed i64 settlement amount required"
                );
            }
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= 60000,
            "Settlement file byte bound exceeded"
        );
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementQuery {
    pub kind: String,
    pub file_id: Uuid,
    pub request_id: Uuid,
}
impl SettlementQuery {
    pub fn new(file_id: Uuid) -> Self {
        Self {
            kind: "SYNTHETIC_OUTGOING_RECONCILIATION_V1".into(),
            file_id,
            request_id: Uuid::new_v4(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.kind == "SYNTHETIC_OUTGOING_RECONCILIATION_V1"
                && !self.file_id.is_nil()
                && !self.request_id.is_nil(),
            "Invalid settlement inquiry scope"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementReply {
    pub request: SettlementQuery,
    pub file: SettlementFile,
}
impl SettlementReply {
    pub fn validate(&self) -> Result<()> {
        self.request.validate()?;
        self.file.validate()?;
        ensure!(
            self.request.file_id == self.file.file_id,
            "Settlement file identity mismatch"
        );
        Ok(())
    }
}
impl crate::client::GlClient {
    pub async fn settlement_file(&self, id: Uuid) -> Result<(SettlementFile, SignedDocument)> {
        self.fetch_settlement_file(id)
            .await
            .map_err(|_| SettlementUnavailable.into())
    }
    async fn fetch_settlement_file(&self, id: Uuid) -> Result<(SettlementFile, SignedDocument)> {
        let request = SettlementQuery::new(id);
        request.validate()?;
        let packet = SignedDocument::sign(crate::protocol::signing_key().as_bytes(), &request)?;
        let mut response = self
            .http
            .post(format!("{}/settlement-files/inquiry", self.base))
            .json(&packet)
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 65536,
                "Settlement response byte bound exceeded"
            );
            bytes.extend_from_slice(&chunk);
        }
        let packet: SignedDocument = serde_json::from_slice(&bytes)?;
        packet.verify(crate::protocol::signing_key().as_bytes())?;
        let reply: SettlementReply = serde_json::from_value(packet.body.clone())?;
        reply.validate()?;
        ensure!(
            reply.request == request,
            "Settlement inquiry response binding mismatch"
        );
        Ok((reply.file, packet))
    }
}
