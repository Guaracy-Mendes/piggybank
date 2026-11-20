use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::entitlements::Entitlements;
use bank_core::Core;
use bank_documents::{Terms, offers::*};
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::{AccountId, ActorContext};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
type App = Arc<(IdentityService, Core, Terms, Entitlements)>;
pub fn router(
    identity: IdentityService,
    core: Core,
    terms: Terms,
    controls: Entitlements,
) -> Router {
    Router::new()
        .route(
            "/api/v1/staff/finance/periodic-offers",
            get(list).post(stage),
        )
        .route("/api/v1/staff/finance/periodic-offers/{id}", get(detail))
        .route(
            "/api/v1/staff/finance/periodic-offers/{id}/approval",
            post(approve),
        )
        .route(
            "/api/v1/staff/finance/periodic-offers/{id}/publication",
            post(publish),
        )
        .route("/api/v1/accounts/{id}/periodic-offers", get(customer_list))
        .route(
            "/api/v1/accounts/{id}/periodic-offers/{offerId}",
            get(customer_detail),
        )
        .route(
            "/api/v1/accounts/{id}/periodic-offers/{offerId}/enrollment",
            get(enrollment).post(enroll),
        )
        .route(
            "/api/v1/accounts/{id}/periodic-offers/{offerId}/withdrawal",
            post(withdraw),
        )
        .with_state(Arc::new((identity, core, terms, controls)))
}
fn error(e: anyhow::Error) -> StatusCode {
    if e.downcast_ref::<OfferInputError>().is_some() {
        return StatusCode::UNPROCESSABLE_ENTITY;
    }
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(db)) => match db.code().as_deref() {
            Some("P0002") => StatusCode::NOT_FOUND,
            Some("P5401") => StatusCode::UNPROCESSABLE_ENTITY,
            Some("P0001" | "23505" | "23514" | "23503") => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::CONFLICT,
    }
}
fn key(h: &HeaderMap) -> Result<Uuid, StatusCode> {
    h.get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or(StatusCode::BAD_REQUEST)
}
async fn staff(
    a: &App,
    h: &HeaderMap,
    write: bool,
    checker: bool,
) -> Result<ActorContext, StatusCode> {
    let identity = a.0.authenticate(h, Trust::Workforce, write).await?;
    let allowed = if checker {
        a.3.finance_approval_authorized(&identity.actor).await
    } else {
        a.3.finance_authorized(&identity.actor, write).await
    }
    .map_err(error)?;
    if !allowed {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(identity.actor)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Digest {
    details_digest: String,
}
async fn list(State(a): State<App>, h: HeaderMap) -> Result<Json<Vec<OfferRequest>>, StatusCode> {
    let actor = staff(&a, &h, false, false).await?;
    a.2.periodic_offer_requests(&a.3, &actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn detail(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<OfferRequest>, StatusCode> {
    let actor = staff(&a, &h, false, false).await?;
    a.2.periodic_offer_request(&a.3, &actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn stage(
    State(a): State<App>,
    h: HeaderMap,
    Json(input): Json<OfferInput>,
) -> Result<(StatusCode, Json<OfferRequest>), StatusCode> {
    let actor = staff(&a, &h, true, false).await?;
    a.2.stage_periodic_offer(&a.3, &actor, key(&h)?, &input)
        .await
        .map(|r| (StatusCode::ACCEPTED, Json(r)))
        .map_err(error)
}
async fn approve(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<Digest>,
) -> Result<Json<OfferRequest>, StatusCode> {
    let actor = staff(&a, &h, true, true).await?;
    a.2.approve_periodic_offer(&a.3, &actor, id, &input.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
async fn publish(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<Digest>,
) -> Result<Json<OfferRequest>, StatusCode> {
    let actor = staff(&a, &h, true, false).await?;
    a.2.publish_periodic_offer(&a.3, &actor, id, &input.details_digest)
        .await
        .map(Json)
        .map_err(error)
}
async fn customer_list(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PublishedOffer>>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, false).await?.actor;
    a.2.customer_periodic_offers(&a.1, &a.3, &actor, AccountId(id))
        .await
        .map(Json)
        .map_err(error)
}
async fn customer_detail(
    State(a): State<App>,
    h: HeaderMap,
    Path((id, offer)): Path<(Uuid, Uuid)>,
) -> Result<Json<PublishedOffer>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, false).await?.actor;
    a.2.customer_periodic_offer(&a.1, &a.3, &actor, AccountId(id), offer)
        .await
        .map(Json)
        .map_err(error)
}
async fn enrollment(
    State(a): State<App>,
    h: HeaderMap,
    Path((id, offer)): Path<(Uuid, Uuid)>,
) -> Result<Json<EnrollmentView>, StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, false).await?.actor;
    a.2.periodic_enrollment(&a.1, &a.3, &actor, AccountId(id), offer)
        .await
        .map(Json)
        .map_err(error)
}
async fn enroll(
    State(a): State<App>,
    h: HeaderMap,
    Path((id, offer)): Path<(Uuid, Uuid)>,
    Json(input): Json<EnrollmentInput>,
) -> Result<(StatusCode, Json<EnrollmentView>), StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, true).await?.actor;
    a.2.request_periodic_enrollment(&a.1, &a.3, &actor, (AccountId(id), offer, key(&h)?), &input)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
async fn withdraw(
    State(a): State<App>,
    h: HeaderMap,
    Path((id, offer)): Path<(Uuid, Uuid)>,
    Json(input): Json<WithdrawalInput>,
) -> Result<(StatusCode, Json<EnrollmentView>), StatusCode> {
    let actor = a.0.authenticate(&h, Trust::Customer, true).await?.actor;
    a.2.withdraw_periodic_enrollment(&a.1, &a.3, &actor, (AccountId(id), offer, key(&h)?), &input)
        .await
        .map(|r| (StatusCode::CREATED, Json(r)))
        .map_err(error)
}
