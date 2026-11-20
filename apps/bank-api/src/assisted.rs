//! WP-34 assisted channels. Staff act for a customer only inside a session the customer authorized in a bank channel;
//! every account decision re-checks the session, the channel's confirmed operations and the common restrictions.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::{
    assisted::Assisted,
    entitlements::{Action, Entitlements},
};
use bank_core::Core;
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::{AccountId, ActorContext, PartyId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

struct AppState {
    identity: IdentityService,
    assisted: Assisted,
    core: Core,
    controls: Entitlements,
}
type App = Arc<AppState>;

pub fn router(
    identity: IdentityService,
    assisted: Assisted,
    core: Core,
    controls: Entitlements,
) -> Router {
    Router::new()
        .route("/api/v1/staff/assisted", get(staff_view).post(open))
        .route("/api/v1/staff/assisted/{id}/closure", post(staff_close))
        .route("/api/v1/staff/assisted/{id}/accounts", get(accounts))
        .route("/api/v1/staff/assisted/{id}/freeze", post(freeze))
        .route("/api/v1/assisted", get(customer_view))
        .route("/api/v1/assisted/{id}/authorization", post(authorize))
        .route("/api/v1/assisted/{id}/closure", post(customer_close))
        .with_state(Arc::new(AppState {
            identity,
            assisted,
            core,
            controls,
        }))
}
fn error(e: anyhow::Error) -> StatusCode {
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(d)) => match d.code().as_deref() {
            Some("P0002") => StatusCode::NOT_FOUND,
            Some("42501") => StatusCode::FORBIDDEN,
            Some("P0001" | "23505" | "23514") => StatusCode::CONFLICT,
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
async fn person(
    a: &App,
    h: &HeaderMap,
    trust: Trust,
    write: bool,
) -> Result<(ActorContext, PartyId), StatusCode> {
    let actor = a.identity.authenticate(h, trust, write).await?.actor;
    let person = a.controls.person(&actor).await.map_err(error)?;
    Ok((actor, person))
}
/// The session actor for one of the caller's own sessions; any other session is 404.
async fn session_actor(
    a: &App,
    h: &HeaderMap,
    id: Uuid,
    write: bool,
) -> Result<(ActorContext, PartyId), StatusCode> {
    let (staff, me) = person(a, h, Trust::Workforce, write).await?;
    let (channel, customer) = a
        .assisted
        .own_session(me, id)
        .await
        .map_err(error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((Assisted::actor(&staff, id, &channel, customer), customer))
}

async fn staff_view(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let (staff, me) = person(&a, &h, Trust::Workforce, false).await?;
    a.assisted
        .staff_view(&staff, me)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenInput {
    channel: String,
    customer_person: Uuid,
    verification: String,
    purpose: String,
}
async fn open(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<OpenInput>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let (staff, me) = person(&a, &h, Trust::Workforce, true).await?;
    let data = json!({"channel": b.channel, "customerPerson": b.customer_person, "verification": b.verification, "purpose": b.purpose});
    a.assisted
        .open(&staff, me, key(&h)?, &data)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(error)
}
async fn staff_close(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let (staff, me) = person(&a, &h, Trust::Workforce, true).await?;
    a.assisted
        .close(&staff, me, id)
        .await
        .map(Json)
        .map_err(error)
}
/// The represented customer's accounts the session may read, with balances; each read is a recorded decision.
async fn accounts(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let (actor, customer) = session_actor(&a, &h, id, false).await?;
    let mut list = vec![];
    for account in a.core.accounts_for(customer).await.map_err(error)? {
        let authority = a
            .core
            .authority(account.id, Action::ReadAccount)
            .await
            .map_err(error)?;
        if !a
            .controls
            .authorize(&actor, Action::ReadAccount, Some(&authority), account.id.0)
            .await
            .map_err(error)?
        {
            // An inactive or expired session reads nothing (the denial is recorded against the session).
            return Err(StatusCode::FORBIDDEN);
        }
        let balance = a.core.balance(account.id).await.map_err(error)?;
        list.push(json!({"account": account, "balance": balance}));
    }
    Ok(Json(json!({"accounts": list})))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FreezeInput {
    account_id: Uuid,
}
/// Protective debit freeze requested by the customer through the channel; an existing credit restriction is kept.
async fn freeze(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<FreezeInput>,
) -> Result<StatusCode, StatusCode> {
    let (actor, customer) = session_actor(&a, &h, id, true).await?;
    let account = a
        .core
        .accounts_for(customer)
        .await
        .map_err(error)?
        .into_iter()
        .find(|x| x.id.0 == b.account_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    a.core
        .restrict(
            &actor,
            &a.controls,
            AccountId(b.account_id),
            true,
            account.credit_blocked,
        )
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn customer_view(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let (_, me) = person(&a, &h, Trust::Customer, false).await?;
    a.assisted.customer_view(me).await.map(Json).map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    approve: bool,
}
async fn authorize(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Approval>,
) -> Result<Json<Value>, StatusCode> {
    let (customer, me) = person(&a, &h, Trust::Customer, true).await?;
    a.assisted
        .authorize(&customer, me, id, b.approve)
        .await
        .map(Json)
        .map_err(error)
}
async fn customer_close(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let (customer, me) = person(&a, &h, Trust::Customer, true).await?;
    a.assisted
        .close(&customer, me, id)
        .await
        .map(Json)
        .map_err(error)
}
