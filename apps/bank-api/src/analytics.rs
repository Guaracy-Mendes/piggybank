//! WP-37 governed warehouse (local adapter): staff-only views, backfill, schema registration, purpose-checked snapshots,
//! reproduction and row tracing. No route reaches Core: the service holds only the Data pool.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::entitlements::Entitlements;
use bank_data::{
    analytics::{Backfill, Denied, SchemaVersion, SnapshotRequest, Warehouse},
    outcomes::KpiWindow,
};
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::ActorContext;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Warehouse, Entitlements)>;
pub fn router(identity: IdentityService, warehouse: Warehouse, controls: Entitlements) -> Router {
    Router::new()
        .route("/api/v1/staff/analytics", get(view))
        .route("/api/v1/staff/analytics/ingestions", post(backfill))
        .route("/api/v1/staff/analytics/schemas", post(register))
        .route("/api/v1/staff/analytics/snapshots", post(snapshot))
        .route(
            "/api/v1/staff/analytics/kpi-snapshots",
            get(kpi_list).post(kpi_snapshot),
        )
        .route("/api/v1/staff/analytics/snapshots/{id}", get(detail))
        .route(
            "/api/v1/staff/analytics/snapshots/{id}/reproduction",
            post(reproduce),
        )
        .route(
            "/api/v1/staff/analytics/snapshots/{id}/rows/{row}/trace",
            get(trace),
        )
        .with_state(Arc::new((identity, warehouse, controls)))
}
fn status(e: &anyhow::Error) -> StatusCode {
    if e.downcast_ref::<Denied>().is_some() {
        return StatusCode::FORBIDDEN;
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
        None => StatusCode::FORBIDDEN,
    }
}
/// Error bodies are replaced by the ingress safe envelope; denial reasons are read from the recorded denials.
fn respond(r: anyhow::Result<Value>) -> Reply {
    r.map(Json).map_err(|e| status(&e))
}
type Reply = Result<Json<Value>, StatusCode>;
async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Workforce, write).await?.actor)
}
async fn view(State(a): State<App>, h: HeaderMap) -> Reply {
    let actor = staff(&a, &h, false).await?;
    respond(a.1.view(&a.2, &actor).await)
}
async fn backfill(State(a): State<App>, h: HeaderMap, Json(b): Json<Backfill>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.backfill(&a.2, &actor, &b).await)
}
async fn register(State(a): State<App>, h: HeaderMap, Json(b): Json<SchemaVersion>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.register_schema(&a.2, &actor, &b).await)
}
async fn snapshot(State(a): State<App>, h: HeaderMap, Json(b): Json<SnapshotRequest>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.snapshot(&a.2, &actor, &b).await)
}
async fn detail(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, false).await?;
    respond(a.1.detail(&a.2, &actor, id).await)
}
async fn reproduce(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.reproduce(&a.2, &actor, id).await)
}
async fn trace(State(a): State<App>, h: HeaderMap, Path((id, row)): Path<(Uuid, i32)>) -> Reply {
    let actor = staff(&a, &h, false).await?;
    respond(a.1.trace(&a.2, &actor, id, row).await)
}
/// WP-41: KPI-01..08 from local synthetic records, labelled as such; targets await DEC-07/DEC-12.
async fn kpi_snapshot(State(a): State<App>, h: HeaderMap, Json(b): Json<KpiWindow>) -> Reply {
    let actor = staff(&a, &h, true).await?;
    respond(a.1.kpi_snapshot(&a.2, &actor, &b).await)
}
async fn kpi_list(State(a): State<App>, h: HeaderMap) -> Reply {
    let actor = staff(&a, &h, false).await?;
    respond(a.1.kpi_snapshots(&a.2, &actor).await)
}
