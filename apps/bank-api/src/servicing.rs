use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::entitlements::Action;
use bank_core::{
    Balance,
    accounts::AccountSummary,
    inquiry::{HistoryCursor, HistoryPage},
    profile::CustomerProfile,
};
use bank_identity::{IdentityService, Trust};
use bank_journeys::{LocalJourneys, profile::ContactStatus};
use bank_platform::domain::AccountId;
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Arc<LocalJourneys>)>;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryQuery {
    from: NaiveDate,
    to: NaiveDate,
    #[serde(default = "page_limit")]
    limit: i64,
    snapshot_id: Option<Uuid>,
    after: Option<i32>,
}
fn page_limit() -> i64 {
    50
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contact {
    email: String,
}
pub fn router(identity: IdentityService, journeys: Arc<LocalJourneys>) -> Router {
    Router::new()
        .route("/api/v1/accounts", get(accounts))
        .route("/api/v1/accounts/{id}/balance", get(balance))
        .route("/api/v1/accounts/{id}/transactions", get(history))
        .route(
            "/api/v1/accounts/{id}/periodic-disclosures",
            get(periodic_disclosures),
        )
        .route(
            "/api/v1/accounts/{id}/periodic-disclosures/{policyId}",
            get(periodic_disclosure),
        )
        .route(
            "/api/v1/accounts/{id}/periodic-disclosures/{policyId}/review",
            get(periodic_review).post(acknowledge_periodic_preview),
        )
        .route("/api/v1/accounts/{id}/freeze", post(freeze))
        .route("/api/v1/accounts/{id}/closure", post(close))
        .route("/api/v1/profile", get(profile))
        .route("/api/v1/profile/contact-changes", post(propose_contact))
        .route("/api/v1/profile/contact-changes/{id}", get(contact_status))
        .route(
            "/api/v1/accounts/{id}/service-requests",
            post(service_request),
        )
        .with_state(Arc::new((identity, journeys)))
}
fn error(e: anyhow::Error) -> StatusCode {
    if e.downcast_ref::<bank_documents::periodic::ReviewInputError>()
        .is_some()
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    if e.downcast_ref::<bank_core::accruals::disclosures::DisclosureLimit>()
        .is_some()
    {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }

    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(db)) if db.code().as_deref() == Some("P0002") => {
            StatusCode::NOT_FOUND
        }
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
async fn accounts(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<Vec<AccountSummary>>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, false).await?;
    let person = match app.1.controls.person(&identity.actor).await {
        Ok(person) => person,
        Err(e) if e.downcast_ref::<sqlx::Error>().is_none() => return Ok(Json(vec![])),
        Err(e) => return Err(error(e)),
    };
    app.1
        .core
        .accounts_for(person)
        .await
        .map(Json)
        .map_err(error)
}
async fn balance(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Balance>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, false).await?;
    let authority = app
        .1
        .core
        .authority(AccountId(id), Action::ReadAccount)
        .await
        .map_err(error)?;
    if !app
        .1
        .controls
        .authorize(&identity.actor, Action::ReadAccount, Some(&authority), id)
        .await
        .map_err(error)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    app.1
        .core
        .balance(AccountId(id))
        .await
        .map(Json)
        .map_err(error)
}
async fn history(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryPage>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, false).await?;
    if query.snapshot_id.is_some() != query.after.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let authority = app
        .1
        .core
        .authority(AccountId(id), Action::ReadAccount)
        .await
        .map_err(error)?;
    if !app
        .1
        .controls
        .authorize(&identity.actor, Action::ReadAccount, Some(&authority), id)
        .await
        .map_err(error)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let cursor = query
        .snapshot_id
        .zip(query.after)
        .map(|(snapshot_id, after)| HistoryCursor { snapshot_id, after });
    app.1
        .core
        .history(
            &identity.actor,
            &app.1.controls,
            AccountId(id),
            (query.from, query.to),
            query.limit,
            cursor,
        )
        .await
        .map(Json)
        .map_err(error)
}
async fn freeze(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    // Preserve any independently imposed credit restriction.
    let account = app
        .1
        .core
        .accounts_for(
            app.1
                .controls
                .person(&identity.actor)
                .await
                .map_err(error)?,
        )
        .await
        .map_err(error)?
        .into_iter()
        .find(|a| a.id.0 == id)
        .ok_or(StatusCode::NOT_FOUND)?;
    app.1
        .core
        .restrict(
            &identity.actor,
            &app.1.controls,
            account.id,
            true,
            account.credit_blocked,
        )
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn close(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    let authority = app
        .1
        .core
        .authority(AccountId(id), Action::ServiceAccount)
        .await
        .map_err(error)?;
    if !app
        .1
        .controls
        .authorize(
            &identity.actor,
            Action::ServiceAccount,
            Some(&authority),
            id,
        )
        .await
        .map_err(error)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    app.1
        .core
        .close_account(&identity.actor, &app.1.controls, AccountId(id))
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn profile(
    State(app): State<App>,
    headers: HeaderMap,
) -> Result<Json<CustomerProfile>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, false).await?;
    app.1
        .core
        .profile(&identity.actor, &app.1.controls)
        .await
        .map(Json)
        .map_err(error)
}
async fn propose_contact(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<Contact>,
) -> Result<(StatusCode, Json<ContactStatus>), StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    let key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    app.1
        .propose_contact(&identity.actor, key, body.email)
        .await
        .map(|v| (StatusCode::ACCEPTED, Json(v)))
        .map_err(error)
}
async fn contact_status(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<ContactStatus>, StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, false).await?;
    app.1
        .contact_status(&identity.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn service_request(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let identity = app.0.authenticate(&headers, Trust::Customer, true).await?;
    let authority = app
        .1
        .core
        .authority(AccountId(id), Action::ServiceAccount)
        .await
        .map_err(error)?;
    if !app
        .1
        .controls
        .authorize(
            &identity.actor,
            Action::ServiceAccount,
            Some(&authority),
            id,
        )
        .await
        .map_err(error)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let origin = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let source = bank_controls::cases::NewCase {
        kind: bank_controls::cases::CaseKind::Support,
        origin_id: origin,
        resource_id: id,
        evidence_refs: vec![],
        customer: Some((
            identity.actor.issuer.clone(),
            identity.actor.subject.clone(),
        )),
    };
    bank_controls::cases::Cases::new(app.1.controls.pool.clone())
        .ensure(&identity.actor, source)
        .await
        .map(|id| (StatusCode::CREATED, Json(serde_json::json!({"caseId":id}))))
        .map_err(error)
}

async fn periodic_disclosures(
    State(app): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<bank_core::accruals::disclosures::DisclosurePage>, StatusCode> {
    let a = app.0.authenticate(&h, Trust::Customer, false).await?;
    app.1
        .core
        .periodic_disclosures(&app.1.controls, &a.actor, AccountId(id))
        .await
        .map(Json)
        .map_err(error)
}
async fn periodic_disclosure(
    State(app): State<App>,
    h: HeaderMap,
    Path((id, policy)): Path<(Uuid, Uuid)>,
) -> Result<Json<bank_core::accruals::disclosures::PeriodicDisclosure>, StatusCode> {
    let a = app.0.authenticate(&h, Trust::Customer, false).await?;
    app.1
        .core
        .periodic_disclosure(&app.1.controls, &a.actor, AccountId(id), policy)
        .await
        .map(Json)
        .map_err(error)
}
async fn periodic_review(
    State(app): State<App>,
    h: HeaderMap,
    Path((id, policy)): Path<(Uuid, Uuid)>,
) -> Result<Json<bank_documents::periodic::PreviewReviewReceipt>, StatusCode> {
    let a = app.0.authenticate(&h, Trust::Customer, false).await?;
    app.1
        .terms
        .periodic_preview_review(
            &app.1.core,
            &app.1.controls,
            &a.actor,
            AccountId(id),
            policy,
        )
        .await
        .map(Json)
        .map_err(error)
}
async fn acknowledge_periodic_preview(
    State(app): State<App>,
    h: HeaderMap,
    Path((id, policy)): Path<(Uuid, Uuid)>,
    Json(input): Json<bank_documents::periodic::ReviewInput>,
) -> Result<
    (
        StatusCode,
        Json<bank_documents::periodic::PreviewReviewReceipt>,
    ),
    StatusCode,
> {
    let a = app.0.authenticate(&h, Trust::Customer, true).await?;
    let key = h
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or(StatusCode::BAD_REQUEST)?;
    app.1
        .terms
        .acknowledge_periodic_preview(
            &app.1.core,
            &app.1.controls,
            &a.actor,
            (AccountId(id), policy, key),
            &input,
        )
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
