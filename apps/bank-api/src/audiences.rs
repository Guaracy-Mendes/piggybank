//! WP-38 audiences, customer insights and the signed journey-partner channel (local simulator role `SIM_JOURNEY`).
//! Partners receive pseudonymous tokens and can only return advisory recommendations; no route here reaches Core,
//! entitlements or onboarding decisions.
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bank_controls::entitlements::Entitlements;
use bank_data::{
    analytics::Denied,
    audience::{Audiences, InvalidSignature},
    insights::Insights,
};
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::ActorContext;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Audiences, Insights, Entitlements)>;
pub fn router(
    identity: IdentityService,
    audiences: Audiences,
    insights: Insights,
    controls: Entitlements,
) -> Router {
    Router::new()
        .route("/api/v1/staff/audiences", get(view).post(build))
        .route("/api/v1/staff/audiences/{id}/exports", post(export))
        .route("/api/v1/staff/audiences/{id}/outcome-reports", post(report))
        .route("/api/v1/insights", get(list))
        .route("/api/v1/insights/{id}/dismissal", post(dismiss))
        .route("/channels/journeys/v1/exports/{id}", get(partner_export))
        .route(
            "/channels/journeys/v1/recommendations",
            post(recommendation),
        )
        .with_state(Arc::new((identity, audiences, insights, controls)))
}
fn status(e: &anyhow::Error) -> StatusCode {
    if e.downcast_ref::<Denied>().is_some() {
        return StatusCode::FORBIDDEN;
    }
    if e.downcast_ref::<InvalidSignature>().is_some() {
        return StatusCode::UNAUTHORIZED;
    }
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(d)) => match d.code().as_deref() {
            Some("42501") => StatusCode::FORBIDDEN,
            Some("P0002") => StatusCode::NOT_FOUND,
            Some("P0001" | "23505" | "23514") => StatusCode::CONFLICT,
            Some(code) if code.starts_with("22") => StatusCode::BAD_REQUEST,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None if e.downcast_ref::<serde_json::Error>().is_some() => StatusCode::BAD_REQUEST,
        None => StatusCode::FORBIDDEN,
    }
}
type Reply = Result<Json<Value>, StatusCode>;
fn respond(r: anyhow::Result<Value>) -> Reply {
    r.map(Json).map_err(|e| status(&e))
}
async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Workforce, write).await?.actor)
}
async fn customer(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Customer, write).await?.actor)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuildInput {
    segment: String,
    version: i32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportInput {
    partner: String,
}
async fn view(State(a): State<App>, h: HeaderMap) -> Reply {
    let actor = staff(&a, &h, false).await?;
    respond(a.1.view(&a.3, &actor).await)
}
async fn build(State(a): State<App>, h: HeaderMap, Json(b): Json<BuildInput>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.build(&a.3, &actor, &b.segment, b.version).await)
}
async fn export(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<ExportInput>,
) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.export(&a.3, &actor, id, &b.partner).await)
}
async fn report(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.report(&a.3, &actor, id).await)
}
async fn list(State(a): State<App>, h: HeaderMap) -> Reply {
    let actor = customer(&a, &h, false).await?;
    respond(a.2.list(&a.3, &actor).await)
}
async fn dismiss(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = customer(&a, &h, true).await?;
    respond(a.2.dismiss(&a.3, &actor, id).await)
}
fn header<'a>(h: &'a HeaderMap, name: &str) -> &'a str {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}
fn signed(result: anyhow::Result<(Vec<u8>, String)>) -> Result<Response, StatusCode> {
    let (raw, signature) = result.map_err(|e| status(&e))?;
    let mut response =
        (StatusCode::OK, [("content-type", "application/json")], raw).into_response();
    response.headers_mut().insert(
        "piggybank-journey-signature",
        HeaderValue::from_str(&signature).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    );
    Ok(response)
}
async fn partner_export(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    signed(
        a.1.partner_export(
            header(&h, "piggybank-journey-partner"),
            id,
            header(&h, "piggybank-journey-signature"),
        )
        .await,
    )
}
async fn recommendation(
    State(a): State<App>,
    h: HeaderMap,
    body: Bytes,
) -> Result<Response, StatusCode> {
    signed(
        a.1.partner_recommendation(
            header(&h, "piggybank-journey-partner"),
            &body,
            header(&h, "piggybank-journey-signature"),
        )
        .await,
    )
}
