//! WP-28 recovery, access recertification and security findings.
//! Recovery commits its decision first, then revokes issuer sessions (no transaction spans the issuer call),
//! then records completion. A failed issuer call leaves the request APPROVED for a safe retry.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::{entitlements::Entitlements, security::Security};
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::ActorContext;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Security, Entitlements)>;
pub fn router(identity: IdentityService, security: Security, controls: Entitlements) -> Router {
    Router::new()
        .route("/api/v1/security/device-revocation", post(revoke_devices))
        .route("/api/v1/security/recoveries", get(customer_recoveries))
        .route(
            "/api/v1/staff/security/recoveries",
            get(staff_recoveries).post(staff_recovery),
        )
        .route(
            "/api/v1/staff/security/recoveries/{id}/decision",
            post(decide),
        )
        .route(
            "/api/v1/staff/security/recoveries/{id}/execution",
            post(retry),
        )
        .route(
            "/api/v1/staff/security/access-review",
            get(review).post(open_review),
        )
        .route(
            "/api/v1/staff/security/access-review/{id}/items",
            post(decide_item),
        )
        .route(
            "/api/v1/staff/security/findings",
            get(findings).post(report),
        )
        .route(
            "/api/v1/staff/security/findings/{id}/disposition",
            post(dispose),
        )
        .with_state(Arc::new((identity, security, controls)))
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
fn key(h: &HeaderMap) -> Result<Uuid, StatusCode> {
    h.get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or(StatusCode::BAD_REQUEST)
}
async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<ActorContext, StatusCode> {
    Ok(a.0.authenticate(h, Trust::Workforce, write).await?.actor)
}
/// Executes an APPROVED recovery at the issuer, then records completion and the safe security notice.
async fn execute(a: &App, id: Uuid) -> Result<Value, StatusCode> {
    let target = a.1.recovery_target(id).await.map_err(error)?;
    if target["state"] == "EXECUTED" {
        return a.1.complete_recovery(id, &json!({})).await.map_err(error);
    }
    if target["state"] != "APPROVED" {
        return Ok(target);
    }
    let outcome = a
        .0
        .revoke_customer(
            target["subject"].as_str().unwrap_or_default(),
            target["kind"] == "CREDENTIAL_RESET",
        )
        .await
        .map_err(|e| {
            tracing::warn!(code = "RECOVERY_EXECUTION_PENDING", recovery = %id, error = %e, "issuer revocation failed; request stays approved");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
    a.1.complete_recovery(id, &outcome).await.map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reason {
    reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    approve: bool,
    reason: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Item {
    grant_id: Uuid,
    decision: String,
    reason: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Disposition {
    status: String,
    disposition: String,
    evidence_ref: Option<String>,
}
async fn revoke_devices(
    State(a): State<App>,
    h: HeaderMap,
    Json(r): Json<Reason>,
) -> Result<Json<Value>, StatusCode> {
    let k = key(&h)?;
    let actor = a.0.authenticate(&h, Trust::Customer, true).await?.actor;
    let request =
        a.1.request_recovery(
            &a.2,
            &actor,
            k,
            &json!({"kind":"LOST_DEVICE","reason":r.reason}),
        )
        .await
        .map_err(error)?;
    let id = serde_json::from_value(request["id"].clone()).map_err(|_| StatusCode::CONFLICT)?;
    execute(&a, id)
        .await
        .map(|v| Json(json!({"id": v["id"], "state": v["state"], "kind": v["kind"]})))
}
async fn customer_recoveries(
    State(a): State<App>,
    h: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, false).await?.actor;
    a.1.recoveries(&a.2, &actor).await.map(Json).map_err(error)
}
async fn staff_recoveries(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    a.1.recoveries(&a.2, &actor).await.map(Json).map_err(error)
}
async fn staff_recovery(
    State(a): State<App>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let k = key(&h)?;
    let actor = staff(&a, &h, true).await?;
    let request =
        a.1.request_recovery(&a.2, &actor, k, &input)
            .await
            .map_err(error)?;
    if request["state"] == "APPROVED" {
        let id = serde_json::from_value(request["id"].clone()).map_err(|_| StatusCode::CONFLICT)?;
        return execute(&a, id).await.map(Json);
    }
    Ok(Json(request))
}
async fn decide(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(d): Json<Decision>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    let decided =
        a.1.decide_recovery(&a.2, &actor, id, d.approve, &d.reason)
            .await
            .map_err(error)?;
    if decided["state"] == "APPROVED" {
        return execute(&a, id).await.map(Json);
    }
    Ok(Json(decided))
}
async fn retry(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    // Only staff who can see the recovery queue may retry execution.
    a.1.recoveries(&a.2, &actor).await.map_err(error)?;
    execute(&a, id).await.map(Json)
}
async fn review(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    a.1.review(&a.2, &actor).await.map(Json).map_err(error)
}
async fn open_review(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.open_review(&a.2, &actor).await.map(Json).map_err(error)
}
async fn decide_item(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(i): Json<Item>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.decide_item(&a.2, &actor, id, i.grant_id, &i.decision, &i.reason)
        .await
        .map(Json)
        .map_err(error)
}
async fn findings(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, false).await?;
    a.1.findings(&a.2, &actor).await.map(Json).map_err(error)
}
async fn report(
    State(a): State<App>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.report_finding(&a.2, &actor, &input)
        .await
        .map(Json)
        .map_err(error)
}
async fn dispose(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(d): Json<Disposition>,
) -> Result<Json<Value>, StatusCode> {
    let actor = staff(&a, &h, true).await?;
    a.1.dispose_finding(
        &a.2,
        &actor,
        id,
        &d.status,
        &d.disposition,
        d.evidence_ref.as_deref(),
    )
    .await
    .map(Json)
    .map_err(error)
}
