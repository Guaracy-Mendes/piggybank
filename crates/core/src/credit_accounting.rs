//! Versioned, read-only native credit receipts for Finance; no allocation capability.
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
pub struct ReturnOriginal {
    pub payment_id: Uuid,
    pub payer_account_id: AccountId,
    pub principal: Money,
    pub batch_id: Uuid,
    pub business_date: NaiveDate,
    pub instruction: Value,
    pub request_digest: String,
    pub finality: Value,
    pub returned_principal_total: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditAccountingFact {
    pub movement_id: Uuid,
    pub evidence_id: Uuid,
    pub kind: String,
    pub original_payment_id: Option<Uuid>,
    pub target_account_id: Option<AccountId>,
    pub receiving_reference: Option<String>,
    pub principal: Money,
    pub business_date: NaiveDate,
    pub request_digest: String,
    pub packet: Value,
    pub suspense_batch_id: Uuid,
    pub suspense_business_date: NaiveDate,
    pub allocation_business_date: Option<NaiveDate>,
    pub allocation_batch_id: Option<Uuid>,
    pub allocation_digest: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub allocated_at: Option<DateTime<Utc>>,
    pub original: Option<ReturnOriginal>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditAccountingSnapshot {
    pub as_of: DateTime<Utc>,
    pub facts: Vec<CreditAccountingFact>,
}
impl Core {
    pub async fn credit_accounting(
        &self,
        range: ReportRange,
        batches: &[Uuid],
        ids: &[Uuid],
    ) -> Result<CreditAccountingSnapshot> {
        range.validate()?;
        ensure!(
            batches.len() <= 1000 && ids.len() <= 3000,
            SnapshotLimitExceeded
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let as_of = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let rows = sqlx::query("SELECT c.*,s.business_date AS suspense_date,aj.business_date AS allocation_date,a.batch_id AS allocation,a.request_digest AS allocation_digest,a.allocated_at,x.payer_account,x.principal AS original_principal,x.batch_id AS original_batch,j.business_date AS original_date,r.instruction,r.request_digest AS original_digest,f.document AS original_finality,(SELECT coalesce(sum(z.principal::numeric),0)::text FROM core.credit_movements z WHERE z.original_payment_id=c.original_payment_id) AS returned FROM core.credit_movements c LEFT JOIN core.credit_allocations a USING(movement_id) JOIN core.journal_batches s ON s.id=c.batch_id LEFT JOIN core.journal_batches aj ON aj.id=a.batch_id LEFT JOIN core.payment_executions x ON x.operation_id=c.original_payment_id LEFT JOIN core.external_reservations r ON r.operation_id=x.operation_id LEFT JOIN core.external_finalities f ON f.operation_id=x.operation_id LEFT JOIN core.journal_batches j ON j.id=x.batch_id WHERE (c.business_date>=$1 AND c.business_date<$2) OR (s.business_date>=$1 AND s.business_date<$2) OR (aj.business_date>=$1 AND aj.business_date<$2) OR c.batch_id=ANY($3) OR a.batch_id=ANY($3) OR c.movement_id=ANY($4) ORDER BY c.movement_id LIMIT 1001")
            .bind(range.from_date).bind(range.to_date).bind(batches).bind(ids).fetch_all(&mut *tx).await?;
        ensure!(rows.len() <= 1000, SnapshotLimitExceeded);
        let mut facts = vec![];
        for r in rows {
            let original =
                r.get::<Option<Uuid>, _>("original_batch")
                    .map(|batch_id| ReturnOriginal {
                        payment_id: r.get("original_payment_id"),
                        payer_account_id: AccountId(r.get("payer_account")),
                        principal: Money::new(r.get("original_principal"), Currency::synthetic()),
                        batch_id,
                        business_date: r.get("original_date"),
                        instruction: r.get("instruction"),
                        request_digest: r.get("original_digest"),
                        finality: r.get("original_finality"),
                        returned_principal_total: r.get("returned"),
                    });
            facts.push(CreditAccountingFact {
                movement_id: r.get("movement_id"),
                evidence_id: r.get("evidence_id"),
                kind: r.get("kind"),
                original_payment_id: r.get("original_payment_id"),
                target_account_id: r.get::<Option<Uuid>, _>("target_account_id").map(AccountId),
                receiving_reference: r.get("receiving_reference"),
                principal: Money::new(r.get("principal"), Currency::synthetic()),
                business_date: r.get("business_date"),
                request_digest: r.get("request_digest"),
                packet: r.get("document"),
                suspense_batch_id: r.get("batch_id"),
                suspense_business_date: r.get("suspense_date"),
                allocation_business_date: r.get("allocation_date"),
                allocation_batch_id: r.get("allocation"),
                allocation_digest: r.get("allocation_digest"),
                recorded_at: r.get("recorded_at"),
                allocated_at: r.get("allocated_at"),
                original,
            });
        }
        tx.commit().await?;
        Ok(CreditAccountingSnapshot { as_of, facts })
    }
}
