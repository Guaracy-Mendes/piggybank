//! Closed immutable provider membership for bounded multi-file clearing snapshots.
use crate::{
    credit_settlement::CreditSettlementFile,
    settlement::{FEE_POLICY, PROVIDER, SettlementFile, SettlementUnavailable},
};
use anyhow::{Result, ensure};
use bank_core::finance_snapshot::ReportRange;
use bank_platform::{authenticity::SignedDocument, domain::fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SetKind {
    Outgoing,
    Credits,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishSet {
    pub set_id: Uuid,
    pub kind: SetKind,
    pub range: ReportRange,
    pub generated_at: DateTime<Utc>,
    pub file_ids: Vec<Uuid>,
}
impl PublishSet {
    pub fn validate(&self) -> Result<()> {
        self.range.validate()?;
        ensure!(
            !self.set_id.is_nil()
                && (1..=10).contains(&self.file_ids.len())
                && self.generated_at <= Utc::now() + chrono::Duration::seconds(5),
            "Invalid bounded settlement set"
        );
        ensure!(
            self.file_ids.iter().all(|id| !id.is_nil())
                && self
                    .file_ids
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == self.file_ids.len(),
            "Unique non-nil member file identities required"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetMember {
    pub file_id: Uuid,
    pub digest: String,
    pub row_count: i32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementSet {
    pub set_id: Uuid,
    pub format_version: i32,
    pub kind: SetKind,
    pub provider: String,
    pub fee_policy: String,
    pub range: ReportRange,
    pub generated_at: DateTime<Utc>,
    pub members: Vec<SetMember>,
    pub row_count: i32,
}
impl SettlementSet {
    pub fn validate(&self) -> Result<()> {
        PublishSet {
            set_id: self.set_id,
            kind: self.kind,
            range: self.range,
            generated_at: self.generated_at,
            file_ids: self.members.iter().map(|m| m.file_id).collect(),
        }
        .validate()?;
        ensure!(
            self.format_version == 1
                && self.provider == PROVIDER
                && self.fee_policy == FEE_POLICY
                && (0..=1000).contains(&self.row_count),
            "Unsupported settlement set scope"
        );
        ensure!(
            self.members.windows(2).all(|w| w[0].file_id < w[1].file_id)
                && self.members.iter().all(|m| (0..=100).contains(&m.row_count)
                    && m.digest.len() == 64
                    && m.digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
                && self.members.iter().map(|m| m.row_count).sum::<i32>() == self.row_count,
            "Exact canonical closed membership required"
        );
        Ok(())
    }
    pub fn verify_member(&self, index: usize, file: &CapturedFile) -> Result<()> {
        let member = self
            .members
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Unexpected settlement set member"))?;
        let (id, range, cutoff, count, digest) = match file {
            CapturedFile::Outgoing(f) => {
                f.validate()?;
                ensure!(
                    self.kind == SetKind::Outgoing,
                    "Settlement set namespace mismatch"
                );
                (
                    f.file_id,
                    f.range,
                    f.generated_at,
                    f.rows.len(),
                    fingerprint(f)?,
                )
            }
            CapturedFile::Credits(f) => {
                f.validate()?;
                ensure!(
                    self.kind == SetKind::Credits,
                    "Settlement set namespace mismatch"
                );
                (
                    f.file_id,
                    f.range,
                    f.generated_at,
                    f.rows.len(),
                    fingerprint(f)?,
                )
            }
        };
        ensure!(
            member.file_id == id
                && member.digest == digest
                && member.row_count == count as i32
                && range == self.range
                && cutoff == self.generated_at,
            "Settlement set membership, period or cutoff mismatch"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "file",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum CapturedFile {
    Outgoing(SettlementFile),
    Credits(CreditSettlementFile),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetQuery {
    pub kind: String,
    pub set_id: Uuid,
    pub request_id: Uuid,
}
impl SetQuery {
    pub fn new(set_id: Uuid) -> Self {
        Self {
            kind: "SYNTHETIC_SETTLEMENT_SET_RECONCILIATION_V1".into(),
            set_id,
            request_id: Uuid::new_v4(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.kind == "SYNTHETIC_SETTLEMENT_SET_RECONCILIATION_V1"
                && !self.set_id.is_nil()
                && !self.request_id.is_nil(),
            "Invalid settlement set inquiry"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetReply {
    pub request: SetQuery,
    pub manifest: SettlementSet,
}
impl SetReply {
    pub fn validate(&self) -> Result<()> {
        self.request.validate()?;
        self.manifest.validate()?;
        ensure!(
            self.request.set_id == self.manifest.set_id,
            "Settlement set response identity mismatch"
        );
        Ok(())
    }
}
impl crate::client::GlClient {
    pub async fn settlement_set(&self, id: Uuid) -> Result<(SettlementSet, SignedDocument)> {
        async {
            let query = SetQuery::new(id);
            query.validate()?;
            let request = SignedDocument::sign(crate::protocol::signing_key().as_bytes(), &query)?;
            let mut response = self
                .http
                .post(format!("{}/settlement-sets/inquiry", self.base))
                .json(&request)
                .send()
                .await?
                .error_for_status()?;
            let mut body = vec![];
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    body.len() + chunk.len() <= 65536,
                    "Settlement set response bound exceeded"
                );
                body.extend_from_slice(&chunk);
            }
            let packet: SignedDocument = serde_json::from_slice(&body)?;
            packet.verify(crate::protocol::signing_key().as_bytes())?;
            let reply: SetReply = serde_json::from_value(packet.body.clone())?;
            reply.validate()?;
            ensure!(
                reply.request == query,
                "Settlement set inquiry response binding mismatch"
            );
            Ok((reply.manifest, packet))
        }
        .await
        .map_err(|_: anyhow::Error| SettlementUnavailable.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn set_http_boundary_rejects_forged_nonce_bad_auth_membership_and_unbounded_responses()
    -> Result<()> {
        for scenario in [
            "valid",
            "wrong_nonce",
            "bad_signature",
            "bad_membership",
            "oversized",
        ] {
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
                            header.starts_with("POST /settlement-sets/inquiry "),
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
                let mut request: SetQuery = serde_json::from_value(signed.body)?;
                request.validate()?;
                let range = ReportRange {
                    from_date: Utc::now().date_naive(),
                    to_date: Utc::now().date_naive() + chrono::Duration::days(1),
                };
                if scenario == "wrong_nonce" {
                    request.request_id = Uuid::new_v4();
                }
                let mut reply = SetReply {
                    request,
                    manifest: SettlementSet {
                        set_id: id,
                        kind: SetKind::Outgoing,
                        format_version: 1,
                        provider: PROVIDER.into(),
                        fee_policy: FEE_POLICY.into(),
                        range,
                        generated_at: Utc::now(),
                        members: vec![SetMember {
                            file_id: Uuid::new_v4(),
                            digest: "0".repeat(64),
                            row_count: 0,
                        }],
                        row_count: 0,
                    },
                };
                if scenario == "bad_membership" {
                    reply.manifest.row_count = 1;
                }
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
                .settlement_set(id)
                .await;
            if scenario == "valid" {
                assert_eq!(response?.0.set_id, id);
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
