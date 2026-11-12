use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use bank_identity::{IdentityService, Trust};
use bank_journeys::{ApplicantProfile, Application, LocalJourneys};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Arc<LocalJourneys>)>;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Create {
    product_version_id: Uuid,
    marketing: bool,
    attribution: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    version: i64,
    profile: ApplicantProfile,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    content: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Acceptance {
    terms_id: Uuid,
}
pub fn router(identity: IdentityService, journeys: Arc<LocalJourneys>) -> Router {
    Router::new()
        .route("/api/v1/products", get(products))
        .route("/api/v1/products/{id}/terms", get(current_terms))
        .route("/api/v1/terms/{id}", get(terms_version))
        .route("/api/v1/applications", get(list).post(create))
        .route("/api/v1/applications/{id}", get(detail))
        .route("/api/v1/applications/{id}/profile", put(save))
        .route("/api/v1/applications/{id}/evidence", post(evidence))
        .route("/api/v1/applications/{id}/terms", post(accept))
        .route("/api/v1/applications/{id}/submit", post(submit))
        .route(
            "/api/v1/applications/{id}/marketing-withdrawal",
            post(withdraw_marketing),
        )
        .with_state(Arc::new((identity, journeys)))
}
fn error(e: anyhow::Error) -> StatusCode {
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(db))
            if matches!(
                db.code().as_deref(),
                Some("P0001" | "23505" | "23514" | "23503")
            ) =>
        {
            StatusCode::CONFLICT
        }
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::CONFLICT,
    }
}
fn key(headers: &HeaderMap) -> Result<&str, StatusCode> {
    headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)
}
async fn products(
    State(app): State<App>,
) -> Result<Json<Vec<bank_core::products::ProductVersion>>, StatusCode> {
    app.1
        .core
        .products(chrono::Utc::now().date_naive())
        .await
        .map(Json)
        .map_err(error)
}
async fn current_terms(
    State(app): State<App>,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_documents::TermsDraft>, StatusCode> {
    app.1
        .terms
        .current(id, chrono::Utc::now().date_naive())
        .await
        .map(Json)
        .map_err(error)
}
async fn terms_version(
    State(app): State<App>,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_documents::TermsDraft>, StatusCode> {
    app.1.terms.version(id).await.map(Json).map_err(error)
}
async fn list(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<Vec<Application>>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, false).await?;
    app.1.list(&identity.actor).await.map(Json).map_err(error)
}
async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<Create>,
) -> Result<(StatusCode, Json<Application>), StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    app.1
        .create(
            &identity.actor,
            key(&headers)?,
            body.product_version_id,
            body.marketing,
            body.attribution,
        )
        .await
        .map(|app| (StatusCode::CREATED, Json(app)))
        .map_err(error)
}
async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Application>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, false).await?;
    app.1
        .get(&identity.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn save(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Profile>,
) -> Result<Json<Application>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    app.1
        .save(&identity.actor, id, body.version, body.profile)
        .await
        .map(Json)
        .map_err(error)
}
async fn evidence(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Evidence>,
) -> Result<Json<Application>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    app.1
        .attach_evidence(&identity.actor, id, body.content.into_bytes())
        .await
        .map(Json)
        .map_err(error)
}
async fn accept(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Acceptance>,
) -> Result<Json<Application>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    app.1
        .accept_terms(&identity.actor, id, body.terms_id)
        .await
        .map(Json)
        .map_err(error)
}
async fn submit(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<bank_platform::operations::OperationStatus>), StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    app.1
        .submit(&identity.actor, id, key(&headers)?)
        .await
        .map(|op| (StatusCode::ACCEPTED, Json(op)))
        .map_err(error)
}
async fn withdraw_marketing(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    app.1
        .withdraw_marketing(&identity.actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
