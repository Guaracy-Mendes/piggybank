use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{Currency, Money, fingerprint},
};
use bank_rails::{
    PROVIDER, RailEvidence, RailInstruction, RailStatus, RecipientEvidence, signing_key,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
#[derive(Clone)]
struct RailApp {
    pool: PgPool,
    key: String,
}
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/rails/recipients/{reference}", get(recipient))
        .route("/rails/instructions", post(submit))
        .route("/rails/inquiry", post(inquiry))
        .route("/rails/cancellation", post(cancel))
        .route("/admin/rail-scenarios", post(scenario))
        .route("/admin/rail-events", post(advance))
        .route("/admin/rail-credits", post(credit))
        .with_state(RailApp {
            pool,
            key: signing_key(),
        })
}
fn authorized(h: &HeaderMap, s: &RailApp) -> Result<(), StatusCode> {
    if h.get("X-Rail-Key").and_then(|h| h.to_str().ok()) == Some(s.key.as_str()) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
fn request(s: &RailApp, p: SignedDocument) -> Result<RailInstruction, StatusCode> {
    p.verify(s.key.as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let r: RailInstruction =
        serde_json::from_value(p.body).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    r.validate().map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    Ok(r)
}
fn signed(s: &RailApp, e: RailEvidence) -> Result<Json<SignedDocument>, StatusCode> {
    SignedDocument::sign(s.key.as_bytes(), &e)
        .map(Json)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
async fn recipient(
    State(s): State<RailApp>,
    h: HeaderMap,
    Path(reference): Path<String>,
) -> Result<Json<SignedDocument>, StatusCode> {
    authorized(&h, &s)?;
    let enabled: bool =
        sqlx::query_scalar("SELECT enabled FROM simulator.rail_recipients WHERE reference=$1")
            .bind(&reference)
            .fetch_optional(&s.pool)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::NOT_FOUND)?;
    SignedDocument::sign(
        s.key.as_bytes(),
        &RecipientEvidence {
            provider: PROVIDER.into(),
            reference,
            enabled,
            currency: Currency::synthetic(),
            verified_at: Utc::now(),
        },
    )
    .map(Json)
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
async fn submit(
    State(s): State<RailApp>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    let r = request(&s, packet)?;
    let mode: String =
        sqlx::query_scalar("SELECT mode FROM simulator.rail_scenarios WHERE instruction_id=$1")
            .bind(r.instruction_id)
            .fetch_optional(&s.pool)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .unwrap_or_else(|| "ACCEPT".into());
    if mode == "BEFORE_ACCEPT_TIMEOUT" {
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let result = persist(&s, &r, None, mode == "REJECT")
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    if mode == "LOST_RESPONSE" {
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    }
    signed(
        &s,
        if mode == "CONFLICT" {
            conflict(result)
        } else {
            result
        },
    )
}
async fn inquiry(
    State(s): State<RailApp>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    let r = request(&s, packet)?;
    let mut tx = s
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let found = existing(&mut tx, &r)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    tx.commit()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let result = found.unwrap_or_else(|| evidence(&r, RailStatus::NotFound, 1));
    signed(&s, result)
}
async fn cancel(
    State(s): State<RailApp>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    let r = request(&s, packet)?;
    let e = persist(&s, &r, Some(RailStatus::Cancelled), false)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    signed(&s, e)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    instruction_id: Uuid,
    mode: String,
}
async fn scenario(
    State(s): State<RailApp>,
    h: HeaderMap,
    Json(b): Json<Scenario>,
) -> Result<StatusCode, StatusCode> {
    authorized(&h, &s)?;
    if ![
        "ACCEPT",
        "REJECT",
        "LOST_RESPONSE",
        "BEFORE_ACCEPT_TIMEOUT",
        "CONFLICT",
    ]
    .contains(&b.mode.as_str())
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    sqlx::query("INSERT INTO simulator.rail_scenarios VALUES($1,$2) ON CONFLICT(instruction_id) DO UPDATE SET mode=EXCLUDED.mode").bind(b.instruction_id).bind(b.mode).execute(&s.pool).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Advance {
    instruction_id: Uuid,
    status: RailStatus,
}
async fn advance(
    State(s): State<RailApp>,
    h: HeaderMap,
    Json(b): Json<Advance>,
) -> Result<Json<SignedDocument>, StatusCode> {
    authorized(&h, &s)?;
    if matches!(b.status, RailStatus::NotFound | RailStatus::Accepted) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT instruction FROM simulator.rail_instructions WHERE id=$1")
            .bind(b.instruction_id)
            .fetch_one(&s.pool)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
    let r: RailInstruction =
        serde_json::from_value(value).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let e = persist(&s, &r, Some(b.status), false)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    signed(&s, e)
}
async fn existing(
    tx: &mut Transaction<'_, Postgres>,
    r: &RailInstruction,
) -> anyhow::Result<Option<RailEvidence>> {
    let row=sqlx::query("SELECT i.fingerprint,e.document FROM simulator.rail_instructions i JOIN simulator.rail_events e ON e.id=i.evidence_id WHERE i.id=$1").bind(r.instruction_id).fetch_optional(&mut **tx).await?;
    if let Some(row) = row {
        anyhow::ensure!(
            row.get::<String, _>("fingerprint") == fingerprint(r)?,
            "Provider instruction identity conflict"
        );
        Ok(Some(serde_json::from_value(row.get("document"))?))
    } else {
        Ok(None)
    }
}
async fn persist(
    s: &RailApp,
    r: &RailInstruction,
    desired: Option<RailStatus>,
    reject: bool,
) -> anyhow::Result<RailEvidence> {
    let mut tx = s.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,140))")
        .bind(r.instruction_id.to_string())
        .execute(&mut *tx)
        .await?;
    if let Some(old) = existing(&mut tx, r).await? {
        if let Some(status) = desired {
            if old.status == status
                || matches!(
                    old.status,
                    RailStatus::Settled | RailStatus::Rejected | RailStatus::Cancelled
                )
            {
                tx.commit().await?;
                return Ok(old);
            }
            let e = evidence(r, status, old.version + 1);
            sqlx::query("UPDATE simulator.rail_instructions SET state=$2,version=$3,evidence_id=$4 WHERE id=$1").bind(r.instruction_id).bind(code(&e.status)).bind(e.version).bind(e.evidence_id).execute(&mut *tx).await?;
            store(&mut tx, &e).await?;
            tx.commit().await?;
            return Ok(e);
        }
        tx.commit().await?;
        return Ok(old);
    }
    let enabled: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM simulator.rail_recipients WHERE reference=$1 AND enabled)",
    )
    .bind(&r.beneficiary_reference)
    .fetch_one(&mut *tx)
    .await?;
    let status = desired.unwrap_or(
        if reject
            || !enabled
            || r.authority_until <= Utc::now()
            || r.requested_date != Utc::now().date_naive()
        {
            RailStatus::Rejected
        } else {
            RailStatus::Accepted
        },
    );
    let e = evidence(r, status, 1);
    sqlx::query("INSERT INTO simulator.rail_instructions(id,fingerprint,instruction,state,version,evidence_id) VALUES($1,$2,$3,$4,$5,$6)").bind(r.instruction_id).bind(fingerprint(r)?).bind(serde_json::to_value(r)?).bind(code(&e.status)).bind(e.version).bind(e.evidence_id).execute(&mut *tx).await?;
    store(&mut tx, &e).await?;
    tx.commit().await?;
    Ok(e)
}
async fn store(tx: &mut Transaction<'_, Postgres>, e: &RailEvidence) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO simulator.rail_events VALUES($1,$2,$3,$4)")
        .bind(e.evidence_id)
        .bind(e.instruction_id)
        .bind(e.version)
        .bind(serde_json::to_value(e)?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
fn code(s: &RailStatus) -> &'static str {
    match s {
        RailStatus::Accepted => "ACCEPTED",
        RailStatus::Pending => "PENDING",
        RailStatus::Settled => "SETTLED",
        RailStatus::Rejected => "REJECTED",
        RailStatus::Cancelled => "CANCELLED",
        RailStatus::NotFound => "NOT_FOUND",
    }
}
fn evidence(r: &RailInstruction, status: RailStatus, version: i64) -> RailEvidence {
    RailEvidence {
        evidence_id: Uuid::new_v4(),
        provider: PROVIDER.into(),
        instruction_id: r.instruction_id,
        payment_id: r.payment_id,
        input_digest: r.input_digest.clone(),
        amount: r.amount,
        status: status.clone(),
        version,
        occurred_at: Utc::now(),
        settlement_date: if status == RailStatus::Settled {
            Some(Utc::now().date_naive())
        } else {
            None
        },
        reason_code: "SIMULATED_RAIL_RESULT".into(),
    }
}
fn conflict(mut e: RailEvidence) -> RailEvidence {
    e.amount = Money::new(
        e.amount.minor_units().saturating_add(1),
        e.amount.currency(),
    );
    e
}

async fn credit(
    State(s): State<RailApp>,
    h: HeaderMap,
    Json(e): Json<bank_rails::CreditEvidence>,
) -> Result<Json<SignedDocument>, StatusCode> {
    authorized(&h, &s)?;
    e.validate().map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let digest = fingerprint(&e).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let value = serde_json::to_value(&e).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    sqlx::query("INSERT INTO simulator.rail_credits VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
        .bind(e.movement_id)
        .bind(e.evidence_id)
        .bind(&digest)
        .bind(&value)
        .execute(&s.pool)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let rows =
        sqlx::query("SELECT * FROM simulator.rail_credits WHERE movement_id=$1 OR evidence_id=$2")
            .bind(e.movement_id)
            .bind(e.evidence_id)
            .fetch_all(&s.pool)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if rows.len() != 1 || rows[0].get::<String, _>("fingerprint") != digest {
        return Err(StatusCode::CONFLICT);
    }
    SignedDocument::sign(s.key.as_bytes(), &e)
        .map(Json)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
