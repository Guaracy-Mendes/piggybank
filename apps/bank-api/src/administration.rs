use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::{
    administration::StaffAccess, entitlements::Entitlements, governance::ChangeKind,
};
use bank_core::{Core, products::ProductVersion};
use bank_documents::{Terms, TermsDraft};
use bank_identity::{IdentityService, Trust};
use bank_platform::domain::PartyId;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;
struct Services {
    identity: IdentityService,
    controls: Entitlements,
    core: Core,
    terms: Terms,
}
type App = Arc<Services>;
pub fn router(
    identity: IdentityService,
    controls: Entitlements,
    core: Core,
    terms: Terms,
) -> Router {
    Router::new()
        .route("/api/v1/staff/me", get(access))
        .route("/api/v1/staff/cases/{id}/account", get(support_account))
        .route(
            "/api/v1/staff/configuration",
            get(configuration).post(propose),
        )
        .route("/api/v1/staff/configuration/{id}/approval", post(approve))
        .route(
            "/api/v1/staff/configuration/{id}/publication",
            post(publish),
        )
        .route("/api/v1/staff/roles", get(roles).post(propose_role))
        .route("/api/v1/staff/roles/{id}/approval", post(approve_role))
        .route("/api/v1/staff/roles/{id}/revocation", post(revoke_role))
        .with_state(Arc::new(Services {
            identity,
            controls,
            core,
            terms,
        }))
}
async fn support_account(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, false).await?;
    let case = bank_controls::cases::Cases::new(s.controls.pool.clone())
        .detail(&a.actor, id)
        .await
        .map_err(error)?;
    let person = s.controls.person(&a.actor).await.map_err(error)?;
    if case.kind != "SUPPORT" || case.owner_person != Some(person.0) || case.state == "CLOSED" {
        return Err(StatusCode::FORBIDDEN);
    }
    let balance = s
        .core
        .balance(bank_platform::domain::AccountId(case.resource_id))
        .await
        .map_err(error)?;
    bank_controls::audit::AuditLog::new(s.controls.pool.clone())
        .append(
            Uuid::new_v4(),
            "support",
            &bank_controls::audit::AuditRecord::new(
                &a.actor,
                "support.account-read",
                case.resource_id,
                id,
            ),
        )
        .await
        .map_err(error)?;
    Ok(Json(
        json!({"caseId":id,"accountId":case.resource_id,"balance":balance}),
    ))
}
fn error(e: anyhow::Error) -> StatusCode {
    match e.downcast_ref::<sqlx::Error>() {
        Some(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
        Some(sqlx::Error::Database(db))
            if matches!(db.code().as_deref(), Some("23505" | "23514" | "P0001")) =>
        {
            StatusCode::CONFLICT
        }
        Some(_) => StatusCode::SERVICE_UNAVAILABLE,
        None => StatusCode::FORBIDDEN,
    }
}
async fn access(State(s): State<App>, h: HeaderMap) -> Result<Json<StaffAccess>, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, false).await?;
    s.controls
        .staff_access(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
async fn configuration(State(s): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, false).await?;
    s.controls
        .configurations(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "details",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
enum Draft {
    Product(ProductVersion),
    Terms(TermsDraft),
}
async fn propose(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<Draft>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, true).await?;
    let (kind, id, details) = match body {
        Draft::Product(d) => {
            d.validate().map_err(error)?;
            (
                ChangeKind::Product,
                d.id,
                serde_json::to_value(d).map_err(|_| StatusCode::BAD_REQUEST)?,
            )
        }
        Draft::Terms(d) => {
            d.validate().map_err(error)?;
            (
                ChangeKind::Terms,
                d.id,
                serde_json::to_value(d).map_err(|_| StatusCode::BAD_REQUEST)?,
            )
        }
    };
    s.controls
        .stage_configuration(&a.actor, kind, id, &details)
        .await
        .map(|id| Json(json!({"proposalId":id})))
        .map_err(error)
}
async fn approve(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, true).await?;
    s.controls
        .approve_change(&a.actor, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn publish(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, true).await?;
    let row =
        sqlx::query("SELECT kind,details FROM controls.configuration_drafts WHERE proposal_id=$1")
            .bind(id)
            .fetch_one(&s.controls.pool)
            .await
            .map_err(|e| error(e.into()))?;
    let details: Value = row.get("details");
    match row.get::<String, _>("kind").as_str() {
        "PRODUCT" => {
            let draft: ProductVersion =
                serde_json::from_value(details).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            s.core
                .publish_product(&s.controls, &a.actor, id, &draft)
                .await
                .map_err(error)?;
        }
        "TERMS" => {
            let draft: TermsDraft =
                serde_json::from_value(details).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            s.terms
                .publish(&s.core, &s.controls, &a.actor, id, &draft)
                .await
                .map_err(error)?;
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    }
    // If publication committed but acknowledgement was lost, the same immutable domain key recovers it.
    sqlx::query("UPDATE controls.configuration_drafts SET published_at=now() WHERE proposal_id=$1 AND published_at IS NULL").bind(id).execute(&s.controls.pool).await.map_err(|e|error(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}
async fn roles(State(s): State<App>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, false).await?;
    s.controls
        .role_register(&a.actor)
        .await
        .map(Json)
        .map_err(error)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoleDraft {
    target_person: Uuid,
    role: String,
    scope: Option<Uuid>,
    valid_until: DateTime<Utc>,
}
async fn propose_role(
    State(s): State<App>,
    h: HeaderMap,
    Json(body): Json<RoleDraft>,
) -> Result<Json<Value>, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, true).await?;
    s.controls
        .propose_role(
            &a.actor,
            PartyId(body.target_person),
            &body.role,
            body.scope,
            body.valid_until,
            Uuid::new_v4(),
        )
        .await
        .map(|id| Json(json!({"proposalId":id})))
        .map_err(error)
}
async fn approve_role(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, true).await?;
    s.controls
        .approve_role(&a.actor, id, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
async fn revoke_role(
    State(s): State<App>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let a = s.identity.authenticate(&h, Trust::Workforce, true).await?;
    s.controls
        .revoke_role(&a.actor, id, id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(error)
}
