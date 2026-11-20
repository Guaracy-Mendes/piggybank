use axum::http::StatusCode;
use bank_platform::domain::{ActorContext, TrustContext};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    Customer,
    Workforce,
    Service,
    /// WP-33 registered partner applications (client credentials). Never a browser login trust.
    Partner,
}
impl Trust {
    pub fn parse(value: &str) -> Result<Self, StatusCode> {
        match value {
            "customer" => Ok(Self::Customer),
            "workforce" => Ok(Self::Workforce),
            "service" => Ok(Self::Service),
            // "partner" is deliberately not parseable: partners have no interactive login or session.
            _ => Err(StatusCode::NOT_FOUND),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Customer => "customer",
            Self::Workforce => "workforce",
            Self::Service => "service",
            Self::Partner => "partner",
        }
    }
    pub fn realm(self) -> &'static str {
        match self {
            Self::Customer => "customers",
            Self::Partner => "partners",
            Self::Workforce | Self::Service => "workforce",
        }
    }
    pub fn audience(self) -> &'static str {
        match self {
            Self::Customer => "bank-customer",
            Self::Workforce => "bank-workforce",
            Self::Service => "bank-service",
            Self::Partner => "bank-partner",
        }
    }
    /// The fixed first-party client. Partner clients are dynamic and checked against the registry instead.
    pub fn client(self) -> &'static str {
        match self {
            Self::Customer => "customer-web",
            Self::Workforce => "staff-web",
            Self::Service => "bank-worker",
            Self::Partner => "",
        }
    }
    pub fn client_secret(self) -> String {
        std::env::var(format!(
            "BANK_{}_CLIENT_SECRET",
            self.name().to_ascii_uppercase()
        ))
        .unwrap_or_else(|_| format!("synthetic-{}-client-only", self.name()))
    }
    pub fn context(self) -> TrustContext {
        match self {
            Self::Customer => TrustContext::Customer,
            Self::Workforce => TrustContext::Workforce,
            Self::Service => TrustContext::Service,
            Self::Partner => TrustContext::Partner,
        }
    }
}

#[derive(Clone)]
pub struct Verifier {
    pub http: reqwest::Client,
    pub issuer_base: String,
    cache: Arc<RwLock<HashMap<String, (Instant, JwkSet)>>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Roles {
    #[serde(default)]
    pub roles: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub exp: u64,
    pub iat: u64,
    #[serde(default)]
    pub azp: String,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub auth_time: Option<i64>,
    #[serde(default)]
    pub realm_access: Option<Roles>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub actor: ActorContext,
    pub expires_at: u64,
}

impl Verifier {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            issuer_base: std::env::var("BANK_ISSUER_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:18280".into()),
            cache: Default::default(),
        })
    }
    pub fn issuer(&self, trust: Trust) -> String {
        format!("{}/realms/{}", self.issuer_base, trust.realm())
    }
    async fn keys(&self, trust: Trust, kid: &str) -> Result<JwkSet, StatusCode> {
        let issuer = self.issuer(trust);
        if let Some((created, keys)) = self.cache.read().await.get(&issuer)
            && created.elapsed() < Duration::from_secs(30)
            && (keys.find(kid).is_some() || created.elapsed() < Duration::from_secs(2))
        {
            return Ok(keys.clone());
        }
        // The key endpoint derives only from configured issuer, never token jku/x5u.
        let keys: JwkSet = self
            .http
            .get(format!("{issuer}/protocol/openid-connect/certs"))
            .send()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .error_for_status()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .json()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        self.cache
            .write()
            .await
            .insert(issuer, (Instant::now(), keys.clone()));
        Ok(keys)
    }
    pub async fn claims(
        &self,
        token: &str,
        trust: Trust,
        audience: &str,
    ) -> Result<Claims, StatusCode> {
        if token.len() > 16_384 {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let header = decode_header(token).map_err(|_| StatusCode::UNAUTHORIZED)?;
        if header.alg != Algorithm::RS256 {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let kid = header.kid.ok_or(StatusCode::UNAUTHORIZED)?;
        let keys = self.keys(trust, &kid).await?;
        let jwk = keys.find(&kid).ok_or(StatusCode::UNAUTHORIZED)?;
        let key = DecodingKey::from_jwk(jwk).map_err(|_| StatusCode::UNAUTHORIZED)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[self.issuer(trust)]);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["exp", "iat", "iss", "sub", "aud"]);
        validation.validate_nbf = true;
        validation.leeway = 2;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| StatusCode::UNAUTHORIZED)?
            .claims;
        if claims.sub.is_empty() || claims.iat > chrono::Utc::now().timestamp() as u64 + 2 {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(claims)
    }
    pub async fn verify(&self, token: &str, trust: Trust) -> Result<Identity, StatusCode> {
        let claims = self.claims(token, trust, trust.audience()).await?;
        // Partner tokens name a registered client in `azp`; the caller must confirm it is ACTIVE in the registry.
        let allowed = (trust == Trust::Partner && claims.azp.starts_with("partner-"))
            || (trust != Trust::Partner && claims.azp == trust.client())
            || (trust == Trust::Customer && claims.azp == "bank-mobile");
        if !allowed || claims.exp.saturating_sub(claims.iat) > 60 {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(Identity {
            expires_at: claims.exp,
            actor: ActorContext {
                issuer: claims.iss,
                subject: claims.sub,
                trust: trust.context(),
                client_id: claims.azp,
                person_id: None,
                represented_party: None,
                roles: claims.realm_access.map(|r| r.roles).unwrap_or_default(),
            },
        })
    }
}
