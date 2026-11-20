//! WP-35 ATM channel: signed network messages and settlement files, customer card tokens and the staff view.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bank_identity::{IdentityService, Trust};
use bank_payments::{Payments, atm::InvalidSignature};
use bank_platform::domain::AccountId;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Arc<Payments>)>;
pub fn router(identity: IdentityService, payments: Arc<Payments>) -> Router {
    Router::new()
        .route("/channels/atm/v1/messages", post(message))
        .route("/channels/atm/v1/settlement-files", post(settlement))
        .route("/api/v1/atm/cards", get(cards).post(issue))
        .route("/api/v1/atm/cards/{id}/block", post(block))
        .route("/api/v1/staff/atm", get(staff))
        .with_state(Arc::new((identity, payments)))
}
fn error(e: anyhow::Error) -> StatusCode {
    if e.downcast_ref::<InvalidSignature>().is_some() {
        return StatusCode::UNAUTHORIZED;
    }
    if e.downcast_ref::<bank_payments::ResourceUnavailable>()
        .is_some()
    {
        return StatusCode::NOT_FOUND;
    }
    if e.downcast_ref::<serde_json::Error>().is_some()
        || e.downcast_ref::<chrono::ParseError>().is_some()
    {
        return StatusCode::BAD_REQUEST;
    }
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::Database(d))
            if d.code().as_deref().is_some_and(|c| c.starts_with("22")) =>
        {
            StatusCode::BAD_REQUEST
        }
        Some(sqlx::Error::Database(_)) | None => StatusCode::CONFLICT,
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
fn signature(h: &HeaderMap) -> String {
    h.get("Piggybank-Atm-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}
/// The network authenticates with an HMAC over the exact body; the response is signed the same way.
async fn message(State(a): State<App>, h: HeaderMap, body: Bytes) -> Result<Response, StatusCode> {
    let (raw, signed) =
        a.1.atm_message(&body, &signature(&h))
            .await
            .map_err(error)?;
    let mut response =
        (StatusCode::OK, [("content-type", "application/json")], raw).into_response();
    response.headers_mut().insert(
        "piggybank-atm-signature",
        HeaderValue::from_str(&signed).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    );
    Ok(response)
}
async fn settlement(
    State(a): State<App>,
    h: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, StatusCode> {
    a.1.atm_settlement(&body, &signature(&h))
        .await
        .map(Json)
        .map_err(error)
}
async fn cards(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, false).await?.actor;
    a.1.atm_cards(&actor).await.map(Json).map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IssueInput {
    account_id: AccountId,
}
async fn issue(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<IssueInput>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, true).await?.actor;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    a.1.atm_issue_card(&actor, key, b.account_id)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(error)
}
async fn block(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, true).await?.actor;
    a.1.atm_block_card(&actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn staff(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Workforce, false).await?.actor;
    a.1.atm_staff_view(&actor)
        .await
        .map(Json)
        .map_err(|_| StatusCode::FORBIDDEN)
}
