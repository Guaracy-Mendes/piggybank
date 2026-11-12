use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::cases::{CaseRecord, Cases, Disposition};
use bank_identity::{IdentityService, Trust};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(
    IdentityService,
    Cases,
    bank_controls::evidence::EvidenceService<bank_adapters::object_store::FilesystemStore>,
)>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default = "page_limit")]
    limit: i64,
}
fn page_limit() -> i64 {
    50
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Information {
    reason_code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    disposition: Disposition,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Approval {
    proposal_id: Uuid,
}
pub fn router(identity: IdentityService, cases: Cases) -> anyhow::Result<Router> {
    let store = bank_adapters::object_store::FilesystemStore::new(
        std::env::var("BANK_EVIDENCE_DIR").unwrap_or_else(|_| ".local/evidence".into()),
    )?;
    let evidence_service =
        bank_controls::evidence::EvidenceService::new(cases.pool.clone(), Arc::new(store));
    Ok(Router::new()
        .route("/api/v1/staff/cases", get(queue))
        .route("/api/v1/staff/cases/{id}", get(detail))
        .route("/api/v1/staff/cases/{id}/context", get(context))
        .route("/api/v1/staff/cases/{id}/claim", post(claim))
        .route("/api/v1/staff/cases/{id}/information", post(information))
        .route("/api/v1/staff/cases/{id}/proposals", post(propose))
        .route("/api/v1/staff/cases/{id}/approval", post(approve))
        .route("/api/v1/cases", get(customer_cases))
        .route("/api/v1/cases/{id}/information", post(provide_information))
        .route("/api/v1/staff/cases/{id}/withdrawal", post(withdraw))
        .route(
            "/api/v1/staff/cases/{id}/evidence/{evidenceId}",
            get(evidence),
        )
        .with_state(Arc::new((identity, cases, evidence_service))))
}
async fn context(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let actor = app
        .0
        .authenticate(&headers, Trust::Workforce, false)
        .await?;
    app.1
        .context(&actor.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
fn error(e: anyhow::Error) -> StatusCode {
    if e.downcast_ref::<sqlx::Error>().is_some() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::FORBIDDEN
    }
}
async fn queue(
    State(app): State<App>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<Vec<CaseRecord>>, StatusCode> {
    let identity = app
        .0
        .authenticate(&headers, Trust::Workforce, false)
        .await?;
    app.1
        .list(&identity.actor, page.limit)
        .await
        .map(Json)
        .map_err(error)
}
async fn detail(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<CaseRecord>, StatusCode> {
    let actor = app
        .0
        .authenticate(&headers, Trust::Workforce, false)
        .await?;
    app.1
        .detail(&actor.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn claim(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let actor = app.0.authenticate(&headers, Trust::Workforce, true).await?;
    app.1
        .claim(&actor.actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn information(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Information>,
) -> Result<StatusCode, StatusCode> {
    let actor = app.0.authenticate(&headers, Trust::Workforce, true).await?;
    app.1
        .request_information(&actor.actor, id, &body.reason_code)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn propose(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Proposal>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let actor = app.0.authenticate(&headers, Trust::Workforce, true).await?;
    app.1
        .propose(&actor.actor, id, body.disposition)
        .await
        .map(|id| Json(serde_json::json!({"proposalId":id})))
        .map_err(error)
}
async fn approve(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Approval>,
) -> Result<StatusCode, StatusCode> {
    let actor = app.0.authenticate(&headers, Trust::Workforce, true).await?;
    app.1
        .approve(&actor.actor, id, body.proposal_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn customer_cases(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let actor = app.0.authenticate(&headers, Trust::Customer, false).await?;
    app.1
        .customer_cases(&actor.actor)
        .await
        .map(Json)
        .map_err(error)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InformationContent {
    request_id: Uuid,
    content: String,
}
async fn provide_information(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<InformationContent>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let actor = app.0.authenticate(&headers, Trust::Customer, true).await?;
    app.1
        .provide_information(
            &actor.actor,
            id,
            body.request_id,
            body.content.into_bytes(),
            &app.2,
        )
        .await
        .map(|evidence| Json(serde_json::json!({"evidenceId":evidence})))
        .map_err(error)
}
async fn withdraw(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let actor = app.0.authenticate(&headers, Trust::Workforce, true).await?;
    app.1
        .withdraw(&actor.actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn evidence(
    State(app): State<App>,
    headers: HeaderMap,
    Path((case, id)): Path<(Uuid, Uuid)>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;
    let actor = app
        .0
        .authenticate(&headers, Trust::Workforce, false)
        .await?;
    let data = app
        .2
        .get_for_case(&actor.actor, case, id, &app.1)
        .await
        .map_err(error)?;
    Ok((
        [
            ("Content-Type", "application/octet-stream"),
            (
                "Content-Disposition",
                &format!("attachment; filename=\"evidence-{id}.bin\""),
            ),
        ],
        data,
    )
        .into_response())
}
