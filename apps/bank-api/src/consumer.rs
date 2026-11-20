use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::entitlements::Entitlements;
use bank_core::Core;
use bank_identity::{IdentityService, Trust};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Core, Entitlements)>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    action: String,
    input: Value,
}
pub fn router(identity: IdentityService, core: Core, controls: Entitlements) -> Router {
    Router::new()
        .route("/api/v1/consumer", get(snapshot))
        .route("/api/v1/consumer/commands", post(command))
        .route("/api/v1/consumer/authorizations", post(review))
        .route("/api/v1/consumer/authorizations/{id}", get(authorization))
        .route("/api/v1/consumer/statements/{id}", get(statement))
        .route("/api/v1/consumer/statements/{id}/pdf", get(statement_pdf))
        .route("/api/v1/consumer/integrations/{scope}", get(integration))
        .route("/api/v1/investments", get(wealth))
        .route("/api/v1/investments/quotes", post(quote))
        .with_state(Arc::new((identity, core, controls)))
}
async fn statement(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?.actor;
    s.1.consumer_statement(&s.2, &a, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn integration(
    State(s): State<App>,
    h: HeaderMap,
    Path(scope): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    if !matches!(
        scope.as_str(),
        "accounts:read" | "transactions:read" | "portfolio:read"
    ) {
        return Err(StatusCode::NOT_FOUND);
    }
    let secret = h
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    s.1.consumer_token_read(secret, &scope)
        .await
        .map(Json)
        .map_err(|_| StatusCode::UNAUTHORIZED)
}
fn error(e: anyhow::Error) -> StatusCode {
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(d)) => match d.code().as_deref() {
            Some("P0002") => StatusCode::NOT_FOUND,
            Some("P0001" | "23505" | "23514" | "23502" | "22003") => StatusCode::CONFLICT,
            Some(code) if code.starts_with("22") => StatusCode::BAD_REQUEST,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::FORBIDDEN,
    }
}
async fn snapshot(State(s): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?.actor;
    s.1.consumer_snapshot(&s.2, &a)
        .await
        .map(Json)
        .map_err(error)
}
async fn wealth(State(s): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?.actor;
    s.1.wealth_snapshot(&s.2, &a).await.map(Json).map_err(error)
}
async fn command(
    State(s): State<App>,
    h: HeaderMap,
    Json(c): Json<Command>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?.actor;
    let key = h
        .get("Idempotency-Key")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|s| !s.is_nil())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.consumer_command(&s.2, &a, key, &c.action, &c.input)
        .await
        .map(Json)
        .map_err(error)
}
async fn review(
    State(s): State<App>,
    h: HeaderMap,
    Json(c): Json<Command>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?.actor;
    s.1.consumer_review(&s.2, &a, &c.action, &c.input)
        .await
        .map(Json)
        .map_err(error)
}
async fn authorization(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?.actor;
    s.1.consumer_authorization(&s.2, &a, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn quote(
    State(s): State<App>,
    h: HeaderMap,
    Json(c): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?.actor;
    s.1.wealth_quote(&s.2, &a, &c)
        .await
        .map(Json)
        .map_err(error)
}

async fn statement_pdf(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;
    let a = s.0.authenticate(&h, Trust::Customer, false).await?.actor;
    let d = s.1.consumer_statement(&s.2, &a, id).await.map_err(error)?;
    Ok((
        [
            ("Content-Type", "application/pdf"),
            ("Cache-Control", "private, no-store"),
            (
                "Content-Disposition",
                "attachment; filename=synthetic-statement.pdf",
            ),
        ],
        crate::consumer_pdf::statement(&d),
    )
        .into_response())
}
