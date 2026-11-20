//! WP-36 synthetic batch credit rail: customer recipients and entries (common approval), operator window close and
//! the signed network endpoint for acknowledgements, settlement and returns.
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
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Arc<Payments>)>;
pub fn router(identity: IdentityService, payments: Arc<Payments>) -> Router {
    Router::new()
        .route(
            "/api/v1/batch/recipients",
            get(recipients).post(add_recipient),
        )
        .route("/api/v1/batch/payments", get(list).post(create))
        .route("/api/v1/batch/payments/{id}", get(detail))
        .route("/api/v1/batch/payments/{id}/confirmation", post(confirm))
        .route("/api/v1/batch/payments/{id}/cancellation", post(cancel))
        .route("/api/v1/staff/batch", get(staff_view))
        .route("/api/v1/staff/batch/window-closure", post(close))
        .route("/channels/batch/v1/messages", post(message))
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
    if e.downcast_ref::<serde_json::Error>().is_some() {
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
fn key(h: &HeaderMap) -> Result<Uuid, StatusCode> {
    h.get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or(StatusCode::BAD_REQUEST)
}
async fn customer(
    a: &App,
    h: &HeaderMap,
    write: bool,
) -> Result<bank_platform::domain::ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Customer, write).await?.actor)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecipientInput {
    label: String,
    routing_number: String,
    account_number: String,
}
async fn recipients(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, false).await?;
    a.1.batch_recipients(&actor).await.map(Json).map_err(error)
}
async fn add_recipient(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<RecipientInput>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let actor = customer(&a, &h, true).await?;
    a.1.batch_add_recipient(
        &actor,
        key(&h)?,
        &b.label,
        &b.routing_number,
        &b.account_number,
    )
    .await
    .map(|v| (StatusCode::CREATED, Json(v)))
    .map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EntryInput {
    account_id: AccountId,
    recipient_id: Uuid,
    amount_minor: i64,
}
async fn list(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, false).await?;
    a.1.batch_list(&actor).await.map(Json).map_err(error)
}
async fn create(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<EntryInput>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let actor = customer(&a, &h, true).await?;
    a.1.batch_create(
        &actor,
        key(&h)?,
        b.account_id,
        b.recipient_id,
        b.amount_minor,
    )
    .await
    .map(|v| (StatusCode::CREATED, Json(v)))
    .map_err(error)
}
async fn detail(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, false).await?;
    a.1.batch_detail(&actor, id).await.map(Json).map_err(error)
}
async fn confirm(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let actor = customer(&a, &h, true).await?;
    a.1.batch_confirm(&actor, id)
        .await
        .map(|v| (StatusCode::ACCEPTED, Json(v)))
        .map_err(error)
}
async fn cancel(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, true).await?;
    a.1.batch_cancel(&actor, id).await.map(Json).map_err(error)
}
async fn staff_view(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Workforce, false).await?.actor;
    a.1.batch_staff_view(&actor)
        .await
        .map(Json)
        .map_err(|_| StatusCode::FORBIDDEN)
}
async fn close(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Workforce, true).await?.actor;
    match a.1.batch_staff_close(&actor).await {
        Ok(file) => Ok(Json(json!({"file": file}))),
        Err(e) if e.downcast_ref::<sqlx::Error>().is_none() => Err(StatusCode::FORBIDDEN),
        Err(e) => Err(error(e)),
    }
}
async fn message(State(a): State<App>, h: HeaderMap, body: Bytes) -> Result<Response, StatusCode> {
    let signature = h
        .get("Piggybank-Batch-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let (raw, signed) = a.1.batch_message(&body, &signature).await.map_err(error)?;
    let mut response =
        (StatusCode::OK, [("content-type", "application/json")], raw).into_response();
    response.headers_mut().insert(
        "piggybank-batch-signature",
        HeaderValue::from_str(&signed).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    );
    Ok(response)
}
