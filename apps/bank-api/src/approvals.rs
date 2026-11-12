use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use bank_controls::entitlements::Entitlements;
use bank_identity::{IdentityService, Trust};
use std::sync::Arc;
use uuid::Uuid;

pub fn router(identity: IdentityService, controls: Entitlements) -> Router {
    Router::new()
        .route("/auth/{trust}/step-up/{challengeId}", get(begin))
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
