//! WP-38 journey-partner channel codec for the single local simulator role (`SIM_JOURNEY`). Requests and responses are
//! HMAC-SHA256 signed with the partner key; a selected partner (DEC-05/06/09) brings its own managed key and mTLS.
use ring::hmac;

/// The only partner with a key in the local profile.
pub const LOCAL_PARTNER: &str = "SIM_JOURNEY";

fn key(partner: &str) -> Option<hmac::Key> {
    if partner != LOCAL_PARTNER {
        return None;
    }
    let secret = std::env::var("BANK_JOURNEY_PARTNER_KEY")
        .unwrap_or_else(|_| "synthetic-journey-partner-key-local-only".into());
    Some(hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes()))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// Signs a response (or, in tests, a request) for `partner`. Unknown partners have no key.
pub fn sign(partner: &str, payload: &[u8]) -> Option<String> {
    key(partner).map(|k| hex(hmac::sign(&k, payload).as_ref()))
}
/// Constant-time verification of a hex signature over `payload`.
pub fn verify(partner: &str, payload: &[u8], signature: &str) -> bool {
    let Some(k) = key(partner) else {
        return false;
    };
    let Some(raw) = (0..signature.len())
        .step_by(2)
        .map(|i| {
            signature
                .get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect::<Option<Vec<u8>>>()
    else {
        return false;
    };
    hmac::verify(&k, payload, &raw).is_ok()
}
/// The signed string for an export fetch (a GET has no body).
pub fn export_request(partner: &str, export: &str) -> Vec<u8> {
    format!("GET\n/channels/journeys/v1/exports/{export}\n{partner}").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_bind_partner_and_payload() {
        let s = sign(LOCAL_PARTNER, b"body").unwrap();
        assert!(verify(LOCAL_PARTNER, b"body", &s));
        assert!(!verify(LOCAL_PARTNER, b"other", &s));
        assert!(!verify("OTHER_PARTNER", b"body", &s));
        assert!(sign("OTHER_PARTNER", b"body").is_none());
        assert!(!verify(LOCAL_PARTNER, b"body", "zz"));
    }
}
