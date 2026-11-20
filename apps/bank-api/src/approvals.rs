use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
};
use bank_controls::entitlements::Entitlements;
use bank_identity::{IdentityService, Trust};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub fn router(identity: IdentityService, controls: Entitlements) -> Router {
    Router::new()
        .route("/auth/{trust}/step-up/{challengeId}", get(begin))
        .route(
            "/api/v1/approvals/{challengeId}/native-step-up",
            post(native_begin),
        )
        .route(
            "/api/v1/approvals/{challengeId}/native-proof",
            post(native_proof),
        )
        .with_state(Arc::new((identity, controls)))
}
async fn begin(
    State(state): State<Arc<(IdentityService, Entitlements)>>,
    Path((trust, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let trust = Trust::parse(&trust)?;
    let identity = state.0.authenticate(&headers, trust, false).await?;
    let challenge = state
        .1
        .pending_challenge(&identity.actor, id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    state
        .0
        .begin_step_up(
            trust,
            &identity.actor,
            challenge.id,
            &challenge.nonce,
            &challenge.intent_digest,
        )
        .await
}

/// WP-27 native reauthentication: returns the server-issued nonce for a fresh PKCE login (prompt=login, max_age=0).
async fn native_begin(
    State(state): State<Arc<(IdentityService, Entitlements)>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let identity = state
        .0
        .authenticate(&headers, Trust::Customer, true)
        .await?;
    let challenge = state
        .1
        .pending_challenge(&identity.actor, id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let expires = state
        .0
        .native_step_up_begin(
            &identity.actor,
            challenge.id,
            &challenge.nonce,
            &challenge.intent_digest,
        )
        .await?;
    Ok(Json(serde_json::json!({
        "challengeId": challenge.id,
        "nonce": challenge.nonce,
        "prompt": "login",
        "maxAge": 0,
        "expiresAt": expires,
    })))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeProof {
    id_token: String,
}
async fn native_proof(
    State(state): State<Arc<(IdentityService, Entitlements)>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<NativeProof>,
) -> Result<StatusCode, StatusCode> {
    let identity = state
        .0
        .authenticate(&headers, Trust::Customer, true)
        .await?;
    state
        .1
        .pending_challenge(&identity.actor, id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    state
        .0
        .native_step_up_complete(&identity.actor, id, &body.id_token)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
