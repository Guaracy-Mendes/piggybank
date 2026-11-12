use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use bank_identity::{IdentityService, Trust};
use bank_platform::operations::{OperationStatus, Operations};
use std::sync::Arc;
use uuid::Uuid;

pub fn router(identity: IdentityService, operations: Operations) -> Router {
    Router::new()
        .route("/api/v1/operations/{id}", get(customer_status))
        .route("/api/v1/staff/operations/{id}", get(staff_status))
        .with_state(Arc::new((identity, operations)))
}
async fn status(
    state: Arc<(IdentityService, Operations)>,
    id: Uuid,
    headers: HeaderMap,
    trust: Trust,
) -> Result<Json<OperationStatus>, StatusCode> {
    let identity = state.0.authenticate(&headers, trust, false).await?;
    state
        .1
        .status(&identity.actor, id)
        .await
        .map(Json)
        .map_err(|error| {
            if error.downcast_ref::<sqlx::Error>().is_some() {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::NOT_FOUND
            }
        })
}
async fn customer_status(
    State(state): State<Arc<(IdentityService, Operations)>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<OperationStatus>, StatusCode> {
    status(state, id, headers, Trust::Customer).await
}
async fn staff_status(
    State(state): State<Arc<(IdentityService, Operations)>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<OperationStatus>, StatusCode> {
    status(state, id, headers, Trust::Workforce).await
}
