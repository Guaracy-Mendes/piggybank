//! WP-25 statements, single-use retrieval, legal holds, disposal and data-subject requests.
//! Readiness refusals and restricted disposals are traceable outcomes, returned with their reasons.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bank_controls::entitlements::Entitlements;
use bank_documents::{
    retention::{DataRequestInput, HoldInput, Restricted},
    statements::{AccessDenied, NotReady, Records},
};
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::ActorContext;
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Records, Entitlements)>;
pub fn router(identity: IdentityService, records: Records, controls: Entitlements) -> Router {
    Router::new()
        .route("/api/v1/documents", get(customer_documents))
        .route("/api/v1/accounts/{id}/statements", post(customer_generate))
        .route(
            "/api/v1/documents/statements/{id}/links",
            post(customer_link),
        )
        .route("/api/v1/documents/downloads/{id}", get(customer_download))
        .route("/api/v1/documents/data-requests", post(submit_request))
        .route(
            "/api/v1/staff/documents/accounts/{id}",
            get(staff_documents),
        )
        .route(
            "/api/v1/staff/documents/accounts/{id}/statements",
            post(staff_generate),
        )
        .route(
            "/api/v1/staff/documents/statements/{id}/links",
            post(staff_link),
        )
        .route(
            "/api/v1/staff/documents/downloads/{id}",
            get(staff_download),
        )
        .route(
            "/api/v1/staff/documents/statements/{id}/disposal",
            post(dispose),
        )
        .route("/api/v1/staff/documents/legal-holds", post(place_hold))
        .route(
            "/api/v1/staff/documents/legal-holds/{id}/release",
            post(release_hold),
        )
        .route("/api/v1/staff/documents/data-requests", get(data_requests))
        .route(
            "/api/v1/staff/documents/data-requests/{id}/processing",
            post(process_request),
        )
        .route(
            "/api/v1/staff/documents/data-requests/{id}/export-disposal",
            post(dispose_export),
        )
        .with_state(Arc::new((identity, records, controls)))
}
fn error(e: anyhow::Error) -> StatusCode {
    if let Some(denied) = e.downcast_ref::<AccessDenied>() {
        // Do not reveal whether a reference exists for another person; spent references are gone.
        return match denied.0.as_str() {
            "EXPIRED" | "ALREADY_USED" | "DISPOSED" => StatusCode::GONE,
            _ => StatusCode::NOT_FOUND,
        };
    }
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
/// Readiness refusals and restricted disposals are outcomes, not failures: nothing was published or removed.
fn outcome(result: anyhow::Result<Value>) -> Result<Json<Value>, StatusCode> {
    match result {
        Ok(value) => Ok(Json(value)),
        Err(e) => {
            if let Some(NotReady(readiness)) = e.downcast_ref::<NotReady>() {
                return Ok(Json(json!({"state":"NOT_READY","readiness":readiness})));
            }
            if let Some(Restricted(value)) = e.downcast_ref::<Restricted>() {
                return Ok(Json(value.clone()));
            }
            Err(error(e))
        }
    }
}
fn key(h: &HeaderMap) -> Result<Uuid, StatusCode> {
    h.get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or(StatusCode::BAD_REQUEST)
}
async fn customer(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Customer, write).await?.actor)
}
async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Workforce, write).await?.actor)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Period {
    period_start: NaiveDate,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reason {
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Format {
    format: Option<String>,
}
async fn customer_documents(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, false).await?;
    a.1.customer_documents(&a.2, &actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn customer_generate(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(p): Json<Period>,
) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, true).await?;
    outcome(a.1.generate(&a.2, &actor, id, p.period_start).await)
}
async fn customer_link(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = customer(&a, &h, true).await?;
    a.1.link(&a.2, &actor, id).await.map(Json).map_err(error)
}
async fn download(
    a: &App,
    actor: &ActorContext,
    token: &str,
    format: Option<String>,
) -> Result<Response, StatusCode> {
    let pdf = match format.as_deref() {
        None | Some("json") => false,
        Some("pdf") => true,
        Some(_) => return Err(StatusCode::BAD_REQUEST),
    };
    let r = a.1.redeem(&a.2, actor, token).await.map_err(error)?;
    if !pdf {
        return Ok(Json(r).into_response());
    }
    let digest = r["contentDigest"].as_str().unwrap_or_default();
    let body =
        crate::consumer_pdf::period_statement(&r["document"], digest, r["supersededBy"].as_str());
    Ok((
        [
            ("Content-Type", "application/pdf"),
            ("Cache-Control", "private, no-store"),
            (
                "Content-Disposition",
                "attachment; filename=synthetic-period-statement.pdf",
            ),
        ],
        body,
    )
        .into_response())
}
async fn customer_download(
    State(a): State<App>,
    h: HeaderMap,
    Path(token): Path<String>,
    Query(f): Query<Format>,
) -> Result<Response, StatusCode> {
    let actor = customer(&a, &h, false).await?;
    download(&a, &actor, &token, f.format).await
}
async fn submit_request(
    State(a): State<App>,
    h: HeaderMap,
    Json(input): Json<DataRequestInput>,
) -> Result<Json<Value>, StatusCode> {
    let k = key(&h)?;
    let actor = customer(&a, &h, true).await?;
    a.1.submit_request(&a.2, &actor, k, &input)
        .await
        .map(Json)
        .map_err(error)
}
async fn staff_documents(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    a.1.staff_documents(&a.2, &actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn staff_generate(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(p): Json<Period>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    outcome(a.1.generate(&a.2, &actor, id, p.period_start).await)
}
async fn staff_link(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.link(&a.2, &actor, id).await.map(Json).map_err(error)
}
async fn staff_download(
    State(a): State<App>,
    h: HeaderMap,
    Path(token): Path<String>,
    Query(f): Query<Format>,
) -> Result<Response, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    download(&a, &actor, &token, f.format).await
}
async fn dispose(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(r): Json<Reason>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    outcome(a.1.dispose(&a.2, &actor, id, &r.reason).await)
}
async fn place_hold(
    State(a): State<App>,
    h: HeaderMap,
    Json(input): Json<HoldInput>,
) -> Result<Json<Value>, StatusCode> {
    let k = key(&h)?;
    let actor = staff(&a, &h, true).await?;
    a.1.place_hold(&a.2, &actor, k, &input)
        .await
        .map(Json)
        .map_err(error)
}
async fn release_hold(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(r): Json<Reason>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.release_hold(&a.2, &actor, id, &r.reason)
        .await
        .map(Json)
        .map_err(error)
}
async fn data_requests(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    a.1.data_requests(&a.2, &actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn process_request(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.process_request(&a.2, &actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn dispose_export(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(r): Json<Reason>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    outcome(a.1.dispose_export(&a.2, &actor, id, &r.reason).await)
}
