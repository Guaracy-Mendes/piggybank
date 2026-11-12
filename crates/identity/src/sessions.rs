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
        let access: String = row.get("access_token");
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
        let refresh: String = row.get("refresh_token");
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
                .bind(id_hash).bind(lease).bind(&tokens.access_token).bind(tokens.refresh_token.unwrap_or(refresh)).execute(&self.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
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
    let trust = browser(Trust::parse(&trust)?)?;
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
        .bind(hash(&session)).bind(&identity.actor.issuer).bind(&identity.actor.subject).bind(trust.name()).bind(tokens.access_token).bind(refresh).bind(id_token).bind(hash(&csrf)).execute(&mut *tx).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
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
        ("id_token_hint", row.get::<String, _>("id_token")),
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
