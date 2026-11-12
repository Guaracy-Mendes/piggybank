//! Bounded Finance inquiry port. This reader has no payment command capability.
use anyhow::{Result, ensure};
use bank_core::finance_snapshot::{ReportRange, SnapshotLimitExceeded};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone)]
pub struct FinanceReader {
    pool: PgPool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentLineage {
    pub payment_id: Uuid,
    pub quote_id: Uuid,
    pub beneficiary_id: Uuid,
    pub receiving_reference: String,
    pub quote: Value,
    pub quote_digest: String,
    pub state: String,
    pub batch_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub transport: Option<Value>,
    pub transport_digest: Option<String>,
    pub receipts: Vec<Value>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentLineageSnapshot {
    pub as_of: DateTime<Utc>,
    pub payments: Vec<PaymentLineage>,
}
impl FinanceReader {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn snapshot(
        &self,
        range: ReportRange,
        ids: &[Uuid],
    ) -> Result<PaymentLineageSnapshot> {
        range.validate()?;
        ensure!(ids.len() <= 3000, SnapshotLimitExceeded);
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let as_of = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let rows=sqlx::query("SELECT i.id,i.quote_id,i.state,i.batch_id,i.created_at,i.updated_at,q.instruction,q.details_digest,q.beneficiary_id,b.receiving_reference,s.document,s.fingerprint FROM payments.intents i JOIN payments.quotes q ON q.id=i.quote_id JOIN payments.beneficiaries b ON b.id=q.beneficiary_id LEFT JOIN payments.rail_submissions s ON s.payment_id=i.id WHERE (i.confirmed_at IS NOT NULL AND (q.instruction->'details'->>'requestedDate')::date>=$1 AND (q.instruction->'details'->>'requestedDate')::date<$2) OR i.id=ANY($3) ORDER BY i.id LIMIT 1001").bind(range.from_date).bind(range.to_date).bind(ids).fetch_all(&mut *tx).await?;
        ensure!(rows.len() <= 1000, SnapshotLimitExceeded);
        let found = rows
            .iter()
            .map(|r| r.get::<Uuid, _>("id"))
            .collect::<Vec<_>>();
        // At most one immutable final receipt per payment version is stored by Payments.
        let receipts=sqlx::query("SELECT payment_id,document FROM payments.rail_receipts WHERE payment_id=ANY($1) AND document->'body'->>'status' IN ('SETTLED','REJECTED','CANCELLED') ORDER BY payment_id,provider_version,evidence_id LIMIT 2001").bind(&found).fetch_all(&mut *tx).await?;
        ensure!(receipts.len() <= 2000, SnapshotLimitExceeded);
        let mut grouped = std::collections::BTreeMap::<Uuid, Vec<Value>>::new();
        for row in receipts {
            grouped
                .entry(row.get("payment_id"))
                .or_default()
                .push(row.get("document"));
        }
        let payments = rows
            .into_iter()
            .map(|r| PaymentLineage {
                payment_id: r.get("id"),
                quote_id: r.get("quote_id"),
                beneficiary_id: r.get("beneficiary_id"),
                receiving_reference: r.get("receiving_reference"),
                quote: r.get("instruction"),
                quote_digest: r.get("details_digest"),
                state: r.get("state"),
                batch_id: r.get("batch_id"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
                transport: r.get("document"),
                transport_digest: r.get("fingerprint"),
                receipts: grouped.remove(&r.get("id")).unwrap_or_default(),
            })
            .collect();
        tx.commit().await?;
        Ok(PaymentLineageSnapshot { as_of, payments })
    }
}

/// Separate V1 archive: adding credit lineage never changes saved outgoing snapshot bytes.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditLineage {
    pub movement_id: Uuid,
    pub evidence_id: Uuid,
    pub packet: Value,
    pub request_digest: String,
    pub target_account_id: Option<bank_platform::domain::AccountId>,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditLineageSnapshot {
    pub as_of: DateTime<Utc>,
    pub credits: Vec<CreditLineage>,
}
impl FinanceReader {
    pub async fn credit_snapshot(
        &self,
        range: ReportRange,
        ids: &[Uuid],
    ) -> Result<CreditLineageSnapshot> {
        range.validate()?;
        ensure!(ids.len() <= 3000, SnapshotLimitExceeded);
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let as_of = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let rows=sqlx::query("SELECT * FROM payments.credit_intents WHERE ((document->'body'->>'businessDate')::date>=$1 AND (document->'body'->>'businessDate')::date<$2) OR movement_id=ANY($3) ORDER BY movement_id LIMIT 1001").bind(range.from_date).bind(range.to_date).bind(ids).fetch_all(&mut *tx).await?;
        ensure!(rows.len() <= 1000, SnapshotLimitExceeded);
        let credits = rows
            .into_iter()
            .map(|r| CreditLineage {
                movement_id: r.get("movement_id"),
                evidence_id: r.get("evidence_id"),
                packet: r.get("document"),
                request_digest: r.get("fingerprint"),
                target_account_id: r
                    .get::<Option<Uuid>, _>("target_account_id")
                    .map(bank_platform::domain::AccountId),
                state: r.get("state"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            })
            .collect();
        tx.commit().await?;
        Ok(CreditLineageSnapshot { as_of, credits })
    }
}
