use crate::protocol::{GlBatch, VerifiedGl, signing_key};
use anyhow::{Result, ensure};
use bank_platform::authenticity::SignedDocument;
use std::time::Duration;
pub(crate) struct EvidenceConflict {
    pub document: serde_json::Value,
}
impl std::fmt::Debug for EvidenceConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GL evidence conflict")
    }
}
impl std::fmt::Display for EvidenceConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GL evidence conflict")
    }
}
impl std::error::Error for EvidenceConflict {}
#[derive(Clone)]
pub struct GlClient {
    pub(crate) base: String,
    pub(crate) http: reqwest::Client,
}
impl GlClient {
    pub fn local() -> Result<Self> {
        Self::new(
            &std::env::var("BANK_FINANCE_PROVIDER_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:4182".into()),
        )
    }
    pub fn new(base: &str) -> Result<Self> {
        let u = url::Url::parse(base)?;
        ensure!(
            u.scheme() == "http"
                && matches!(
                    u.host_str(),
                    Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
                )
                && u.username().is_empty()
                && u.password().is_none()
                && u.query().is_none()
                && u.fragment().is_none()
                && matches!(u.path(), "" | "/"),
            "Synthetic GL requires a loopback base URL"
        );
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_millis(1500))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    pub async fn book(&self, batch: &GlBatch) -> Result<VerifiedGl> {
        self.call("/gl/batches", batch).await
    }
    pub async fn inquire(&self, batch: &GlBatch) -> Result<VerifiedGl> {
        self.call("/gl/inquiry", batch).await
    }
    async fn call(&self, path: &str, batch: &GlBatch) -> Result<VerifiedGl> {
        batch.validate()?;
        let packet = SignedDocument::sign(signing_key().as_bytes(), batch)?;
        let mut r = self
            .http
            .post(format!("{}{path}", self.base))
            .json(&packet)
            .send()
            .await?;
        if r.status() == reqwest::StatusCode::CONFLICT {
            return Err(EvidenceConflict {
                document: serde_json::json!({"status":409}),
            }
            .into());
        }
        ensure!(
            r.status().is_success(),
            "GL transport did not authenticate booking"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = r.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 65536,
                "GL response bound exceeded"
            );
            bytes.extend_from_slice(&chunk);
        }
        let document:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|EvidenceConflict{document:serde_json::json!({"invalidBodyDigest":bank_platform::domain::fingerprint(&bytes).unwrap_or_default()})})?;
        let verified = (|| VerifiedGl::verify(serde_json::from_value(document.clone())?, batch))();
        verified.map_err(|_| EvidenceConflict { document }.into())
    }
}
