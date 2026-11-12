//! Explicit immutable settlement-file fixtures, independent of the bank's ledger/projections.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use bank_finance::{
    protocol::signing_key,
    settlement::{SettlementFile, SettlementQuery, SettlementReply},
};
use bank_platform::{authenticity::SignedDocument, domain::fingerprint};
use sqlx::{PgPool, Row};
#[derive(Clone)]
struct App {
    pool: PgPool,
    key: String,
}
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/admin/settlement-files", post(publish))
        .route("/settlement-files/inquiry", post(inquiry))
        .with_state(App {
            pool,
            key: signing_key(),
        })
}
async fn publish(
    State(app): State<App>,
    headers: HeaderMap,
    Json(file): Json<SettlementFile>,
) -> Result<StatusCode, StatusCode> {
    if headers.get("X-Finance-Key").and_then(|v| v.to_str().ok()) != Some(app.key.as_str()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    file.validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let digest = fingerprint(&file).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let mut tx = app
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let changed=sqlx::query("INSERT INTO simulator.settlement_files(id,document,digest) VALUES($1,$2,$3) ON CONFLICT(id) DO NOTHING").bind(file.file_id).bind(serde_json::to_value(&file).map_err(|_|StatusCode::UNPROCESSABLE_ENTITY)?).bind(&digest).execute(&mut *tx).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.rows_affected()==1;
    let stored: String =
        sqlx::query_scalar("SELECT digest FROM simulator.settlement_files WHERE id=$1")
            .bind(file.file_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if stored != digest {
        return Err(StatusCode::CONFLICT);
    }
    tx.commit()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(if changed {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    })
}
async fn inquiry(
    State(app): State<App>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    packet
        .verify(app.key.as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let request: SettlementQuery =
        serde_json::from_value(packet.body).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    request
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let row = sqlx::query("SELECT document,digest FROM simulator.settlement_files WHERE id=$1")
        .bind(request.file_id)
        .fetch_optional(&app.pool)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let file: SettlementFile =
        serde_json::from_value(row.get("document")).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    file.validate()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if fingerprint(&file).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        != row.get::<String, _>("digest")
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    SignedDocument::sign(app.key.as_bytes(), &SettlementReply { request, file })
        .map(Json)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
