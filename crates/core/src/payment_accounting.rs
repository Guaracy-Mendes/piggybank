//! Read-only payment accounting facts; preserved separately from the V1 GL source format.
use crate::{
    Core,
    finance_snapshot::{ReportRange, SnapshotLimitExceeded},
};
use anyhow::{Result, ensure};
use bank_platform::domain::{AccountId, Currency, Money};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentAccountingFact {
    pub payment_id: Uuid,
    pub request_digest: String,
    pub rail: String,
    pub payer_account_id: AccountId,
    pub payee_account_id: Option<AccountId>,
    pub principal: Money,
    pub fee: Money,
    pub product_version_id: Uuid,
    pub requested_date: NaiveDate,
    pub business_date: Option<NaiveDate>,
    pub outcome: String,
    pub batch_id: Option<Uuid>,
    pub evidence_id: Option<Uuid>,
    pub evidence_digest: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub packet: Option<Value>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentAccountingSnapshot {
    pub as_of: DateTime<Utc>,
    pub facts: Vec<PaymentAccountingFact>,
}
impl Core {
    pub async fn payment_accounting(
        &self,
        range: ReportRange,
        batches: &[Uuid],
        operations: &[Uuid],
    ) -> Result<PaymentAccountingSnapshot> {
        range.validate()?;
        ensure!(
            batches.len() <= 1000 && operations.len() <= 3000,
            SnapshotLimitExceeded
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let as_of = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let rows=sqlx::query("SELECT u.operation_id,u.request_digest,u.payer_account,u.principal,u.product_version_id,u.business_date AS requested_date,u.state AS usage_state,x.payee_account,coalesce(x.fee,(r.instruction->'details'->'fee'->>'minorUnits')::bigint) AS fee,x.batch_id,j.business_date,r.operation_id IS NOT NULL AS external,f.outcome,f.evidence_id,f.evidence_digest,f.document,coalesce(f.recorded_at,x.created_at,r.created_at) AS recorded_at FROM core.payment_limit_usage u LEFT JOIN core.payment_executions x USING(operation_id) LEFT JOIN core.external_reservations r USING(operation_id) LEFT JOIN core.external_finalities f USING(operation_id) LEFT JOIN core.journal_batches j ON j.id=x.batch_id WHERE (u.business_date>=$1 AND u.business_date<$2) OR (j.business_date>=$1 AND j.business_date<$2) OR x.batch_id=ANY($3) OR u.operation_id=ANY($4) ORDER BY u.operation_id LIMIT 1001")
            .bind(range.from_date).bind(range.to_date).bind(batches).bind(operations).fetch_all(&mut *tx).await?;
        ensure!(rows.len() <= 1000, SnapshotLimitExceeded);
        let mut facts = vec![];
        for r in rows {
            let external: bool = r.get("external");
            let state: String = r.get("usage_state");
            let outcome = r.get::<Option<String>, _>("outcome").unwrap_or_else(|| {
                match state.as_str() {
                    "POSTED" => "SETTLED",
                    "RELEASED" => "RELEASED",
                    _ => "RESERVED",
                }
                .into()
            });
            facts.push(PaymentAccountingFact {
                payment_id: r.get("operation_id"),
                request_digest: r.get("request_digest"),
                rail: if external { "SIMULATED" } else { "INTERNAL" }.into(),
                payer_account_id: AccountId(r.get("payer_account")),
                payee_account_id: r.get::<Option<Uuid>, _>("payee_account").map(AccountId),
                principal: Money::new(r.get("principal"), Currency::synthetic()),
                fee: Money::new(r.get("fee"), Currency::synthetic()),
                product_version_id: r.get("product_version_id"),
                requested_date: r.get("requested_date"),
                business_date: r.get("business_date"),
                outcome,
                batch_id: r.get("batch_id"),
                evidence_id: r.get("evidence_id"),
                evidence_digest: r.get("evidence_digest"),
                recorded_at: r.get("recorded_at"),
                packet: r.get("document"),
            });
        }
        tx.commit().await?;
        Ok(PaymentAccountingSnapshot { as_of, facts })
    }
}
