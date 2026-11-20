//! WP-33 consented partner API, customer consent and delegation management, and the staff partner registry.
//!
//! Partner calls (`/partner/v1/...`) carry a client-credentials bearer token from the `partners` realm and a
//! `Piggybank-Api-Version` header. The token's `azp` must name a registered ACTIVE client. Consented calls also
//! carry `Consent-Id`, which is resolved server-side to the consenting customer; Controls re-checks that live grant
//! at every decision. A revoked, expired, foreign or suspended grant answers 403 without saying which; a resource
//! outside the grant answers 404. Partners may draft payments but never approve them.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bank_controls::entitlements::{Action, Entitlements};
use bank_core::{Core, inquiry::HistoryCursor};
use bank_identity::{
    IdentityService, Trust,
    consents::{Binding, Partners, SCOPE_ACCOUNTS, SCOPE_PAYMENTS},
};
use bank_payments::Payments;
use bank_platform::domain::{AccountId, ActorContext, Money, PartyId};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

struct AppState {
    identity: IdentityService,
    partners: Partners,
    core: Core,
    controls: Entitlements,
    payments: Arc<Payments>,
}
type App = Arc<AppState>;

pub fn router(
    identity: IdentityService,
    partners: Partners,
    core: Core,
    controls: Entitlements,
    payments: Arc<Payments>,
) -> Router {
    Router::new()
        .route("/partner/v1/consents", post(partner_request))
        .route(
            "/partner/v1/consents/{id}",
            get(partner_consent).delete(partner_withdraw),
        )
        .route("/partner/v1/accounts", get(partner_accounts))
        .route("/partner/v1/accounts/{id}/balance", get(partner_balance))
        .route(
            "/partner/v1/accounts/{id}/transactions",
            get(partner_history),
        )
        .route("/partner/v1/beneficiaries", get(partner_beneficiaries))
        .route("/partner/v1/payment-quotes", post(partner_quote))
        .route("/partner/v1/payments", post(partner_payment))
        .route("/partner/v1/payments/{id}", get(partner_payment_status))
        .route("/api/v1/consents", get(customer_consents))
        .route("/api/v1/consents/{id}", get(customer_consent))
        .route(
            "/api/v1/consents/{id}/authorization",
            post(customer_authorize),
        )
        .route("/api/v1/consents/{id}/revocation", post(customer_revoke))
        .route(
            "/api/v1/delegations",
            get(delegations).post(grant_delegation),
        )
        .route("/api/v1/delegations/reference", post(delegate_reference))
        .route(
            "/api/v1/delegations/{id}/revocation",
            post(revoke_delegation),
        )
        .route(
            "/api/v1/staff/partners",
            get(staff_partners).post(staff_propose),
        )
        .route("/api/v1/staff/partners/{id}/decision", post(staff_decide))
        .route("/api/v1/staff/consents/{id}/revocation", post(staff_revoke))
        .with_state(Arc::new(AppState {
            identity,
            partners,
            core,
            controls,
            payments,
        }))
}

