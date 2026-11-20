use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_finance::{ExportStatus, Finance, RetryCommand};
use bank_identity::{IdentityService, Trust};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Arc<Finance>)>;
pub fn router(identity: IdentityService, finance: Arc<Finance>) -> Router {
    Router::new()
        .route("/api/v1/staff/finance/exports", get(list))
        .route("/api/v1/staff/finance/exports/{id}", get(detail))
        .route(
            "/api/v1/staff/finance/exports/{id}/recovery-requests",
            post(recover),
        )
        .route("/api/v1/staff/finance/breaks", get(breaks))
        .route(
            "/api/v1/staff/finance/reports",
            get(reports).post(create_report),
        )
        .route("/api/v1/staff/finance/reports/{id}", get(report_detail))
        .route(
            "/api/v1/staff/finance/reports/{id}/items",
            get(report_items),
        )
        .route("/api/v1/staff/finance/liquidity", get(liquidity))
        .route(
            "/api/v1/staff/finance/report-revisions",
            get(revisions).post(propose_revision),
        )
        .route("/api/v1/staff/finance/report-revisions/{id}", get(revision))
        .route(
            "/api/v1/staff/finance/report-revisions/{id}/approval",
            post(approve_revision),
        )
        .route(
            "/api/v1/staff/finance/report-revisions/{id}/publication",
            post(publish_revision),
        )
        .route(
            "/api/v1/staff/finance/reconciliations",
            get(reconciliations).post(create_reconciliation),
        )
        .route(
            "/api/v1/staff/finance/reconciliations/{id}",
            get(reconciliation),
        )
        .route(
            "/api/v1/staff/finance/reconciliations/{id}/items",
            get(reconciliation_items),
        )
        .route(
            "/api/v1/staff/finance/credit-reconciliations",
            get(credit_reconciliations).post(create_credit_reconciliation),
        )
        .route(
            "/api/v1/staff/finance/credit-reconciliations/{id}",
            get(credit_reconciliation),
        )
        .route(
            "/api/v1/staff/finance/credit-reconciliations/{id}/items",
            get(credit_reconciliation_items),
        )
        .route(
            "/api/v1/staff/finance/reconciliation-groups",
            get(group_reconciliations).post(create_group_reconciliation),
        )
        .route(
            "/api/v1/staff/finance/reconciliation-groups/{id}",
            get(group_reconciliation),
        )
        .route(
            "/api/v1/staff/finance/reconciliation-groups/{id}/items",
            get(group_reconciliation_items),
        )
        .route(
            "/api/v1/staff/finance/periodic-policies",
            get(periodic_policies).post(propose_periodic_policy),
        )
        .route(
            "/api/v1/staff/finance/periodic-policies/{id}",
            get(periodic_policy),
        )
        .route(
            "/api/v1/staff/finance/periodic-policies/{id}/approval",
            post(approve_periodic_policy),
        )
        .route(
            "/api/v1/staff/finance/periodic-policies/{id}/publication",
            post(publish_periodic_policy),
        )
        .route(
            "/api/v1/staff/finance/periodic-policies/{id}/preview",
            post(preview_periodic_policy),
        )
        .route(
            "/api/v1/staff/finance/corrections",
            get(corrections).post(propose_correction),
        )
        .route("/api/v1/staff/finance/corrections/{id}", get(correction))
        .route(
            "/api/v1/staff/finance/corrections/{id}/approval",
            post(approve_correction),
        )
        .route(
            "/api/v1/staff/finance/corrections/{id}/execution",
            post(execute_correction),
        )
        .route(
            "/api/v1/staff/finance/periodic-bases",
            get(periodic_bases).post(capture_periodic_basis),
        )
        .route(
            "/api/v1/staff/finance/periodic-bases/{id}",
            get(periodic_basis),
        )
        .route(
            "/api/v1/staff/finance/periodic-bases/{id}/items",
            get(periodic_basis_items),
        )
        .route(
            "/api/v1/staff/finance/audit-delivery",
            get(audit_delivery_status),
        )
        .route(
            "/api/v1/staff/finance/audit-delivery/{id}",
            get(audit_delivery),
        )
        .route(
            "/api/v1/staff/finance/periodic-activations",
            get(periodic_activations).post(propose_periodic_activation),
        )
        .route(
            "/api/v1/staff/finance/periodic-activations/{id}",
            get(periodic_activation),
        )
        .route(
            "/api/v1/staff/finance/periodic-activations/{id}/approval",
            post(approve_periodic_activation),
        )
        .route(
            "/api/v1/staff/finance/periodic-activations/{id}/execution",
            post(execute_periodic_activation),
        )
        .route(
            "/api/v1/staff/finance/periodic-activations/{id}/stop",
            post(stop_periodic_activation),
        )
        .route(
            "/api/v1/staff/finance/periodic-runs",
            get(periodic_runs).post(queue_periodic_run),
        )
        .route(
            "/api/v1/staff/finance/periodic-runs/{id}",
            get(periodic_run),
        )
        .with_state(Arc::new((identity, finance)))
}
fn error(e: anyhow::Error) -> StatusCode {
    if e.downcast_ref::<bank_finance::audit::AuditDeliveryConflict>()
        .is_some()
    {
        return StatusCode::CONFLICT;
    }
    if e.downcast_ref::<bank_core::accruals::basis::BasisConflict>()
        .is_some()
    {
        return StatusCode::CONFLICT;
    }
    if e.downcast_ref::<bank_core::accruals::basis::BasisInputError>()
        .is_some()
        || e.downcast_ref::<bank_core::accruals::basis::BasisLimit>()
            .is_some()
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    if e.downcast_ref::<bank_core::corrections::CorrectionConflict>()
        .is_some()
    {
        return StatusCode::CONFLICT;
    }
    if e.downcast_ref::<bank_core::corrections::CorrectionInputError>()
        .is_some()
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    if e.downcast_ref::<bank_core::accruals::PeriodicConflict>()
        .is_some()
    {
        return StatusCode::CONFLICT;
    }
    if e.downcast_ref::<bank_core::accruals::PeriodicInputError>()
        .is_some()
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    if e.downcast_ref::<bank_finance::settlement::SettlementUnavailable>()
        .is_some()
    {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    if e.downcast_ref::<bank_finance::reconciliation::ReconciliationKeyConflict>()
        .is_some()
    {
        return StatusCode::CONFLICT;
    }
    if e.downcast_ref::<bank_finance::reconciliation::ReconciliationInputError>()
        .is_some()
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    if e.downcast_ref::<bank_finance::revisions::RevisionConflict>()
        .is_some()
    {
        return StatusCode::CONFLICT;
    }
    if e.downcast_ref::<bank_core::finance_snapshot::SnapshotLimitExceeded>()
        .is_some()
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    if e.downcast_ref::<bank_finance::reports::ReportKeyConflict>()
        .is_some()
    {
        return StatusCode::CONFLICT;
    }
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(d)) if d.code().as_deref() == Some("P0002") => {
            StatusCode::NOT_FOUND
        }
        Some(sqlx::Error::Database(d))
            if matches!(d.code().as_deref(), Some("P5501" | "P5401")) =>
        {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        Some(sqlx::Error::Database(d))
            if matches!(d.code().as_deref(), Some("23505" | "23514" | "P0001")) =>
        {
            StatusCode::CONFLICT
        }
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::FORBIDDEN,
    }
}
async fn list(State(s): State<App>, h: HeaderMap) -> Result<Json<Vec<ExportStatus>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.staff_exports(&a.actor).await.map(Json).map_err(error)
}
async fn detail(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.staff_detail(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    reason: String,
}
async fn recover(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Recovery>,
) -> Result<(StatusCode, Json<RetryCommand>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.retry_export(&a.actor, id, key, &body.reason)
        .await
        .map(|r| (StatusCode::ACCEPTED, Json(r)))
        .map_err(error)
}
async fn breaks(State(s): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.staff_breaks(&a.actor).await.map(Json).map_err(error)
}
async fn reports(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_finance::reports::ReportSummary>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.staff_reports(&a.actor).await.map(Json).map_err(error)
}
async fn create_report(
    State(s): State<App>,
    h: HeaderMap,
    Json(range): Json<bank_core::finance_snapshot::ReportRange>,
) -> Result<(StatusCode, Json<bank_finance::reports::ReportDetail>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    range
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    s.1.create_report(&a.actor, key, range)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
async fn report_detail(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::reports::ReportDetail>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.report_detail(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<i32>,
    limit: Option<i64>,
}
async fn report_items(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<bank_finance::reports::ReportPage>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.report_items(
        &a.actor,
        id,
        page.after.unwrap_or(0),
        page.limit.unwrap_or(25),
    )
    .await
    .map(Json)
    .map_err(error)
}
async fn liquidity(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<bank_finance::liquidity::LiquiditySnapshot>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.staff_liquidity(&a.actor).await.map(Json).map_err(error)
}
async fn revisions(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_finance::revisions::RevisionRecord>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.report_revisions(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn revision(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::revisions::RevisionRecord>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.report_revision(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn propose_revision(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::revisions::RevisionInput>,
) -> Result<(StatusCode, Json<bank_finance::revisions::RevisionRecord>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.propose_report_revision(&a.actor, key, &body)
        .await
        .map(|r| (StatusCode::ACCEPTED, Json(r)))
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RevisionScope {
    details_digest: String,
}
async fn approve_revision(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::revisions::RevisionRecord>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.approve_report_revision(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
async fn publish_revision(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::revisions::RevisionRecord>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.publish_report_revision(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReconciliationFilter {
    report_id: Option<Uuid>,
}
async fn reconciliations(
    State(s): State<App>,
    h: HeaderMap,
    Query(filter): Query<ReconciliationFilter>,
) -> Result<Json<Vec<bank_finance::reconciliation::ReconciliationSummary>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.reconciliations(&a.actor, filter.report_id)
        .await
        .map(Json)
        .map_err(error)
}
async fn create_reconciliation(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::reconciliation::ReconciliationInput>,
) -> Result<
    (
        StatusCode,
        Json<bank_finance::reconciliation::ReconciliationDetail>,
    ),
    StatusCode,
> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.create_reconciliation(&a.actor, key, body)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
async fn reconciliation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::reconciliation::ReconciliationDetail>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.reconciliation_detail(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn reconciliation_items(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<bank_finance::reconciliation::ReconciliationPage>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.reconciliation_items(
        &a.actor,
        id,
        page.after.unwrap_or(0),
        page.limit.unwrap_or(25),
    )
    .await
    .map(Json)
    .map_err(error)
}

async fn credit_reconciliations(
    State(s): State<App>,
    h: HeaderMap,
    Query(filter): Query<ReconciliationFilter>,
) -> Result<Json<Vec<bank_finance::credit_reconciliation::CreditReconciliationSummary>>, StatusCode>
{
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.credit_reconciliations(&a.actor, filter.report_id)
        .await
        .map(Json)
        .map_err(error)
}
async fn create_credit_reconciliation(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::reconciliation::ReconciliationInput>,
) -> Result<
    (
        StatusCode,
        Json<bank_finance::credit_reconciliation::CreditReconciliationDetail>,
    ),
    StatusCode,
> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.create_credit_reconciliation(&a.actor, key, body)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
async fn credit_reconciliation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::credit_reconciliation::CreditReconciliationDetail>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.credit_reconciliation_detail(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn credit_reconciliation_items(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<bank_finance::credit_reconciliation::CreditReconciliationPage>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.credit_reconciliation_items(
        &a.actor,
        id,
        page.after.unwrap_or(0),
        page.limit.unwrap_or(25),
    )
    .await
    .map(Json)
    .map_err(error)
}

async fn group_reconciliations(
    State(s): State<App>,
    h: HeaderMap,
    Query(filter): Query<ReconciliationFilter>,
) -> Result<Json<Vec<bank_finance::group_reconciliation::GroupSummary>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.group_reconciliations(&a.actor, filter.report_id)
        .await
        .map(Json)
        .map_err(error)
}
async fn create_group_reconciliation(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::group_reconciliation::GroupInput>,
) -> Result<
    (
        StatusCode,
        Json<bank_finance::group_reconciliation::GroupDetail>,
    ),
    StatusCode,
> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.create_group_reconciliation(&a.actor, key, body)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
async fn group_reconciliation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::group_reconciliation::GroupDetail>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.group_reconciliation_detail(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn group_reconciliation_items(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<Page>,
) -> Result<Json<bank_finance::group_reconciliation::GroupPage>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.group_reconciliation_items(
        &a.actor,
        id,
        page.after.unwrap_or(0),
        page.limit.unwrap_or(25),
    )
    .await
    .map(Json)
    .map_err(error)
}

async fn periodic_policies(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_finance::periodic::PolicyRequest>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_policy_requests(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn periodic_policy(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::periodic::PolicyRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_policy_request(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn propose_periodic_policy(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::periodic::PolicyInput>,
) -> Result<(StatusCode, Json<bank_finance::periodic::PolicyRequest>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.propose_periodic_policy(&a.actor, key, &body)
        .await
        .map(|r| (StatusCode::ACCEPTED, Json(r)))
        .map_err(error)
}
async fn approve_periodic_policy(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::periodic::PolicyRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.approve_periodic_policy(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
async fn publish_periodic_policy(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::periodic::PolicyRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.publish_periodic_policy(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
async fn preview_periodic_policy(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<bank_core::accruals::PreviewInput>,
) -> Result<Json<bank_core::accruals::PeriodicPreview>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.preview_periodic_policy(&a.actor, id, &body)
        .await
        .map(Json)
        .map_err(error)
}

async fn corrections(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_finance::corrections::CorrectionRequest>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.correction_requests(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn correction(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::corrections::CorrectionRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.correction_request(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn propose_correction(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::corrections::CorrectionInput>,
) -> Result<
    (
        StatusCode,
        Json<bank_finance::corrections::CorrectionRequest>,
    ),
    StatusCode,
> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.propose_correction(&a.actor, key, &body)
        .await
        .map(|r| (StatusCode::ACCEPTED, Json(r)))
        .map_err(error)
}
async fn approve_correction(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::corrections::CorrectionRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.approve_correction(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
async fn execute_correction(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::corrections::CorrectionRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.execute_correction(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BasisFilter {
    policy_id: Option<Uuid>,
    account_id: Option<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BasisPageQuery {
    after: Option<u32>,
    limit: Option<u32>,
}
async fn periodic_bases(
    State(s): State<App>,
    h: HeaderMap,
    Query(filter): Query<BasisFilter>,
) -> Result<Json<Vec<bank_core::accruals::basis::BasisDetail>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_bases(&a.actor, filter.policy_id, filter.account_id)
        .await
        .map(Json)
        .map_err(error)
}
async fn capture_periodic_basis(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_core::accruals::basis::BasisInput>,
) -> Result<(StatusCode, Json<bank_core::accruals::basis::BasisDetail>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    s.1.capture_periodic_basis(&a.actor, key, &body)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
async fn periodic_basis(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_core::accruals::basis::BasisDetail>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_basis(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn periodic_basis_items(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<BasisPageQuery>,
) -> Result<Json<bank_core::accruals::basis::BasisPage>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_basis_items(
        &a.actor,
        id,
        page.after.unwrap_or(0),
        page.limit.unwrap_or(25),
    )
    .await
    .map(Json)
    .map_err(error)
}

async fn audit_delivery_status(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<bank_finance::audit::AuditDeliveryStatus>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.audit_delivery_status(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn audit_delivery(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::audit::AuditDeliveryItem>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.audit_delivery(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}

fn periodic_key(h: &HeaderMap) -> Result<Uuid, StatusCode> {
    h.get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .filter(|id| !id.is_nil())
        .ok_or(StatusCode::BAD_REQUEST)
}
async fn periodic_activations(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_finance::periodic_execution::ActivationRequest>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_activations(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn periodic_activation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::periodic_execution::ActivationRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_activation(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn propose_periodic_activation(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::periodic_execution::ActivationInput>,
) -> Result<
    (
        StatusCode,
        Json<bank_finance::periodic_execution::ActivationRequest>,
    ),
    StatusCode,
> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.propose_periodic_activation(&a.actor, periodic_key(&h)?, &body)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
async fn approve_periodic_activation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::periodic_execution::ActivationRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.approve_periodic_activation(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
async fn execute_periodic_activation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<RevisionScope>,
) -> Result<Json<bank_finance::periodic_execution::ActivationRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.execute_periodic_activation(&a.actor, id, &body.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeriodicStop {
    reason: String,
}
async fn stop_periodic_activation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<PeriodicStop>,
) -> Result<Json<bank_finance::periodic_execution::ActivationRequest>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.periodic_commands
        .stop(&s.1.controls, &a.actor, id, &body.reason)
        .await
        .map_err(error)?;
    s.1.periodic_activation(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn periodic_runs(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_finance::periodic_execution::PeriodRun>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_runs(&a.actor).await.map(Json).map_err(error)
}
async fn periodic_run(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_finance::periodic_execution::PeriodRun>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.periodic_run(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn queue_periodic_run(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<bank_finance::periodic_execution::PeriodInput>,
) -> Result<
    (
        StatusCode,
        Json<bank_finance::periodic_execution::PeriodRun>,
    ),
    StatusCode,
> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.queue_periodic_run(&a.actor, periodic_key(&h)?, &body)
        .await
        .map(|r| (StatusCode::ACCEPTED, Json(r)))
        .map_err(error)
}
