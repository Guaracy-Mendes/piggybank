use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use bank_controls::checks::{Checks, simulator_key};
use bank_platform::authenticity::SignedDocument;
pub fn router(checks: Checks) -> Router {
    Router::new()
        .route("/api/v1/providers/simulated/controls", post(control_result))
        .with_state(checks)
}
async fn control_result(
    State(checks): State<Checks>,
    Json(packet): Json<SignedDocument>,
) -> Result<StatusCode, StatusCode> {
    packet
        .verify(simulator_key().as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    checks
        .record(&packet)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|error| {
            if error.downcast_ref::<sqlx::Error>().is_some() {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::CONFLICT
            }
        })
}
