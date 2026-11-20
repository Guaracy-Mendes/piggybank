//! WP-28 (SEC-02, resolves SF-001): server-held session tokens are sealed with AES-256-GCM before storage.
//! The key ring comes from `BANK_SESSION_KEYS_FILE` (one `kid:base64key` per line; the first line seals, every
//! line opens), re-read on use so rotation needs no restart. Each ciphertext is bound to its row and column via
//! associated data. Without a key file the local synthetic profile uses a fixed, documented synthetic key.
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use sha2::{Digest, Sha256};

pub struct KeyRing {
    keys: Vec<(String, [u8; 32])>,
}
impl KeyRing {
    pub fn load() -> Result<Self, String> {
        let mut keys = Vec::new();
        if let Ok(path) = std::env::var("BANK_SESSION_KEYS_FILE") {
            let text = std::fs::read_to_string(&path)
                .map_err(|_| "session key file unreadable".to_owned())?;
            for line in text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
            {
                let (kid, b64) = line.split_once(':').ok_or("malformed session key line")?;
                let raw = STANDARD.decode(b64).map_err(|_| "malformed session key")?;
                let key: [u8; 32] = raw
                    .try_into()
                    .map_err(|_| "session keys must be 32 bytes")?;
                if kid.is_empty() || kid.contains(':') {
                    return Err("invalid session key id".into());
                }
                keys.push((kid.to_owned(), key));
            }
            if keys.is_empty() {
                return Err("session key file has no keys".into());
            }
        } else {
            keys.push((
                "synthetic-v1".into(),
                Sha256::digest(b"piggybank-synthetic-local-session-key").into(),
            ));
        }
        Ok(Self { keys })
    }
    fn key(raw: &[u8; 32]) -> LessSafeKey {
        LessSafeKey::new(UnboundKey::new(&AES_256_GCM, raw).expect("32-byte AES-256 key"))
    }
    pub fn seal(&self, plain: &str, aad: &str) -> String {
        let (kid, raw) = &self.keys[0];
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).expect("operating system entropy unavailable");
        let mut data = plain.as_bytes().to_vec();
        Self::key(raw)
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad.as_bytes()),
                &mut data,
            )
            .expect("AES-GCM seal");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&data);
        format!("v1:{kid}:{}", STANDARD.encode(out))
    }
    /// Legacy plaintext rows (written before WP-28) are accepted and re-sealed on their next refresh.
    pub fn open(&self, stored: &str, aad: &str) -> Option<String> {
        let Some(rest) = stored.strip_prefix("v1:") else {
            return Some(stored.to_owned());
        };
        let (kid, b64) = rest.split_once(':')?;
        let raw = self.keys.iter().find(|(k, _)| k == kid)?.1;
        let mut data = STANDARD.decode(b64).ok()?;
        if data.len() < NONCE_LEN {
            return None;
        }
        let body = data.split_off(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(&data).ok()?;
        let mut body = body;
        let plain = Self::key(&raw)
            .open_in_place(nonce, Aad::from(aad.as_bytes()), &mut body)
            .ok()?;
        String::from_utf8(plain.to_vec()).ok()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sealed_tokens_bind_to_their_row_and_rotate() {
        let ring = KeyRing::load().unwrap();
        let sealed = ring.seal("token-value", "row:access");
        assert!(sealed.starts_with("v1:synthetic-v1:") && !sealed.contains("token-value"));
        assert_eq!(
            ring.open(&sealed, "row:access").as_deref(),
            Some("token-value")
        );
        assert_eq!(ring.open(&sealed, "other:access"), None);
        assert_eq!(ring.open("legacy", "row:access").as_deref(), Some("legacy"));
        let old = KeyRing {
            keys: vec![("old".into(), [7; 32])],
        };
        let rotated = KeyRing {
            keys: vec![("new".into(), [9; 32]), ("old".into(), [7; 32])],
        };
        let before = old.seal("t", "a");
        assert_eq!(rotated.open(&before, "a").as_deref(), Some("t"));
        assert!(rotated.seal("t", "a").starts_with("v1:new:"));
        assert_eq!(
            KeyRing {
                keys: vec![("new".into(), [9; 32])]
            }
            .open(&before, "a"),
            None
        );
    }
}
