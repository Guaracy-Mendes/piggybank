use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_identity::{IdentityService, Trust};
use bank_payments::{Beneficiary, PaymentStatus, Payments, Quote};
use bank_platform::domain::{AccountId, Money};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
fn internal_rail() -> String {
    "INTERNAL".into()
}
type App = Arc<(IdentityService, Arc<Payments>)>;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Recipient {
    label: String,
    receiving_reference: String,
    #[serde(default = "internal_rail")]
    rail: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QuoteInput {
    payer_account_id: AccountId,
    beneficiary_id: Uuid,
    amount: Money,
    requested_date: NaiveDate,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntentInput {
    quote_id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Confirmation {
    quote_id: Uuid,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryInput {
    action: bank_payments::RecoveryAction,
    reason: String,
}
pub fn router(identity: IdentityService, payments: Arc<Payments>) -> Router {
    Router::new()
        .route("/api/v1/accounts/{id}/receiving-reference", post(receiving))
        .route(
            "/api/v1/beneficiaries",
            get(beneficiaries).post(add_beneficiary),
        )
        .route(
            "/api/v1/beneficiaries/{id}/revocation",
            post(revoke_beneficiary),
        )
        .route("/api/v1/payment-quotes", post(quote))
        .route("/api/v1/payment-credits", get(customer_credits))
        .route("/api/v1/staff/payment-credits", get(staff_credits))
        .route("/api/v1/payments", get(list).post(create))
        .route("/api/v1/payments/{id}", get(detail))
        .route("/api/v1/payments/{id}/approval", post(challenge))
        .route(
            "/api/v1/payments/{id}/draft-cancellation",
            post(cancel_draft),
        )
        .route("/api/v1/staff/payment-evidence-quarantine", get(quarantine))
        .route("/api/v1/payments/{id}/confirmation", post(confirm))
        .route("/api/v1/payments/{id}/cancellation", post(cancellation))
        .route(
            "/api/v1/payments/{id}/recovery-requests",
            get(customer_recovery_history),
        )
        .route(
            "/api/v1/staff/payments/{id}/recovery-requests",
            get(staff_recovery_history).post(staff_recover),
        )
        .route("/api/v1/staff/payment-reconciliation", get(reconciliation))
        .route("/api/v1/staff/payments", get(staff_list))
        .route("/api/v1/staff/payments/{id}", get(staff_detail))
        .route("/api/v1/providers/simulated/payments", post(rail_callback))
        .route("/api/v1/staff/rails", get(rails).post(rail_proposal))
        .route("/api/v1/staff/rails/pause", post(rail_pause))
        .route("/api/v1/staff/rails/resume-proposal", post(rail_resume))
        .route(
            "/api/v1/staff/rails/changes/{id}/approval",
            post(rail_approve),
        )
        .route(
            "/api/v1/staff/rails/changes/{id}/application",
            post(rail_apply),
        )
        .with_state(Arc::new((identity, payments)))
}
fn error(e: anyhow::Error) -> StatusCode {
    if e.downcast_ref::<bank_payments::ResourceUnavailable>()
        .is_some()
    {
        return StatusCode::NOT_FOUND;
    }
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(d))
            if matches!(
                d.code().as_deref(),
                Some("P0001" | "23505" | "23514" | "22003")
            ) =>
        {
            StatusCode::CONFLICT
        }
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::CONFLICT,
    }
}
fn key(h: &HeaderMap) -> Result<Uuid, StatusCode> {
    h.get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or(StatusCode::BAD_REQUEST)
}
async fn receiving(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    let authority =
        s.1.core
            .authority(
                AccountId(id),
                bank_controls::entitlements::Action::ReadAccount,
            )
            .await
            .map_err(error)?;
    if !s
        .1
        .controls
        .authorize(
            &a.actor,
            bank_controls::entitlements::Action::ReadAccount,
            Some(&authority),
            id,
        )
        .await
        .map_err(error)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    s.1.core
        .receiving_reference(&a.actor, &s.1.controls, AccountId(id))
        .await
        .map(|r| Json(serde_json::json!({"reference":r})))
        .map_err(error)
}
async fn beneficiaries(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<Beneficiary>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?;
    s.1.beneficiaries(&a.actor).await.map(Json).map_err(error)
}
async fn add_beneficiary(
    State(s): State<App>,
    h: HeaderMap,
    Json(b): Json<Recipient>,
) -> Result<(StatusCode, Json<Beneficiary>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    s.1.add_rail_beneficiary(
        &a.actor,
        key(&h)?,
        &b.label,
        &b.receiving_reference,
        &b.rail,
    )
    .await
    .map(|v| (StatusCode::CREATED, Json(v)))
    .map_err(error)
}
async fn revoke_beneficiary(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    s.1.revoke_beneficiary(&a.actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn quote(
    State(s): State<App>,
    h: HeaderMap,
    Json(b): Json<QuoteInput>,
) -> Result<(StatusCode, Json<Quote>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    s.1.quote(
        &a.actor,
        b.payer_account_id,
        b.beneficiary_id,
        b.amount,
        b.requested_date,
    )
    .await
    .map(|v| (StatusCode::CREATED, Json(v)))
    .map_err(error)
}
async fn create(
    State(s): State<App>,
    h: HeaderMap,
    Json(b): Json<IntentInput>,
) -> Result<(StatusCode, Json<PaymentStatus>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    s.1.create_intent(&a.actor, key(&h)?, b.quote_id)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(error)
}
async fn list(State(s): State<App>, h: HeaderMap) -> Result<Json<Vec<PaymentStatus>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?;
    s.1.list(&a.actor).await.map(Json).map_err(error)
}
async fn detail(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<PaymentStatus>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?;
    s.1.detail(&a.actor, id).await.map(Json).map_err(error)
}
async fn challenge(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<PaymentStatus>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    s.1.challenge(&a.actor, id).await.map(Json).map_err(error)
}
async fn confirm(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<Confirmation>,
) -> Result<(StatusCode, Json<PaymentStatus>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    if s.1.detail(&a.actor, id).await.map_err(error)?.quote_id != body.quote_id {
        return Err(StatusCode::CONFLICT);
    }
    s.1.confirm(&a.actor, id)
        .await
        .map(|v| (StatusCode::ACCEPTED, Json(v)))
        .map_err(error)
}
async fn staff_list(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<PaymentStatus>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    if !s
        .1
        .controls
        .payment_inquiry_authorized(&a.actor)
        .await
        .map_err(error)?
    {
        return Err(StatusCode::FORBIDDEN);
    }
    s.1.staff_list(&a.actor).await.map(Json).map_err(error)
}
async fn staff_detail(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<PaymentStatus>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    if !s
        .1
        .controls
        .payment_inquiry_authorized(&a.actor)
        .await
        .map_err(error)?
    {
        return Err(StatusCode::FORBIDDEN);
    }
    s.1.staff_detail(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}

async fn rails(State(s): State<App>, h: HeaderMap) -> Result<Json<serde_json::Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    let changes =
        s.1.rail_changes(&a.actor)
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?;
    let policy = s.1.rail_policy().await.map_err(error)?;
    Ok(Json(
        serde_json::json!({"snapshot":policy,"changes":changes["items"]}),
    ))
}
async fn rail_proposal(
    State(s): State<App>,
    h: HeaderMap,
    Json(draft): Json<bank_payments::policy::RailPolicyVersion>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.propose_rail_policy(&a.actor, &draft)
        .await
        .map(|id| {
            (
                StatusCode::CREATED,
                Json(serde_json::json!({"proposalId":id})),
            )
        })
        .map_err(error)
}
async fn rail_pause(State(s): State<App>, h: HeaderMap) -> Result<StatusCode, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.pause_rail(&a.actor)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn rail_resume(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.propose_rail_resume(&a.actor)
        .await
        .map(|id| {
            (
                StatusCode::CREATED,
                Json(serde_json::json!({"proposalId":id})),
            )
        })
        .map_err(error)
}
async fn rail_approve(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.approve_rail_change(&a.actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn rail_apply(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.apply_rail_change(&a.actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}

async fn rail_callback(
    State(s): State<App>,
    Json(packet): Json<bank_platform::authenticity::SignedDocument>,
) -> Result<StatusCode, StatusCode> {
    // WP-39: each provider signs with its own key (evidence without a provider is the original provider's).
    let key = match bank_rails::evidence_provider(&packet) {
        Some(provider) => bank_rails::provider_key(provider).ok_or(StatusCode::UNAUTHORIZED)?,
        None if packet.body.get("provider").is_none() => bank_rails::signing_key(),
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    packet
        .verify(key.as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    s.1.accept_rail_evidence(&packet)
        .await
        .map(|_| StatusCode::ACCEPTED)
        .map_err(error)
}

async fn customer_credits(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_payments::CreditStatus>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?;
    s.1.customer_credits(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn staff_credits(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_payments::CreditStatus>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.staff_credits(&a.actor).await.map(Json).map_err(error)
}

async fn cancel_draft(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<PaymentStatus>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    s.1.cancel_draft(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn quarantine(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    s.1.staff_quarantine(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}

async fn cancellation(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<bank_payments::RecoveryRequest>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, true).await?;
    s.1.customer_cancellation(&a.actor, id, key(&h)?)
        .await
        .map(|v| (StatusCode::ACCEPTED, Json(v)))
        .map_err(error)
}
async fn customer_recovery_history(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<bank_payments::RecoveryRequest>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Customer, false).await?;
    s.1.recovery_history(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn staff_recovery_history(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<bank_payments::RecoveryRequest>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    if !s
        .1
        .controls
        .payment_inquiry_authorized(&a.actor)
        .await
        .map_err(error)?
    {
        return Err(StatusCode::FORBIDDEN);
    }
    s.1.recovery_history(&a.actor, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn staff_recover(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<RecoveryInput>,
) -> Result<(StatusCode, Json<bank_payments::RecoveryRequest>), StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, true).await?;
    s.1.recovery_authorized(&a.actor).await.map_err(|e| {
        if e.downcast_ref::<sqlx::Error>().is_some() {
            error(e)
        } else {
            StatusCode::FORBIDDEN
        }
    })?;
    s.1.staff_recovery(&a.actor, id, key(&h)?, b.action, &b.reason)
        .await
        .map(|v| (StatusCode::ACCEPTED, Json(v)))
        .map_err(error)
}
async fn reconciliation(
    State(s): State<App>,
    h: HeaderMap,
) -> Result<Json<Vec<bank_payments::ReconciliationBreak>>, StatusCode> {
    let a = s.0.authenticate(&h, Trust::Workforce, false).await?;
    if !s
        .1
        .controls
        .payment_inquiry_authorized(&a.actor)
        .await
        .map_err(error)?
    {
        return Err(StatusCode::FORBIDDEN);
    }
    s.1.staff_reconciliation(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
