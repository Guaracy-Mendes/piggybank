//! WP-39 routing policies (template with two simulated providers): staff view, maker proposals and independent
//! approval. Customers' payments are routed by the worker; nothing here sends or books money.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_identity::{IdentityService, Trust};
use bank_payments::Payments;
use bank_platform::domain::ActorContext;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Arc<Payments>)>;
pub fn router(identity: IdentityService, payments: Arc<Payments>) -> Router {
    Router::new()
        .route("/api/v1/staff/routing", get(view))
        .route("/api/v1/staff/routing/changes", post(propose))
        .route("/api/v1/staff/routing/changes/{id}/decision", post(decide))
        .with_state(Arc::new((identity, payments)))
}
fn status(e: anyhow::Error) -> StatusCode {
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::Database(d)) => match d.code().as_deref() {
            Some("P0002") => StatusCode::NOT_FOUND,
            Some(code) if code.starts_with("22") => StatusCode::BAD_REQUEST,
            Some("P0001") => {
                let text = d.message();
                if text.contains("role") || text.contains("identity") || text.contains("trust") {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::CONFLICT
                }
            }
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::FORBIDDEN,
    }
}
async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Workforce, write).await?.actor)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Proposal {
    document: Option<Value>,
    version: Option<i32>,
    mode: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Decision {
    approve: bool,
}
async fn view(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    a.1.routing_view(&actor).await.map(Json).map_err(status)
}
async fn propose(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<Proposal>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    if b.document.is_some() == b.version.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    a.1.propose_route_change(&actor, b.document.as_ref(), b.version, &b.mode)
        .await
        .map(Json)
        .map_err(status)
}
async fn decide(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Decision>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.decide_route_change(&actor, id, b.approve)
        .await
        .map(Json)
        .map_err(status)
}