fn error(e: anyhow::Error) -> StatusCode {
    if e.downcast_ref::<bank_payments::ResourceUnavailable>()
        .is_some()
    {
        return StatusCode::NOT_FOUND;
    }
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(d)) => match d.code().as_deref() {
            Some("P0002") => StatusCode::NOT_FOUND,
            Some("42501") => StatusCode::FORBIDDEN,
            Some("P0001" | "23505" | "23514" | "23502" | "23503") => StatusCode::CONFLICT,
            Some(code) if code.starts_with("22") => StatusCode::BAD_REQUEST,
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
fn bad(status: StatusCode) -> Response {
    status.into_response()
}

// ===================== Partner boundary =====================

/// A verified partner call. `notice` carries deprecation headers to add to the successful response.
struct PartnerCall {
    client_id: String,
    notice: Vec<(HeaderName, HeaderValue)>,
}
impl PartnerCall {
    fn respond(&self, status: StatusCode, body: Value) -> Response {
        let mut response = (status, Json(body)).into_response();
        for (name, value) in &self.notice {
            response.headers_mut().insert(name.clone(), value.clone());
        }
        response
    }
}

/// Token presence, version, token verification, registry and quota checks for every partner call, in that order.
async fn partner(a: &App, h: &HeaderMap) -> Result<PartnerCall, Response> {
    let token = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| bad(StatusCode::UNAUTHORIZED))?;
    let version = h
        .get("Piggybank-Api-Version")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST))?;
    let info = a
        .partners
        .api_version(version)
        .await
        .map_err(|e| bad(error(e)))?
        .ok_or_else(|| bad(StatusCode::BAD_REQUEST))?;
    let sunset = info["sunsetAt"].as_str().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| {
            d.with_timezone(&chrono::Utc)
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string()
        })
    });
    let mut notice = vec![];
    if let Some(date) = &sunset
        && let Ok(value) = HeaderValue::from_str(date)
    {
        notice.push((HeaderName::from_static("sunset"), value));
    }
    if info["state"] == "SUNSET" {
        let mut gone = StatusCode::GONE.into_response();
        for (name, value) in notice {
            gone.headers_mut().insert(name, value);
        }
        return Err(gone);
    }
    if info["state"] == "DEPRECATED" {
        notice.push((
            HeaderName::from_static("deprecation"),
            HeaderValue::from_static("true"),
        ));
    }
    let verified = a
        .identity
        .verifier
        .verify(token, Trust::Partner)
        .await
        .map_err(bad)?;
    let client_id = verified.actor.client_id;
    let client = a
        .partners
        .active_client(&client_id)
        .await
        .map_err(|e| bad(error(e)))?
        .ok_or_else(|| bad(StatusCode::UNAUTHORIZED))?;
    let quota = client["quotaPerMinute"].as_i64().unwrap_or(1);
    if !a
        .partners
        .quota_permits(&client_id, quota)
        .await
        .map_err(|e| bad(error(e)))?
    {
        let mut limited = StatusCode::TOO_MANY_REQUESTS.into_response();
        limited
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        return Err(limited);
    }
    Ok(PartnerCall { client_id, notice })
}
/// Resolves `Consent-Id` to a live binding with the required scope, or 403 without saying why.
async fn consented(
    a: &App,
    h: &HeaderMap,
    call: &PartnerCall,
    scope: &str,
) -> Result<(Binding, ActorContext), Response> {
    let consent = h
        .get("Consent-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| bad(StatusCode::FORBIDDEN))?;
    let binding = a
        .partners
        .binding(&call.client_id, consent)
        .await
        .map_err(|e| bad(error(e)))?
        .filter(|b| b.has_scope(scope))
        .ok_or_else(|| bad(StatusCode::FORBIDDEN))?;
    let actor = binding.actor(&call.client_id);
    Ok((binding, actor))
}
async fn readable(a: &App, actor: &ActorContext, account: Uuid) -> Result<(), Response> {
    let authority = a
        .core
        .authority(AccountId(account), Action::ReadAccount)
        .await
        .map_err(|_| bad(StatusCode::NOT_FOUND))?;
    if a.controls
        .authorize(actor, Action::ReadAccount, Some(&authority), account)
        .await
        .map_err(|e| bad(error(e)))?
    {
        Ok(())
    } else {
        Err(bad(StatusCode::NOT_FOUND))
    }
}
fn customer_web() -> String {
    std::env::var("BANK_CUSTOMER_WEB_ORIGIN")
        .or_else(|_| std::env::var("BANK_PUBLIC_BASE"))
        .unwrap_or_else(|_| "http://127.0.0.1:4180".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConsentRequest {
    scopes: Vec<String>,
    duration_days: i32,
}
async fn partner_request(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<ConsentRequest>,
) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let Ok(request_key) = key(&h) else {
        return bad(StatusCode::BAD_REQUEST);
    };
    if b.scopes.is_empty() || b.scopes.len() > 2 || !(1..=90).contains(&b.duration_days) {
        return bad(StatusCode::BAD_REQUEST);
    }
    let data = json!({"scopes": b.scopes, "durationDays": b.duration_days});
    match a
        .partners
        .request_consent(&call.client_id, request_key, &data)
        .await
    {
        Ok(mut v) => {
            v["authorizationUrl"] = json!(format!(
                "{}/#/connected/{}",
                customer_web(),
                v["consentId"].as_str().unwrap_or_default()
            ));
            call.respond(StatusCode::CREATED, v)
        }
        Err(e) => bad(error(e)),
    }
}
async fn partner_consent(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match a.partners.partner_status(&call.client_id, id).await {
        Ok(Some(v)) => call.respond(StatusCode::OK, v),
        Ok(None) => bad(StatusCode::NOT_FOUND),
        Err(e) => bad(error(e)),
    }
}
async fn partner_withdraw(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match a.partners.partner_withdraw(&call.client_id, id).await {
        Ok(v) => call.respond(StatusCode::OK, v),
        Err(e) => bad(error(e)),
    }
}
async fn partner_accounts(State(a): State<App>, h: HeaderMap) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (binding, actor) = match consented(&a, &h, &call, SCOPE_ACCOUNTS).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let mut accounts = vec![];
    for id in &binding.account_ids {
        // An account closed or restricted since consent drops out rather than failing the whole call.
        if readable(&a, &actor, *id).await.is_ok()
            && let Ok(balance) = a.core.balance(AccountId(*id)).await
        {
            accounts.push(json!({"accountId": id, "balance": balance}));
        }
    }
    call.respond(StatusCode::OK, json!({"accounts": accounts}))
}
async fn partner_balance(State(a): State<App>, h: HeaderMap, Path(id): Path<Uuid>) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (_, actor) = match consented(&a, &h, &call, SCOPE_ACCOUNTS).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Err(r) = readable(&a, &actor, id).await {
        return r;
    }
    match a.core.balance(AccountId(id)).await {
        Ok(b) => call.respond(StatusCode::OK, json!(b)),
        Err(e) => bad(error(e)),
    }
}
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
async fn partner_history(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<HistoryQuery>,
) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (_, actor) = match consented(&a, &h, &call, SCOPE_ACCOUNTS).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if q.snapshot_id.is_some() != q.after.is_some() {
        return bad(StatusCode::BAD_REQUEST);
    }
    if let Err(r) = readable(&a, &actor, id).await {
        return r;
    }
    let cursor = q
        .snapshot_id
        .zip(q.after)
        .map(|(snapshot_id, after)| HistoryCursor { snapshot_id, after });
    match a
        .core
        .history(
            &actor,
            &a.controls,
            AccountId(id),
            (q.from, q.to),
            q.limit,
            cursor,
        )
        .await
    {
        Ok(page) => call.respond(StatusCode::OK, json!(page)),
        Err(e) => bad(error(e)),
    }
}
async fn partner_beneficiaries(State(a): State<App>, h: HeaderMap) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (_, actor) = match consented(&a, &h, &call, SCOPE_PAYMENTS).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match a.payments.beneficiaries(&actor).await {
        // Labels and identifiers only: receiving references stay inside the bank.
        Ok(list) => call.respond(
            StatusCode::OK,
            json!({"beneficiaries": list.iter().filter(|b| b.state == "VERIFIED").map(|b| json!({"id": b.id, "label": b.label})).collect::<Vec<_>>()}),
        ),
        Err(e) => bad(error(e)),
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QuoteInput {
    payer_account_id: AccountId,
    beneficiary_id: Uuid,
    amount: Money,
    requested_date: NaiveDate,
}
async fn partner_quote(State(a): State<App>, h: HeaderMap, Json(b): Json<QuoteInput>) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (_, actor) = match consented(&a, &h, &call, SCOPE_PAYMENTS).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match a
        .payments
        .quote(
            &actor,
            b.payer_account_id,
            b.beneficiary_id,
            b.amount,
            b.requested_date,
        )
        .await
    {
        Ok(q) => call.respond(StatusCode::CREATED, json!(q)),
        Err(e) => bad(error(e)),
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IntentInput {
    quote_id: Uuid,
}
async fn partner_payment(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<IntentInput>,
) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (_, actor) = match consented(&a, &h, &call, SCOPE_PAYMENTS).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Ok(request_key) = key(&h) else {
        return bad(StatusCode::BAD_REQUEST);
    };
    match a
        .payments
        .create_intent(&actor, request_key, b.quote_id)
        .await
    {
        Ok(status) => call.respond(StatusCode::CREATED, partner_payment_view(&status)),
        Err(e) => bad(error(e)),
    }
}
/// What a partner may see of a payment it initiated: no approval links, which are for the customer's own channel.
fn partner_payment_view(p: &bank_payments::PaymentStatus) -> Value {
    json!({"id": p.id, "quoteId": p.quote_id, "state": p.state, "reasonCode": p.reason_code, "total": p.total,
           "payerAccountId": p.details.payer_account_id, "beneficiaryId": p.details.beneficiary_id,
           "customerApprovalRequired": p.state == "DRAFT", "expiresAt": p.expires_at, "updatedAt": p.updated_at})
}
async fn partner_payment_status(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let call = match partner(&a, &h).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (_, actor) = match consented(&a, &h, &call, SCOPE_PAYMENTS).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match a.payments.partner_detail(&actor, id).await {
        Ok(status) => call.respond(StatusCode::OK, partner_payment_view(&status)),
        Err(e) => bad(error(e)),
    }
}

// ===================== Customer consent and delegation =====================

async fn customer(
    a: &App,
    h: &HeaderMap,
    write: bool,
) -> Result<(ActorContext, PartyId), StatusCode> {
    let actor = a
        .identity
        .authenticate(h, Trust::Customer, write)
        .await?
        .actor;
    let person = a.controls.person(&actor).await.map_err(error)?;
    Ok((actor, person))
}
async fn customer_consents(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let (_, person) = customer(&a, &h, false).await?;
    a.partners
        .customer_list(person)
        .await
        .map(Json)
        .map_err(error)
}
async fn customer_consent(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let (_, person) = customer(&a, &h, false).await?;
    a.partners
        .customer_view(person, id)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Authorization {
    approve: bool,
    #[serde(default)]
    account_ids: Vec<Uuid>,
}
async fn customer_authorize(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Authorization>,
) -> Result<Json<Value>, StatusCode> {
    let (actor, person) = customer(&a, &h, true).await?;
    a.partners
        .authorize(&actor, person, id, b.approve, &b.account_ids)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reason {
    reason: String,
}
async fn customer_revoke(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Reason>,
) -> Result<Json<Value>, StatusCode> {
    let (actor, person) = customer(&a, &h, true).await?;
    a.partners
        .revoke(&actor, person, id, &b.reason)
        .await
        .map(Json)
        .map_err(error)
}
async fn delegations(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let (_, person) = customer(&a, &h, false).await?;
    a.core
        .delegations_for(person)
        .await
        .map(Json)
        .map_err(error)
}
async fn delegate_reference(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let (_, person) = customer(&a, &h, true).await?;
    a.core
        .delegate_reference(person)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DelegationInput {
    account_id: Uuid,
    reference: String,
    authority: String,
    payment_limit_minor: Option<i64>,
    duration_days: i32,
}
async fn grant_delegation(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<DelegationInput>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let (_, person) = customer(&a, &h, true).await?;
    let mut data = json!({"accountId": b.account_id, "reference": b.reference, "authority": b.authority, "durationDays": b.duration_days});
    if let Some(limit) = b.payment_limit_minor {
        data["paymentLimitMinor"] = json!(limit);
    }
    a.core
        .grant_delegation(person, key(&h)?, &data)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(error)
}
async fn revoke_delegation(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Reason>,
) -> Result<Json<Value>, StatusCode> {
    let (_, person) = customer(&a, &h, true).await?;
    a.core
        .revoke_delegation(person, id, &b.reason)
        .await
        .map(Json)
        .map_err(error)
}

// ===================== Staff registry =====================

async fn staff(a: &App, h: &HeaderMap, write: bool) -> Result<(ActorContext, PartyId), StatusCode> {
    let actor = a
        .identity
        .authenticate(h, Trust::Workforce, write)
        .await?
        .actor;
    let person = a.controls.person(&actor).await.map_err(error)?;
    Ok((actor, person))
}
async fn staff_partners(State(a): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let (actor, person) = staff(&a, &h, false).await?;
    a.partners
        .staff_view(&actor, person)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Registration {
    client_id: String,
    display_name: String,
    allowed_scopes: Vec<String>,
    quota_per_minute: i32,
    callback_url: Option<String>,
}
async fn staff_propose(
    State(a): State<App>,
    h: HeaderMap,
    Json(b): Json<Registration>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let (actor, person) = staff(&a, &h, true).await?;
    let data = json!({"clientId": b.client_id, "displayName": b.display_name, "allowedScopes": b.allowed_scopes,
                      "quotaPerMinute": b.quota_per_minute, "callbackUrl": b.callback_url.unwrap_or_default()});
    let (client, secret) = a
        .partners
        .propose(&actor, person, key(&h)?, &data)
        .await
        .map_err(error)?;
    // The callback signing secret is shown exactly once, on first creation.
    Ok((
        StatusCode::CREATED,
        Json(json!({"client": client, "callbackSecret": secret})),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    decision: String,
    reason: String,
}
async fn staff_decide(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Decision>,
) -> Result<Json<Value>, StatusCode> {
    let (actor, person) = staff(&a, &h, true).await?;
    a.partners
        .decide(&actor, person, id, &b.decision, &b.reason)
        .await
        .map(Json)
        .map_err(error)
}
async fn staff_revoke(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Reason>,
) -> Result<Json<Value>, StatusCode> {
    let (actor, person) = staff(&a, &h, true).await?;
    a.partners
        .revoke(&actor, person, id, &b.reason)
        .await
        .map(Json)
        .map_err(error)
}
