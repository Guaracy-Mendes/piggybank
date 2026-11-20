use crate::oidc::{Identity, Trust, Verifier};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct IdentityService {
    pub verifier: Verifier,
    pub pool: PgPool,
    pub public_base: String,
}
impl IdentityService {
    pub async fn begin_step_up(
        &self,
        trust: Trust,
        actor: &bank_platform::domain::ActorContext,
        challenge: uuid::Uuid,
        nonce: &str,
        digest: &str,
    ) -> Result<Response, StatusCode> {
        browser(trust)?;
        let mut response =
            login(State(Arc::new(self.clone())), Path(trust.name().to_owned())).await?;
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|s| s.to_str().ok())
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let mut url = url::Url::parse(location).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let state = pairs
            .iter()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.clone())
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        sqlx::query("UPDATE identity.login_attempts SET nonce=$2,challenge_id=$3,intent_digest=$4,expected_issuer=$5,expected_subject=$6 WHERE state_hash=$1")
            .bind(hash(&state)).bind(nonce).bind(challenge).bind(digest).bind(&actor.issuer).bind(&actor.subject).execute(&self.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        url.set_query(None);
        for (key, value) in pairs {
            url.query_pairs_mut()
                .append_pair(&key, if key == "nonce" { nonce } else { &value });
        }
        url.query_pairs_mut()
            .append_pair("max_age", "0")
            .append_pair("prompt", "login");
        response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_str(url.as_str()).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        );
        Ok(response)
    }
    /// WP-27: start native action-bound reauthentication for a pending challenge owned by this mobile actor.
    pub async fn native_step_up_begin(
        &self,
        actor: &bank_platform::domain::ActorContext,
        challenge: uuid::Uuid,
        nonce: &str,
        digest: &str,
    ) -> Result<chrono::DateTime<chrono::Utc>, StatusCode> {
        if actor.trust != bank_platform::domain::TrustContext::Customer
            || actor.client_id != "bank-mobile"
        {
            return Err(StatusCode::FORBIDDEN);
        }
        // One timestamp for both columns: two clock_timestamp() calls could differ by a microsecond and break the
        // 3-minute CHECK, which surfaced as an intermittent 503 (found by the WP-30 resilience drills).
        let row = sqlx::query("INSERT INTO identity.native_step_ups(challenge_id,issuer,subject,client_id,nonce,intent_digest,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,now(),now()+interval '3 minutes') ON CONFLICT(challenge_id) DO UPDATE SET created_at=EXCLUDED.created_at,expires_at=EXCLUDED.expires_at WHERE identity.native_step_ups.issuer=EXCLUDED.issuer AND identity.native_step_ups.subject=EXCLUDED.subject AND identity.native_step_ups.nonce=EXCLUDED.nonce AND identity.native_step_ups.intent_digest=EXCLUDED.intent_digest RETURNING expires_at")
            .bind(challenge).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(nonce).bind(digest)
            .fetch_optional(&self.pool).await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::CONFLICT)?;
        Ok(row.get("expires_at"))
    }
    /// WP-27: verify a fresh native ID token (audience bank-mobile, matching nonce/subject, auth_time after begin)
    /// and record the same single-use step-up proof the browser callback records.
    pub async fn native_step_up_complete(
        &self,
        actor: &bank_platform::domain::ActorContext,
        challenge: uuid::Uuid,
        id_token: &str,
    ) -> Result<(), StatusCode> {
        if actor.trust != bank_platform::domain::TrustContext::Customer
            || actor.client_id != "bank-mobile"
        {
            return Err(StatusCode::FORBIDDEN);
        }
        let row = sqlx::query("SELECT nonce,intent_digest,created_at FROM identity.native_step_ups WHERE challenge_id=$1 AND issuer=$2 AND subject=$3 AND expires_at>clock_timestamp()")
            .bind(challenge).bind(&actor.issuer).bind(&actor.subject)
            .fetch_optional(&self.pool).await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::NOT_FOUND)?;
        let claims = self
            .verifier
            .claims(id_token, Trust::Customer, "bank-mobile")
            .await?;
        let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
        let nonce: String = row.get("nonce");
        let fresh = claims.auth_time.is_some_and(|time| {
            time >= created.timestamp() - 1 && time <= chrono::Utc::now().timestamp() + 2
        });
        if claims.sub != actor.subject
            || claims.iss != actor.issuer
            || (!claims.azp.is_empty() && claims.azp != "bank-mobile")
            || claims
                .nonce
                .as_deref()
                .map(|n| n.as_bytes().ct_eq(nonce.as_bytes()).into())
                != Some(true)
            || !fresh
        {
            tracing::warn!(
                code = "NATIVE_STEP_UP_MISMATCH",
                fresh,
                "native step-up evidence rejected"
            );
            return Err(StatusCode::UNAUTHORIZED);
        }
        let digest: String = row.get("intent_digest");
        let authenticated = chrono::DateTime::<chrono::Utc>::from_timestamp(
            claims.auth_time.unwrap_or_default(),
            0,
        )
        .ok_or(StatusCode::UNAUTHORIZED)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        // A native-only customer has no browser callback; register the verified subject as the callback does.
        sqlx::query("INSERT INTO identity.subjects(issuer,subject,trust) VALUES($1,$2,'customer') ON CONFLICT(issuer,subject) DO NOTHING")
            .bind(&actor.issuer).bind(&actor.subject)
            .execute(&mut *tx).await.map_err(|e| { tracing::warn!(code = "NATIVE_STEP_UP_WRITE", error = %e); StatusCode::SERVICE_UNAVAILABLE })?;
        sqlx::query("INSERT INTO identity.step_up_proofs(id,challenge_id,issuer,subject,intent_digest,authenticated_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+interval '2 minutes') ON CONFLICT(challenge_id) DO NOTHING")
            .bind(uuid::Uuid::new_v4()).bind(challenge).bind(&actor.issuer).bind(&actor.subject).bind(&digest).bind(authenticated)
            .execute(&mut *tx).await.map_err(|e| { tracing::warn!(code = "NATIVE_STEP_UP_WRITE", error = %e); StatusCode::SERVICE_UNAVAILABLE })?;
        sqlx::query("DELETE FROM identity.native_step_ups WHERE challenge_id=$1")
            .bind(challenge)
            .execute(&mut *tx)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        sqlx::query("INSERT INTO identity.audit(id,issuer,subject,action) VALUES($1,$2,$3,'step-up.native')")
            .bind(uuid::Uuid::new_v4()).bind(&actor.issuer).bind(&actor.subject)
            .execute(&mut *tx).await.map_err(|e| { tracing::warn!(code = "NATIVE_STEP_UP_WRITE", error = %e); StatusCode::SERVICE_UNAVAILABLE })?;
        tx.commit()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Ok(())
    }
    pub async fn consume_step_up(
        &self,
        actor: &bank_platform::domain::ActorContext,
        challenge: uuid::Uuid,
        operation: uuid::Uuid,
        digest: &str,
    ) -> anyhow::Result<()> {
        let proof=sqlx::query("UPDATE identity.step_up_proofs SET use_operation_id=$5 WHERE challenge_id=$1 AND issuer=$2 AND subject=$3 AND intent_digest=$4 AND expires_at>now() AND (use_operation_id IS NULL OR use_operation_id=$5) RETURNING id")
            .bind(challenge).bind(&actor.issuer).bind(&actor.subject).bind(digest).bind(operation).fetch_optional(&self.pool).await?;
        anyhow::ensure!(
            proof.is_some(),
            "A current action-bound reauthentication proof is required"
        );
        Ok(())
    }
    pub async fn step_up_ready(
        &self,
        actor: &bank_platform::domain::ActorContext,
        challenge: uuid::Uuid,
        operation: uuid::Uuid,
        digest: &str,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM identity.step_up_proofs WHERE challenge_id=$1 AND issuer=$2 AND subject=$3 AND intent_digest=$4 AND (use_operation_id IS NULL OR use_operation_id=$5) AND expires_at>now())").bind(challenge).bind(&actor.issuer).bind(&actor.subject).bind(digest).bind(operation).fetch_one(&self.pool).await?)
    }
    pub async fn bound_step_up_until(
        &self,
        actor: &bank_platform::domain::ActorContext,
        challenge: uuid::Uuid,
        operation: uuid::Uuid,
        digest: &str,
    ) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
        Ok(sqlx::query_scalar("SELECT expires_at FROM identity.step_up_proofs WHERE challenge_id=$1 AND issuer=$2 AND subject=$3 AND intent_digest=$4 AND use_operation_id=$5 AND expires_at>now()")
            .bind(challenge).bind(&actor.issuer).bind(&actor.subject).bind(digest).bind(operation).fetch_one(&self.pool).await?)
    }
    pub async fn new() -> anyhow::Result<Self> {
        Ok(Self {
            verifier: Verifier::new()?,
            pool: bank_platform::database::Module::Identity.connect().await?,
            public_base: std::env::var("BANK_PUBLIC_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:4180".into()),
        })
    }
    pub async fn authenticate(
        &self,
        headers: &HeaderMap,
        trust: Trust,
        mutating: bool,
    ) -> Result<Identity, StatusCode> {
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|s| s.to_str().ok()) {
            let web_origin = std::env::var(format!(
                "BANK_{}_WEB_ORIGIN",
                trust.name().to_ascii_uppercase()
            ))
            .unwrap_or_else(|_| self.public_base.clone());
            if origin != self.public_base && origin != web_origin {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        if let Some(auth) = headers.get(header::AUTHORIZATION) {
            let token = auth
                .to_str()
                .ok()
                .and_then(|s| s.strip_prefix("Bearer "))
                .ok_or(StatusCode::UNAUTHORIZED)?;
            return self.verifier.verify(token, trust).await;
        }
        if trust == Trust::Service {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let session_id =
            cookie(headers, &format!("pb_{}", trust.name())).ok_or(StatusCode::UNAUTHORIZED)?;
        let id_hash = hash(&session_id);
        let row=sqlx::query("SELECT access_token,refresh_token,issuer,subject,csrf_hash FROM identity.sessions WHERE id_hash=$1 AND trust=$2 AND expires_at>now()")
            .bind(&id_hash).bind(trust.name()).fetch_optional(&self.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.ok_or(StatusCode::UNAUTHORIZED)?;
        if mutating {
            let token = headers
                .get("x-csrf-token")
                .and_then(|s| s.to_str().ok())
                .ok_or(StatusCode::FORBIDDEN)?;
            let csrf_cookie = cookie(headers, &format!("pb_csrf_{}", trust.name()))
                .ok_or(StatusCode::FORBIDDEN)?;
            let expected: String = row.get("csrf_hash");
            let origin = headers
                .get(header::ORIGIN)
                .and_then(|s| s.to_str().ok())
                .ok_or(StatusCode::FORBIDDEN)?;
            let web_origin = std::env::var(format!(
                "BANK_{}_WEB_ORIGIN",
                trust.name().to_ascii_uppercase()
            ))
            .unwrap_or_else(|_| self.public_base.clone());
            if !equal(token, &csrf_cookie)
                || !equal(&hash(token), &expected)
                || (origin != self.public_base && origin != web_origin)
            {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        let ring = crate::seal::KeyRing::load().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let access = ring
            .open(
                &row.get::<String, _>("access_token"),
                &format!("{id_hash}:access"),
            )
            .ok_or(StatusCode::UNAUTHORIZED)?;
        match self.verifier.verify(&access, trust).await {
            Ok(identity) => {
                if identity.actor.issuer != row.get::<String, _>("issuer")
                    || identity.actor.subject != row.get::<String, _>("subject")
                {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Ok(identity)
            }
            Err(StatusCode::UNAUTHORIZED) => self.refresh(&id_hash, trust, &row).await,
            Err(error) => Err(error),
        }
    }
    async fn refresh(
        &self,
        id_hash: &str,
        trust: Trust,
        row: &sqlx::postgres::PgRow,
    ) -> Result<Identity, StatusCode> {
        let lease = uuid::Uuid::new_v4();
        let claimed=sqlx::query("UPDATE identity.sessions SET refresh_lease=$2,refresh_until=now()+interval '10 seconds' WHERE id_hash=$1 AND (refresh_until IS NULL OR refresh_until<now()) RETURNING id_hash")
            .bind(id_hash).bind(lease).fetch_optional(&self.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
        if claimed.is_none() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let ring = crate::seal::KeyRing::load().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let refresh = ring
            .open(
                &row.get::<String, _>("refresh_token"),
                &format!("{id_hash}:refresh"),
            )
            .ok_or(StatusCode::UNAUTHORIZED)?;
        // No DB transaction is held during the external token call.
        let response = self
            .verifier
            .http
            .post(format!(
                "{}/protocol/openid-connect/token",
                self.verifier.issuer(trust)
            ))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", trust.client()),
                ("client_secret", trust.client_secret().as_str()),
            ])
            .send()
            .await;
        let result=async {
            let response=response.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
            if !response.status().is_success(){return Err(if response.status().is_client_error(){StatusCode::UNAUTHORIZED}else{StatusCode::SERVICE_UNAVAILABLE});}
            let tokens:Tokens=response.json().await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
            let identity=self.verifier.verify(&tokens.access_token,trust).await?;
            let issuer:String=row.get("issuer");let subject:String=row.get("subject");
            if identity.actor.issuer!=issuer || identity.actor.subject!=subject {return Err(StatusCode::UNAUTHORIZED);}
            let updated=sqlx::query("UPDATE identity.sessions SET access_token=$3,refresh_token=$4,refresh_lease=NULL,refresh_until=NULL WHERE id_hash=$1 AND refresh_lease=$2")
                .bind(id_hash).bind(lease).bind(ring.seal(&tokens.access_token,&format!("{id_hash}:access"))).bind(ring.seal(&tokens.refresh_token.unwrap_or(refresh),&format!("{id_hash}:refresh"))).execute(&self.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
            if updated.rows_affected()!=1 {return Err(StatusCode::UNAUTHORIZED);}
            Ok(identity)
        }.await;
        if result.is_err() {
            if matches!(result, Err(StatusCode::UNAUTHORIZED)) {
                let _ = sqlx::query(
                    "DELETE FROM identity.sessions WHERE id_hash=$1 AND refresh_lease=$2",
                )
                .bind(id_hash)
                .bind(lease)
                .execute(&self.pool)
                .await;
            } else {
                let _=sqlx::query("UPDATE identity.sessions SET refresh_lease=NULL,refresh_until=NULL WHERE id_hash=$1 AND refresh_lease=$2").bind(id_hash).bind(lease).execute(&self.pool).await;
            }
        }
        result
    }
}

fn random() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system entropy unavailable");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
}
fn set_cookie(response: &mut Response, name: &str, value: &str, http_only: bool, max_age: u32) {
    // HTTP-only loopback profile. Non-local startup is disabled until TLS-qualified configuration exists.
    let value = format!(
        "{name}={value}; Path=/; SameSite=Lax; Max-Age={max_age}{}",
        if http_only { "; HttpOnly" } else { "" }
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&value).expect("internally generated cookie"),
    );
}
fn browser(trust: Trust) -> Result<Trust, StatusCode> {
    if trust == Trust::Service {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(trust)
    }
}

pub fn router(service: IdentityService) -> Router {
    Router::new()
        .route("/auth/{trust}/login", get(login))
        .route("/auth/{trust}/register", get(register))
        .route(
            "/auth/{trust}/contact-verification",
            get(contact_status).post(contact_send),
        )
        .route(
            "/auth/{trust}/contact-verification/confirm",
            post(contact_confirm),
        )
        .route("/auth/{trust}/callback", get(callback))
        .route("/auth/{trust}/me", get(me))
        .route("/auth/{trust}/logout", post(logout))
        .route("/auth/{trust}/security", get(security))
        .route("/auth/{trust}/signed-out", get(signed_out))
        .route("/auth/{trust}/end-session/{ticket}", get(end_session))
        .with_state(Arc::new(service))
}

async fn login(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
) -> Result<Response, StatusCode> {
    begin(&service, browser(Trust::parse(&trust)?)?, false).await
}

/// UI-02: customers may create a sign-in at the issuer (OIDC `prompt=create`); credentials are only entered there.
/// The same PKCE, state and nonce checks apply, and the callback signs the new identity in like any other login.
async fn register(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
) -> Result<Response, StatusCode> {
    match Trust::parse(&trust)? {
        Trust::Customer => begin(&service, Trust::Customer, true).await,
        _ => Err(StatusCode::NOT_FOUND),
    }
}

/// UI-03: customer-only email ownership by one-time code (simulated delivery; see `crate::contact`).
async fn contact_identity(
    service: &IdentityService,
    trust: &str,
    headers: &HeaderMap,
    mutating: bool,
) -> Result<Identity, StatusCode> {
    match Trust::parse(trust)? {
        Trust::Customer => {
            service
                .authenticate(headers, Trust::Customer, mutating)
                .await
        }
        _ => Err(StatusCode::NOT_FOUND),
    }
}
fn contact_error(error: anyhow::Error) -> StatusCode {
    use crate::contact::ContactError;
    match error.downcast_ref::<ContactError>() {
        Some(ContactError::AlreadyVerified) => StatusCode::CONFLICT,
        Some(ContactError::ResendTooSoon | ContactError::TooManyCodes) => {
            StatusCode::TOO_MANY_REQUESTS
        }
        Some(ContactError::NoEmail) => StatusCode::UNPROCESSABLE_ENTITY,
        None if error.to_string().starts_with("A 6-digit code") => StatusCode::BAD_REQUEST,
        None if error.to_string().starts_with("Email changed") => StatusCode::CONFLICT,
        None => StatusCode::SERVICE_UNAVAILABLE,
    }
}
async fn contact_status(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
    headers: HeaderMap,
) -> Result<Json<crate::contact::ContactVerification>, StatusCode> {
    let identity = contact_identity(&service, &trust, &headers, false).await?;
    service
        .contact_verification(&identity.actor.issuer, &identity.actor.subject)
        .await
        .map(Json)
        .map_err(contact_error)
}
async fn contact_send(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
    headers: HeaderMap,
) -> Result<Json<crate::contact::ContactVerification>, StatusCode> {
    let identity = contact_identity(&service, &trust, &headers, true).await?;
    service
        .send_contact_code(&identity.actor.issuer, &identity.actor.subject)
        .await
        .map(Json)
        .map_err(contact_error)
}
#[derive(Deserialize)]
struct ContactCode {
    code: String,
}
async fn contact_confirm(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ContactCode>,
) -> Result<Json<crate::contact::ContactVerification>, StatusCode> {
    let identity = contact_identity(&service, &trust, &headers, true).await?;
    service
        .confirm_contact_code(&identity.actor.issuer, &identity.actor.subject, &body.code)
        .await
        .map(Json)
        .map_err(contact_error)
}

async fn begin(
    service: &IdentityService,
    trust: Trust,
    create: bool,
) -> Result<Response, StatusCode> {
    let state = random();
    let verifier = random();
    let nonce = random();
    sqlx::query("INSERT INTO identity.login_attempts(state_hash,trust,verifier,nonce,expires_at) VALUES($1,$2,$3,$4,now()+interval '5 minutes')")
        .bind(hash(&state)).bind(trust.name()).bind(&verifier).bind(&nonce).execute(&service.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    let mut url = url::Url::parse(&format!(
        "{}/protocol/openid-connect/auth",
        service.verifier.issuer(trust)
    ))
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", trust.client().to_owned()),
        (
            "redirect_uri",
            format!("{}/auth/{}/callback", service.public_base, trust.name()),
        ),
        ("response_type", "code".into()),
        ("scope", "openid profile".into()),
        ("state", state.clone()),
        ("nonce", nonce),
        ("code_challenge_method", "S256".into()),
        (
            "code_challenge",
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        ),
    ]);
    if create {
        url.query_pairs_mut().append_pair("prompt", "create");
    }
    let mut response = Redirect::to(url.as_str()).into_response();
    set_cookie(
        &mut response,
        &format!("pb_login_{}", trust.name()),
        &state,
        true,
        300,
    );
    Ok(response)
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    code: String,
    iss: Option<String>,
}
#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

async fn callback(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
    Query(query): Query<Callback>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let trust = browser(Trust::parse(&trust)?)?;
    let state =
        cookie(&headers, &format!("pb_login_{}", trust.name())).ok_or(StatusCode::UNAUTHORIZED)?;
    if !equal(&state, &query.state)
        || query
            .iss
            .as_ref()
            .is_some_and(|iss| *iss != service.verifier.issuer(trust))
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let attempt=sqlx::query("DELETE FROM identity.login_attempts WHERE state_hash=$1 AND trust=$2 AND expires_at>now() RETURNING *")
        .bind(hash(&state)).bind(trust.name()).fetch_optional(&service.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.ok_or(StatusCode::UNAUTHORIZED)?;
    let verifier: String = attempt.get("verifier");
    let nonce: String = attempt.get("nonce");
    let tokens: Tokens = service
        .verifier
        .http
        .post(format!(
            "{}/protocol/openid-connect/token",
            service.verifier.issuer(trust)
        ))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", trust.client()),
            ("client_secret", trust.client_secret().as_str()),
            ("code", query.code.as_str()),
            ("code_verifier", verifier.as_str()),
            (
                "redirect_uri",
                format!("{}/auth/{}/callback", service.public_base, trust.name()).as_str(),
            ),
        ])
        .send()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .error_for_status()
        .map_err(|_| StatusCode::UNAUTHORIZED)?
        .json()
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let identity = service.verifier.verify(&tokens.access_token, trust).await?;
    let id_token = tokens.id_token.ok_or(StatusCode::UNAUTHORIZED)?;
    let id_claims = service
        .verifier
        .claims(&id_token, trust, trust.client())
        .await?;
    if id_claims.nonce.as_deref() != Some(&nonce) || id_claims.sub != identity.actor.subject {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let refresh = tokens.refresh_token.ok_or(StatusCode::UNAUTHORIZED)?;
    let session = random();
    let session_hash = hash(&session);
    let ring = crate::seal::KeyRing::load().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let csrf = random();
    let mut tx = service
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    sqlx::query("INSERT INTO identity.subjects(issuer,subject,trust) VALUES($1,$2,$3) ON CONFLICT(issuer,subject) DO NOTHING")
        .bind(&identity.actor.issuer).bind(&identity.actor.subject).bind(trust.name()).execute(&mut *tx).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    if let Some(challenge) = attempt.get::<Option<uuid::Uuid>, _>("challenge_id") {
        let expected_issuer: Option<String> = attempt.get("expected_issuer");
        let expected_subject: Option<String> = attempt.get("expected_subject");
        let created: chrono::DateTime<chrono::Utc> = attempt.get("created_at");
        if expected_issuer.as_deref() != Some(&identity.actor.issuer)
            || expected_subject.as_deref() != Some(&identity.actor.subject)
            || id_claims.auth_time.is_none_or(|time| {
                time < created.timestamp() || time > chrono::Utc::now().timestamp() + 2
            })
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let digest: Option<String> = attempt.get("intent_digest");
        sqlx::query("INSERT INTO identity.step_up_proofs(id,challenge_id,issuer,subject,intent_digest,authenticated_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+interval '2 minutes') ON CONFLICT(challenge_id) DO NOTHING")
            .bind(uuid::Uuid::new_v4()).bind(challenge).bind(&identity.actor.issuer).bind(&identity.actor.subject).bind(digest.ok_or(StatusCode::UNAUTHORIZED)?)
            .bind(chrono::DateTime::<chrono::Utc>::from_timestamp(id_claims.auth_time.ok_or(StatusCode::UNAUTHORIZED)?,0).ok_or(StatusCode::UNAUTHORIZED)?)
            .execute(&mut *tx).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    }
    sqlx::query("INSERT INTO identity.sessions(id_hash,issuer,subject,trust,access_token,refresh_token,id_token,csrf_hash,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,now()+interval '30 minutes')")
        .bind(&session_hash).bind(&identity.actor.issuer).bind(&identity.actor.subject).bind(trust.name())
        .bind(ring.seal(&tokens.access_token,&format!("{session_hash}:access"))).bind(ring.seal(&refresh,&format!("{session_hash}:refresh"))).bind(ring.seal(&id_token,&format!("{session_hash}:id")))
        .bind(hash(&csrf)).execute(&mut *tx).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    sqlx::query(
        "INSERT INTO identity.audit(id,issuer,subject,action) VALUES($1,$2,$3,'session.created')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(&identity.actor.issuer)
    .bind(&identity.actor.subject)
    .execute(&mut *tx)
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tx.commit()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let destination = web_origin(trust)?.unwrap_or_else(|| format!("/auth/{}/me", trust.name()));
    let mut response = Redirect::to(&destination).into_response();
    set_cookie(
        &mut response,
        &format!("pb_{}", trust.name()),
        &session,
        true,
        1800,
    );
    set_cookie(
        &mut response,
        &format!("pb_csrf_{}", trust.name()),
        &csrf,
        false,
        1800,
    );
    set_cookie(
        &mut response,
        &format!("pb_login_{}", trust.name()),
        "",
        true,
        0,
    );
    Ok(response)
}

async fn me(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Identity>, StatusCode> {
    Ok(Json(
        service
            .authenticate(&headers, Trust::parse(&trust)?, false)
            .await?,
    ))
}
async fn security(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
    headers: HeaderMap,
) -> Result<Redirect, StatusCode> {
    let trust = browser(Trust::parse(&trust)?)?;
    service.authenticate(&headers, trust, false).await?;
    Ok(Redirect::to(&format!(
        "{}/account",
        service.verifier.issuer(trust)
    )))
}
async fn logout(
    State(service): State<Arc<IdentityService>>,
    Path(trust): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    if headers.contains_key(header::AUTHORIZATION) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let trust = browser(Trust::parse(&trust)?)?;
    let identity = service.authenticate(&headers, trust, true).await?;
    let session =
        cookie(&headers, &format!("pb_{}", trust.name())).ok_or(StatusCode::UNAUTHORIZED)?;
    let mut tx = service
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let row = sqlx::query("DELETE FROM identity.sessions WHERE id_hash=$1 RETURNING id_token")
        .bind(hash(&session))
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    sqlx::query(
        "INSERT INTO identity.audit(id,issuer,subject,action) VALUES($1,$2,$3,'session.revoked')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(identity.actor.issuer)
    .bind(identity.actor.subject)
    .execute(&mut *tx)
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut url = url::Url::parse(&format!(
        "{}/protocol/openid-connect/logout",
        service.verifier.issuer(trust)
    ))
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    url.query_pairs_mut().extend_pairs([
        (
            "id_token_hint",
            crate::seal::KeyRing::load()
                .ok()
                .and_then(|ring| {
                    ring.open(
                        &row.get::<String, _>("id_token"),
                        &format!("{}:id", hash(&session)),
                    )
                })
                .unwrap_or_default(),
        ),
        (
            "post_logout_redirect_uri",
            format!("{}/auth/{}/signed-out", service.public_base, trust.name()),
        ),
    ]);
    let wants_navigation = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|part| part.trim() == "application/json"));
    let mut response = if wants_navigation {
        let ticket = random();
        sqlx::query(
            "INSERT INTO identity.logout_navigation(ticket_hash,trust,location) VALUES($1,$2,$3)",
        )
        .bind(hash(&ticket))
        .bind(trust.name())
        .bind(url.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Json(
            serde_json::json!({"continueTo":format!("/auth/{}/end-session/{ticket}",trust.name())}),
        )
        .into_response()
    } else {
        Redirect::to(url.as_str()).into_response()
    };
    tx.commit()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    set_cookie(&mut response, &format!("pb_{}", trust.name()), "", true, 0);
    set_cookie(
        &mut response,
        &format!("pb_csrf_{}", trust.name()),
        "",
        false,
        0,
    );
    Ok(response)
}

fn web_origin(trust: Trust) -> Result<Option<String>, StatusCode> {
    let Ok(value) = std::env::var(format!(
        "BANK_{}_WEB_ORIGIN",
        trust.name().to_ascii_uppercase()
    )) else {
        return Ok(None);
    };
    let url = url::Url::parse(&value).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(Some(value.trim_end_matches('/').to_string()))
}
async fn signed_out(Path(trust): Path<String>) -> Result<Response, StatusCode> {
    let trust = browser(Trust::parse(&trust)?)?;
    Ok(if let Some(origin) = web_origin(trust)? {
        Redirect::to(&origin).into_response()
    } else {
        Json(serde_json::json!({"status":"signed_out"})).into_response()
    })
}
async fn end_session(
    State(service): State<Arc<IdentityService>>,
    Path((trust, ticket)): Path<(String, String)>,
) -> Result<Redirect, StatusCode> {
    let trust = browser(Trust::parse(&trust)?)?;
    if ticket.len() > 128 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let location:Option<String>=sqlx::query_scalar("DELETE FROM identity.logout_navigation WHERE ticket_hash=$1 AND trust=$2 AND expires_at>now() RETURNING location").bind(hash(&ticket)).bind(trust.name()).fetch_optional(&service.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Redirect::to(&location.ok_or(StatusCode::UNAUTHORIZED)?))
}
