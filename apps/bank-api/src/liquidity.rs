//! WP-40 liquidity forecasting and controlled finance automation (staff only). Forecasts, policy changes, automated
//! settlement-prefunding actions and restriction lifts; every change is maker/checker and every booking goes through
//! Core's approved command. Customers' payments only read the restriction gate.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_finance::Finance;
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::ActorContext;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Arc<Finance>)>;
pub fn router(identity: IdentityService, finance: Arc<Finance>) -> Router {
    Router::new()
        .route("/api/v1/staff/liquidity/forecasts", get(view).post(run))
        .route("/api/v1/staff/liquidity/forecasts/{id}", get(forecast))
        .route(
            "/api/v1/staff/liquidity/forecasts/{id}/reproduction",
            post(reproduce),
        )
        .route("/api/v1/staff/liquidity/policies", post(propose_policy))
        .route(
            "/api/v1/staff/liquidity/policies/{version}/decision",
            post(decide_policy),
        )
        .route(
            "/api/v1/staff/liquidity/actions/{id}/confirmation",
            post(confirm_action),
        )
        .route(
            "/api/v1/staff/liquidity/actions/{id}/decision",
            post(decide_action),
        )
        .route(
            "/api/v1/staff/liquidity/actions/{id}/execution",
            post(execute_action),
        )
        .route(
            "/api/v1/staff/liquidity/restrictions/{id}/lifts",
            post(propose_lift),
        )
        .route(
            "/api/v1/staff/liquidity/lifts/{id}/decision",
            post(decide_lift),
        )
        .with_state(Arc::new((identity, finance)))
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
type Reply = Result<Json<Value>, StatusCode>;
fn respond(r: anyhow::Result<Value>) -> Reply {
    r.map(Json).map_err(status)
}
async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Workforce, write).await?.actor)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyInput {
    document: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Decision {
    approve: bool,
}
async fn view(State(a): State<App>, h: HeaderMap) -> Reply {
    let actor = staff(&a, &h, false).await?;
    respond(a.1.liquidity_view(&actor).await)
}
async fn run(State(a): State<App>, h: HeaderMap) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.run_forecast(&actor).await)
}
async fn forecast(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, false).await?;
    respond(a.1.forecast(&actor, id).await)
}
async fn reproduce(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.reproduce_forecast(&actor, id).await)
}
async fn propose_policy(State(a): State<App>, h: HeaderMap, Json(b): Json<PolicyInput>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.propose_liquidity_policy(&actor, &b.document).await)
}
async fn decide_policy(
    State(a): State<App>,
    h: HeaderMap,
    Path(version): Path<i32>,
    Json(b): Json<Decision>,
) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(
        a.1.decide_liquidity_policy(&actor, version, b.approve)
            .await,
    )
}
async fn confirm_action(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.confirm_liquidity_action(&actor, id).await)
}
async fn decide_action(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Decision>,
) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.decide_liquidity_action(&actor, id, b.approve).await)
}
async fn execute_action(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.execute_liquidity_action(&actor, id).await)
}
async fn propose_lift(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.propose_restriction_lift(&actor, id).await)
}
async fn decide_lift(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Decision>,
) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.decide_restriction_lift(&actor, id, b.approve).await)
}
