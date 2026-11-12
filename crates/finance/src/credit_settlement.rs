//! Independent incoming and return clearing files; distinct from the outgoing V1 protocol.
pub use crate::settlement::{FEE_POLICY, PROVIDER, SettlementUnavailable};
use anyhow::{Result, ensure};
use bank_core::finance_snapshot::ReportRange;
use bank_platform::authenticity::SignedDocument;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditSettlementRow {
    pub line_id: Uuid,
    pub movement_id: Uuid,
    pub event_type: bank_rails::CreditKind,
    pub original_payment_id: Option<Uuid>,
    pub receiving_reference: Option<String>,
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
pub struct CreditSettlementFile {
    pub file_id: Uuid,
    pub format_version: i32,
    pub provider: String,
    pub fee_policy: String,
    pub range: ReportRange,
    pub generated_at: DateTime<Utc>,
    pub rows: Vec<CreditSettlementRow>,
}
impl CreditSettlementFile {
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
                    && !row.movement_id.is_nil()
                    && !row.evidence_id.is_nil()
                    && row.original_payment_id.is_none_or(|id| !id.is_nil())
                    && row
                        .receiving_reference
                        .as_ref()
                        .is_none_or(|r| r.len() <= 80 && r.is_ascii())
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
pub struct CreditSettlementQuery {
    pub kind: String,
    pub file_id: Uuid,
    pub request_id: Uuid,
}
impl CreditSettlementQuery {
    pub fn new(file_id: Uuid) -> Self {
        Self {
            kind: "SYNTHETIC_CREDIT_RECONCILIATION_V1".into(),
            file_id,
            request_id: Uuid::new_v4(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.kind == "SYNTHETIC_CREDIT_RECONCILIATION_V1"
                && !self.file_id.is_nil()
                && !self.request_id.is_nil(),
            "Invalid settlement inquiry scope"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditSettlementReply {
    pub request: CreditSettlementQuery,
    pub file: CreditSettlementFile,
}
impl CreditSettlementReply {
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
    pub async fn credit_settlement_file(
        &self,
        id: Uuid,
    ) -> Result<(CreditSettlementFile, SignedDocument)> {
        self.fetch_credit_settlement_file(id)
            .await
            .map_err(|_| SettlementUnavailable.into())
    }
    async fn fetch_credit_settlement_file(
        &self,
        id: Uuid,
    ) -> Result<(CreditSettlementFile, SignedDocument)> {
        let request = CreditSettlementQuery::new(id);
        request.validate()?;
        let packet = SignedDocument::sign(crate::protocol::signing_key().as_bytes(), &request)?;
        let mut response = self
            .http
            .post(format!("{}/credit-settlement-files/inquiry", self.base))
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
        let reply: CreditSettlementReply = serde_json::from_value(packet.body.clone())?;
        reply.validate()?;
        ensure!(
            reply.request == request,
            "Settlement inquiry response binding mismatch"
        );
        Ok((reply.file, packet))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn credit_file_http_boundary_rejects_forged_nonce_bad_auth_and_unbounded_responses()
    -> Result<()> {
        for scenario in ["valid", "wrong_nonce", "bad_signature", "oversized"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let base = format!("http://{}", listener.local_addr()?);
            let id = Uuid::new_v4();
            let task = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await?;
                let mut bytes = vec![];
                let body = loop {
                    let mut chunk = [0u8; 4096];
                    let n = stream.read(&mut chunk).await?;
                    ensure!(
                        n > 0 && bytes.len() + n <= 65536,
                        "Bounded test request required"
                    );
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&bytes[..end])?;
                        ensure!(
                            header.starts_with("POST /credit-settlement-files/inquiry "),
                            "Credit-specific path required"
                        );
                        let len = header
                            .lines()
                            .find_map(|l| {
                                l.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + len {
                            break bytes[end + 4..end + 4 + len].to_vec();
                        }
                    }
                };
                let signed: SignedDocument = serde_json::from_slice(&body)?;
                signed.verify(crate::protocol::signing_key().as_bytes())?;
                let mut request: CreditSettlementQuery = serde_json::from_value(signed.body)?;
                request.validate()?;
                let range = ReportRange {
                    from_date: Utc::now().date_naive(),
                    to_date: Utc::now().date_naive() + chrono::Duration::days(1),
                };
                if scenario == "wrong_nonce" {
                    request.request_id = Uuid::new_v4();
                }
                let reply = CreditSettlementReply {
                    request,
                    file: CreditSettlementFile {
                        file_id: id,
                        format_version: 1,
                        provider: PROVIDER.into(),
                        fee_policy: FEE_POLICY.into(),
                        range,
                        generated_at: Utc::now(),
                        rows: vec![],
                    },
                };
                let key = if scenario == "bad_signature" {
                    "synthetic-untrusted-finance-key-2026".into()
                } else {
                    crate::protocol::signing_key()
                };
                let body = if scenario == "oversized" {
                    vec![b' '; 65537]
                } else {
                    serde_json::to_vec(&SignedDocument::sign(key.as_bytes(), &reply)?)?
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).await?;
                let _ = stream.write_all(&body).await;
                Ok::<_, anyhow::Error>(())
            });
            let response = crate::client::GlClient::new(&base)?
                .credit_settlement_file(id)
                .await;
            if scenario == "valid" {
                assert_eq!(response?.0.file_id, id);
            } else {
                assert!(
                    response.err().unwrap().is::<SettlementUnavailable>(),
                    "{scenario}"
                );
            }
            tokio::time::timeout(std::time::Duration::from_secs(3), task).await???;
        }
        Ok(())
    }
}
