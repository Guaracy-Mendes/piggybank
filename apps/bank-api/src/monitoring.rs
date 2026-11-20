//! WP-26 monitoring, data quality, operations reporting, correlation trace and notifications.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::{entitlements::Entitlements, monitoring::Monitoring};
use bank_data::alerts::Alerts;
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::ActorContext;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Monitoring, Alerts, Entitlements)>;
pub fn router(
    identity: IdentityService,
    monitoring: Monitoring,
    alerts: Alerts,
    controls: Entitlements,
) -> Router {
    Router::new()
        .route("/api/v1/staff/monitoring/alerts", get(monitoring_alerts))
        .route("/api/v1/staff/monitoring/scans", post(scan))
        .route(
            "/api/v1/staff/monitoring/alerts/{id}/disposition",
            post(propose),
        )
        .route(
            "/api/v1/staff/monitoring/alerts/{id}/approval",
            post(approve),
        )
        .route("/api/v1/staff/operations-report", get(snapshot))
        .route("/api/v1/staff/traces/{id}", get(trace))
        .route("/api/v1/staff/data-quality", get(quality))
        .route("/api/v1/staff/data-quality/repairs", post(repair))
        .route(
            "/api/v1/staff/data-quality/alerts/{id}/acknowledgement",
            post(acknowledge),
        )
        .route("/api/v1/staff/notifications", get(staff_notifications))
        .route("/api/v1/staff/notifications/channels", post(channel))
        .route("/api/v1/staff/pages", get(pages))
        .route(
            "/api/v1/staff/pages/{id}/acknowledgement",
            post(acknowledge_page),
        )
        .route("/api/v1/staff/oncall", post(oncall))
        .route("/api/v1/notifications", get(messages))
        .route("/api/v1/notifications/preferences", post(preference))
        .with_state(Arc::new((identity, monitoring, alerts, controls)))
}
fn error(e: anyhow::Error) -> StatusCode {
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(d)) => match d.code().as_deref() {
            Some("P0002") => StatusCode::NOT_FOUND,
            Some("P0001" | "23505" | "23514" | "23502" | "23503") => StatusCode::CONFLICT,
            Some(code) if code.starts_with("22") => StatusCode::BAD_REQUEST,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::FORBIDDEN,
    }
}
async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Workforce, write).await?.actor)
}
async fn customer(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Customer, write).await?.actor)
}
fn json(r: anyhow::Result<Value>) -> Result<Json<Value>, StatusCode> {
    r.map(Json).map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    disposition: String,
    rationale: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Repair {
    batch_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Channel {
    channel: String,
    available: bool,
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preference {
    purpose: String,
    enabled: bool,
}
async fn monitoring_alerts(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    json(a.1.alerts(&a.3, &actor).await)
}
async fn scan(
    State(a): State<App>,
    h: HeaderMap,
    Json(w): Json<Window>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(a.1.scan(&a.3, &actor, w.from, w.to).await)
}
async fn propose(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(p): Json<Proposal>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(
        a.1.propose(&a.3, &actor, id, &p.disposition, &p.rationale)
            .await,
    )
}
async fn approve(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(a.1.approve(&a.3, &actor, id).await)
}
async fn snapshot(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    json(a.1.snapshot(&a.3, &actor).await)
}
async fn trace(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    json(a.1.trace(&a.3, &actor, id).await)
}
async fn quality(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    json(a.2.quality_view(&a.3, &actor).await)
}
async fn repair(
    State(a): State<App>,
    h: HeaderMap,
    Json(r): Json<Repair>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(a.2.repair(&a.3, &actor, r.batch_id).await)
}
async fn acknowledge(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(n): Json<Note>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(a.2.acknowledge(&a.3, &actor, id, &n.note).await)
}
async fn staff_notifications(
    State(a): State<App>,
    h: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    json(a.2.notifications(&a.3, &actor).await)
}
async fn channel(
    State(a): State<App>,
    h: HeaderMap,
    Json(c): Json<Channel>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(
        a.2.set_channel(&a.3, &actor, &c.channel, c.available, &c.reason)
            .await,
    )
}
async fn messages(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, false).await?;
    json(a.2.messages(&a.3, &actor).await)
}
async fn preference(
    State(a): State<App>,
    h: HeaderMap,
    Json(p): Json<Preference>,
) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, true).await?;
    json(
        a.2.set_preference(&a.3, &actor, &p.purpose, p.enabled)
            .await,
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OnCall {
    person_id: Uuid,
    tier: String,
    active: bool,
}
async fn pages(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    json(a.1.pages(&a.3, &actor).await)
}
async fn acknowledge_page(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(a.1.acknowledge(&a.3, &actor, id).await)
}
async fn oncall(
    State(a): State<App>,
    h: HeaderMap,
    Json(o): Json<OnCall>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    json(
        a.1.set_oncall(&a.3, &actor, o.person_id, &o.tier, o.active)
            .await,
    )
}
