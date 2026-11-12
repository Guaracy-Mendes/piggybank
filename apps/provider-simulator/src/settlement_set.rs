//! Provider-issued closed membership; bank databases never supply clearing rows or totals.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use bank_finance::{
    credit_settlement::CreditSettlementFile,
    protocol::signing_key,
    settlement::{FEE_POLICY, PROVIDER, SettlementFile},
    settlement_set::{PublishSet, SetKind, SetMember, SetQuery, SetReply, SettlementSet},
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
        .route("/admin/settlement-sets", post(publish))
        .route("/settlement-sets/inquiry", post(inquiry))
        .with_state(App {
            pool,
            key: signing_key(),
        })
}
async fn publish(
    State(app): State<App>,
    headers: HeaderMap,
    Json(mut input): Json<PublishSet>,
) -> Result<(StatusCode, Json<SettlementSet>), StatusCode> {
    if headers.get("X-Finance-Key").and_then(|v| v.to_str().ok()) != Some(app.key.as_str()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    input
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    input.file_ids.sort();
    let mut tx = app
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut members = vec![];
    for id in &input.file_ids {
        let query = match input.kind {
            SetKind::Outgoing => {
                "SELECT document,digest FROM simulator.settlement_files WHERE id=$1"
            }
            SetKind::Credits => {
                "SELECT document,digest FROM simulator.credit_settlement_files WHERE id=$1"
            }
        };
        let row = sqlx::query(query)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
        let (range, cutoff, count, digest) = match input.kind {
            SetKind::Outgoing => {
                let f: SettlementFile = serde_json::from_value(row.get("document"))
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                f.validate().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                (
                    f.range,
                    f.generated_at,
                    f.rows.len(),
                    fingerprint(&f).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
                )
            }
            SetKind::Credits => {
                let f: CreditSettlementFile = serde_json::from_value(row.get("document"))
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                f.validate().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
                (
                    f.range,
                    f.generated_at,
                    f.rows.len(),
                    fingerprint(&f).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
                )
            }
        };
        if range != input.range || cutoff != input.generated_at {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        if digest != row.get::<String, _>("digest") {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        members.push(SetMember {
            file_id: *id,
            digest,
            row_count: count as i32,
        });
    }
    let manifest = SettlementSet {
        set_id: input.set_id,
        format_version: 1,
        kind: input.kind,
        provider: PROVIDER.into(),
        fee_policy: FEE_POLICY.into(),
        range: input.range,
        generated_at: input.generated_at,
        row_count: members.iter().map(|m| m.row_count).sum(),
        members,
    };
    manifest
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let digest = fingerprint(&manifest).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let changed=sqlx::query("INSERT INTO simulator.settlement_sets(id,document,digest) VALUES($1,$2,$3) ON CONFLICT(id) DO NOTHING").bind(input.set_id).bind(serde_json::to_value(&manifest).map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?).bind(&digest).execute(&mut *tx).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.rows_affected()==1;
    let prior: String =
        sqlx::query_scalar("SELECT digest FROM simulator.settlement_sets WHERE id=$1")
            .bind(input.set_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if prior != digest {
        return Err(StatusCode::CONFLICT);
    }
    tx.commit()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((
        if changed {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(manifest),
    ))
}
async fn inquiry(
    State(app): State<App>,
    Json(packet): Json<SignedDocument>,
) -> Result<Json<SignedDocument>, StatusCode> {
    packet
        .verify(app.key.as_bytes())
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let request: SetQuery =
        serde_json::from_value(packet.body).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    request
        .validate()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let row = sqlx::query("SELECT document,digest FROM simulator.settlement_sets WHERE id=$1")
        .bind(request.set_id)
        .fetch_optional(&app.pool)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let manifest: SettlementSet =
        serde_json::from_value(row.get("document")).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    manifest
        .validate()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if fingerprint(&manifest).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        != row.get::<String, _>("digest")
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    SignedDocument::sign(app.key.as_bytes(), &SetReply { request, manifest })
        .map(Json)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
