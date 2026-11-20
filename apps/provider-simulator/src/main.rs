//! Persistent synthetic provider. No real identity or network integration is enabled here.
mod credit_settlement;
mod gl;
mod liquidity;
mod rail;
mod settlement;
mod settlement_set;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_controls::checks::{CheckRequest, CheckResult, Outcome, simulator_key};
use bank_platform::{
    authenticity::SignedDocument, config::Config, database::Module, domain::fingerprint,
    init_tracing, shutdown_signal,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone)]
struct App {
    pool: PgPool,
    key: String,
}
type Error = StatusCode;
fn authorized(headers: &HeaderMap, app: &App) -> Result<(), Error> {
    if headers.get("X-Simulator-Key").and_then(|h| h.to_str().ok()) == Some(app.key.as_str()) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    request_id: Uuid,
    mode: String,
}
async fn scenario(
    State(app): State<App>,
    headers: HeaderMap,
    Json(body): Json<Scenario>,
) -> Result<StatusCode, Error> {
    authorized(&headers, &app)?;
    if ![
        "ALLOW",
        "DENY",
        "REVIEW",
        "PENDING",
        "TIMEOUT",
        "LOST_RESPONSE",
        "LATE",
        "CONFLICT",
    ]
    .contains(&body.mode.as_str())
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    sqlx::query("INSERT INTO simulator.scenarios VALUES($1,$2) ON CONFLICT(request_id) DO UPDATE SET mode=EXCLUDED.mode").bind(body.request_id).bind(body.mode).execute(&app.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn check(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<CheckRequest>,
) -> Result<Json<SignedDocument>, Error> {
    authorized(&headers, &app)?;
    if request.input_digest.len() != 64 || request.policy_version != "synthetic-controls-v1" {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let result = run(&app, &request)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(Json(result))
}
async fn inquiry(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<SignedDocument>, Error> {
    authorized(&headers, &app)?;
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT document FROM simulator.checks WHERE id=$1")
            .bind(id)
            .fetch_optional(&app.pool)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(
        SignedDocument::sign(app.key.as_bytes(), &value)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    ))
}
async fn run(app: &App, request: &CheckRequest) -> anyhow::Result<SignedDocument> {
    let mut tx = app.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,42))")
        .bind(request.id.to_string())
        .execute(&mut *tx)
        .await?;
    let mode: String =
        sqlx::query_scalar("SELECT mode FROM simulator.scenarios WHERE request_id=$1")
            .bind(request.id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_else(|| "ALLOW".into());
    let prior =
        sqlx::query("SELECT request_fingerprint,document FROM simulator.checks WHERE id=$1")
            .bind(request.id)
            .fetch_optional(&mut *tx)
            .await?;
    let desired = match mode.as_str() {
        "DENY" => Outcome::Deny,
        "REVIEW" => Outcome::Review,
        "PENDING" | "TIMEOUT" => Outcome::Pending,
        _ => Outcome::Allow,
    };
    let digest = fingerprint(request)?;
    let mut result = if let Some(row) = prior {
        anyhow::ensure!(
            row.get::<String, _>("request_fingerprint") == digest,
            "Provider identity conflict"
        );
        serde_json::from_value::<CheckResult>(row.get("document"))?
    } else {
        CheckResult {
            request: request.clone(),
            outcome: desired,
            evidence_id: Uuid::new_v4(),
            expires_at: Utc::now() + Duration::minutes(5),
            reason_code: "SIMULATED_DECISION".into(),
        }
    };
    if result.outcome == Outcome::Pending && desired != Outcome::Pending {
        result.outcome = desired;
        result.expires_at = Utc::now() + Duration::minutes(5);
    }
    if mode == "LATE" {
        result.expires_at = Utc::now() - Duration::seconds(1);
    }
    sqlx::query("INSERT INTO simulator.checks(id,request_fingerprint,request,document) VALUES($1,$2,$3,$4) ON CONFLICT(id) DO UPDATE SET document=EXCLUDED.document")
        .bind(request.id).bind(digest).bind(serde_json::to_value(request)?).bind(serde_json::to_value(&result)?).execute(&mut *tx).await?;
    tx.commit().await?;
    if mode == "CONFLICT" {
        result.outcome = if result.outcome == Outcome::Deny {
            Outcome::Allow
        } else {
            Outcome::Deny
        };
    }
    // Acceptance was persisted before injecting the lost/late response.
    if mode == "TIMEOUT" || mode == "LOST_RESPONSE" {
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    }
    SignedDocument::sign(app.key.as_bytes(), &result)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let config = Config::load(4182)?;
    anyhow::ensure!(
        config.bind.ip().is_loopback(),
        "Synthetic simulator must bind loopback"
    );
    let app = App {
        pool: Module::Simulator.connect().await?,
        key: simulator_key(),
    };
    let rails = rail::router(app.pool.clone()).merge(rail::router_b(app.pool.clone()));
    let gl = gl::router(app.pool.clone());
    let liquidity = liquidity::router(app.pool.clone());
    let settlement = settlement::router(app.pool.clone());
    let settlement_set = settlement_set::router(app.pool.clone());
    let credit_settlement = credit_settlement::router(app.pool.clone());
    let router = Router::new()
        .route(
            "/health/live",
            get(|| async { Json(serde_json::json!({"status":"alive","profile":"SYNTHETIC"})) }),
        )
        .route("/checks", post(check))
        .route("/checks/{id}", get(inquiry))
        .route("/admin/scenarios", post(scenario))
        .with_state(app)
        .merge(rails)
        .merge(gl)
        .merge(liquidity)
        .merge(settlement)
        .merge(settlement_set)
        .merge(credit_settlement)
        .layer(axum::extract::DefaultBodyLimit::max(65536));
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address=%config.bind,"starting synthetic provider simulator");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}
