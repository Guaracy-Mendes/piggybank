use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use bank_finance::liquidity::{LiquidityEvidence, LiquidityReply, LiquidityRequest, liquidity_key};
use bank_platform::{authenticity::SignedDocument, domain::fingerprint};
use sqlx::{PgPool, Row};
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/liquidity/inquiry", post(inquire))
        .route("/admin/liquidity-positions", post(publish))
        .with_state(pool)
}
async fn inquire(
    State(pool): State<PgPool>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    packet
        .verify(liquidity_key().as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let request: LiquidityRequest =
        serde_json::from_value(packet.body).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    request
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let doc: serde_json::Value = sqlx::query_scalar(
        "SELECT document FROM simulator.liquidity_positions ORDER BY version DESC LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .ok_or(StatusCode::NOT_FOUND)?;
    let position: LiquidityEvidence =
        serde_json::from_value(doc).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    SignedDocument::sign(
        liquidity_key().as_bytes(),
        &LiquidityReply { request, position },
    )
    .map(Json)
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
async fn publish(
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(position): Json<LiquidityEvidence>,
) -> Result<StatusCode, StatusCode> {
    if headers.get("X-Liquidity-Key").and_then(|h| h.to_str().ok())
        != Some(liquidity_key().as_str())
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    position
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    sqlx::query("SELECT pg_advisory_xact_lock(168,1)")
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    sqlx::query("INSERT INTO simulator.liquidity_positions(evidence_id,version,document,digest) VALUES($1,$2,$3,$4) ON CONFLICT(evidence_id) DO NOTHING").bind(position.evidence_id).bind(position.version).bind(serde_json::to_value(&position).map_err(|_|StatusCode::UNPROCESSABLE_ENTITY)?).bind(fingerprint(&position).map_err(|_|StatusCode::UNPROCESSABLE_ENTITY)?).execute(&mut *tx).await.map_err(|_|StatusCode::CONFLICT)?;
    let row = sqlx::query("SELECT digest FROM simulator.liquidity_positions WHERE evidence_id=$1")
        .bind(position.evidence_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if row.get::<String, _>("digest")
        != fingerprint(&position).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?
    {
        return Err(StatusCode::CONFLICT);
    }
    tx.commit()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(StatusCode::CREATED)
}
