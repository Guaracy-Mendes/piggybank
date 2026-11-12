//! Synthetic GL accepts the exact original Core batch at most once, across process restarts.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use bank_finance::protocol::{GlBatch, GlEvidence, GlStatus, signing_key};
use bank_platform::{authenticity::SignedDocument, domain::fingerprint};
use chrono::Utc;
use serde::Deserialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone)]
struct App {
    pool: PgPool,
    key: String,
}
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/gl/batches", post(book))
        .route("/gl/inquiry", post(inquire))
        .route("/admin/gl-scenarios", post(scenario))
        .with_state(App {
            pool,
            key: signing_key(),
        })
}
fn request(app: &App, packet: SignedDocument) -> Result<GlBatch, StatusCode> {
    packet
        .verify(app.key.as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let batch: GlBatch =
        serde_json::from_value(packet.body).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    batch
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    Ok(batch)
}
fn evidence(batch: &GlBatch, status: GlStatus) -> anyhow::Result<GlEvidence> {
    let reference = if status == GlStatus::Booked {
        Some(format!("SIMGL-{}", batch.source_batch_id.simple()))
    } else {
        None
    };
    Ok(GlEvidence {
        evidence_id: Uuid::new_v4(),
        provider: "SYNTHETIC_GL".into(),
        export_id: batch.export_id,
        source_batch_id: batch.source_batch_id,
        input_digest: fingerprint(batch)?,
        status,
        provider_journal_reference: reference,
        business_date: batch.business_date,
        debit_total: batch.debit_total.clone(),
        credit_total: batch.credit_total.clone(),
        occurred_at: Utc::now(),
        version: 1,
    })
}
fn signed(
    app: &App,
    mut e: GlEvidence,
    conflict: bool,
) -> Result<Json<SignedDocument>, StatusCode> {
    if conflict {
        e.input_digest = "0".repeat(64);
    }
    SignedDocument::sign(app.key.as_bytes(), &e)
        .map(Json)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
async fn mode(app: &App, id: Uuid) -> Result<String, StatusCode> {
    Ok(
        sqlx::query_scalar("SELECT mode FROM simulator.gl_scenarios WHERE source_batch_id=$1")
            .bind(id)
            .fetch_optional(&app.pool)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .unwrap_or_else(|| "ACCEPT".into()),
    )
}
async fn book(
    State(app): State<App>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    let batch = request(&app, packet)?;
    let mode = mode(&app, batch.source_batch_id).await?;
    if mode == "BEFORE_ACCEPT_TIMEOUT" {
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let e = persist(&app, &batch)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    if mode == "LOST_RESPONSE" {
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    }
    signed(&app, e, mode == "CONFLICT")
}
async fn persist(app: &App, batch: &GlBatch) -> anyhow::Result<GlEvidence> {
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,166))")
        .bind(batch.source_batch_id.to_string())
        .execute(&mut *tx)
        .await?;
    let prior = sqlx::query(
        "SELECT fingerprint,receipt FROM simulator.gl_batches WHERE source_batch_id=$1",
    )
    .bind(batch.source_batch_id)
    .fetch_optional(&mut *tx)
    .await?;
    let e = if let Some(row) = prior {
        anyhow::ensure!(
            row.get::<String, _>("fingerprint") == fingerprint(batch)?,
            "Original GL source scope conflict"
        );
        serde_json::from_value(row.get("receipt"))?
    } else {
        let e = evidence(batch, GlStatus::Booked)?;
        sqlx::query("INSERT INTO simulator.gl_batches(source_batch_id,export_id,fingerprint,document,receipt) VALUES($1,$2,$3,$4,$5)").bind(batch.source_batch_id).bind(batch.export_id).bind(fingerprint(batch)?).bind(serde_json::to_value(batch)?).bind(serde_json::to_value(&e)?).execute(&mut *tx).await?;
        e
    };
    tx.commit().await?;
    Ok(e)
}
async fn inquire(
    State(app): State<App>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    let batch = request(&app, packet)?;
    let row = sqlx::query(
        "SELECT fingerprint,receipt FROM simulator.gl_batches WHERE source_batch_id=$1",
    )
    .bind(batch.source_batch_id)
    .fetch_optional(&app.pool)
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let e = if let Some(row) = row {
        if row.get::<String, _>("fingerprint")
            != fingerprint(&batch).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?
        {
            return Err(StatusCode::CONFLICT);
        }
        serde_json::from_value(row.get("receipt")).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    } else {
        evidence(&batch, GlStatus::NotFound).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    };
    signed(
        &app,
        e,
        mode(&app, batch.source_batch_id).await? == "CONFLICT",
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    source_batch_id: Uuid,
    mode: String,
}
async fn scenario(
    State(app): State<App>,
    headers: HeaderMap,
    Json(r): Json<Scenario>,
) -> Result<StatusCode, StatusCode> {
    if headers.get("X-Finance-Key").and_then(|h| h.to_str().ok()) != Some(&app.key) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if ![
        "ACCEPT",
        "LOST_RESPONSE",
        "BEFORE_ACCEPT_TIMEOUT",
        "CONFLICT",
    ]
    .contains(&r.mode.as_str())
        || r.source_batch_id.is_nil()
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    sqlx::query("INSERT INTO simulator.gl_scenarios VALUES($1,$2) ON CONFLICT(source_batch_id) DO UPDATE SET mode=EXCLUDED.mode").bind(r.source_batch_id).bind(r.mode).execute(&app.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(StatusCode::NO_CONTENT)
}
