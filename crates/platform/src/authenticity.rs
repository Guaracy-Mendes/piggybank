//! Shared synthetic-provider message authentication; keys come from runtime configuration.
use crate::domain::fingerprint;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedDocument {
    pub timestamp: i64,
    pub signature: String,
    pub body: serde_json::Value,
}
impl SignedDocument {
    pub fn sign<T: Serialize>(key: &[u8], body: &T) -> Result<Self> {
        ensure!(key.len() >= 32, "Provider signing key too short");
        let body = serde_json::to_value(body)?;
        let timestamp = chrono::Utc::now().timestamp();
        let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
        mac.update(format!("{timestamp}\n{}", fingerprint(&body)?).as_bytes());
        Ok(Self {
            timestamp,
            signature: URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()),
            body,
        })
    }
    pub fn verify(&self, key: &[u8]) -> Result<()> {
        ensure!(
            key.len() >= 32 && serde_json::to_vec(&self.body)?.len() <= 65536,
            "Invalid signed payload bound"
        );
        ensure!(
            chrono::Utc::now()
                .timestamp()
                .checked_sub(self.timestamp)
                .is_some_and(|delta| delta.unsigned_abs() <= 60),
            "Stale provider authentication timestamp"
        );
        self.verify_integrity(key)
    }
    /// Verify the integrity of an already authenticated immutable archive. Live ingress must use verify().
    pub fn verify_integrity(&self, key: &[u8]) -> Result<()> {
        ensure!(
            key.len() >= 32 && serde_json::to_vec(&self.body)?.len() <= 65536,
            "Invalid signed archive bound"
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(key)?;
        mac.update(format!("{}\n{}", self.timestamp, fingerprint(&self.body)?).as_bytes());
        mac.verify_slice(&URL_SAFE_NO_PAD.decode(&self.signature)?)?;
        Ok(())
    }
}
