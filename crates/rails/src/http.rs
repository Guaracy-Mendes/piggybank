use crate::{RailInstruction, RecipientEvidence, VerifiedEvidence, signing_key};
use anyhow::{Result, ensure};
use bank_platform::authenticity::SignedDocument;
use std::time::Duration;
#[derive(Clone)]
pub struct RailClient {
    base: String,
    key: String,
    http: reqwest::Client,
}
impl RailClient {
    pub fn local() -> Result<Self> {
        Self::new(
            &std::env::var("BANK_RAIL_PROVIDER_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:4182".into()),
        )
    }
    pub fn new(base: &str) -> Result<Self> {
        let url = url::Url::parse(base)?;
        ensure!(
            url.scheme() == "http"
                && matches!(
                    url.host_str(),
                    Some("127.0.0.1" | "[::1]" | "::1" | "localhost")
                )
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && matches!(url.path(), "" | "/"),
            "Simulated rail adapter requires a loopback base URL"
        );
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            key: signing_key(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_millis(1500))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    pub async fn verify_recipient(&self, reference: &str) -> Result<RecipientEvidence> {
        ensure!(
            crate::valid_recipient_reference(reference),
            "Malformed simulated recipient"
        );
        let response = self
            .http
            .get(format!("{}/rails/recipients/{reference}", self.base))
            .header("X-Rail-Key", &self.key)
            .send()
            .await?;
        let document = bounded(response).await?;
        document.verify(self.key.as_bytes())?;
        let evidence: RecipientEvidence = serde_json::from_value(document.body)?;
        evidence.validate(reference)?;
        Ok(evidence)
    }
    pub async fn submit(&self, instruction: &RailInstruction) -> Result<VerifiedEvidence> {
        instruction.validate()?;
        let packet = SignedDocument::sign(self.key.as_bytes(), instruction)?;
        let response = self
            .http
            .post(format!("{}/rails/instructions", self.base))
            .json(&packet)
            .send()
            .await?;
        VerifiedEvidence::verify(bounded(response).await?, &self.key, instruction)
    }
    pub async fn cancel(&self, instruction: &RailInstruction) -> Result<VerifiedEvidence> {
        instruction.validate()?;
        let packet = SignedDocument::sign(self.key.as_bytes(), instruction)?;
        let response = self
            .http
            .post(format!("{}/rails/cancellation", self.base))
            .json(&packet)
            .send()
            .await?;
        VerifiedEvidence::verify(bounded(response).await?, &self.key, instruction)
    }
    pub async fn inquire(&self, instruction: &RailInstruction) -> Result<VerifiedEvidence> {
        let packet = SignedDocument::sign(self.key.as_bytes(), instruction)?;
        let response = self
            .http
            .post(format!("{}/rails/inquiry", self.base))
            .json(&packet)
            .send()
            .await?;
        VerifiedEvidence::verify(bounded(response).await?, &self.key, instruction)
    }
}
async fn bounded(mut response: reqwest::Response) -> Result<SignedDocument> {
    ensure!(
        response.status().is_success(),
        "Rail transport status did not authenticate a financial outcome"
    );
    let mut bytes = vec![];
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 65536,
            "Rail response bound exceeded"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
