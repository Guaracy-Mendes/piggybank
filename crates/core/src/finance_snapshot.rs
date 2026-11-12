//! Bounded, consistent Finance reads. No financial command capability is exposed.
use crate::{
    Core,
    finance::FinancialBatch,
    inquiry::{JournalBatch, JournalEntry},
};
use anyhow::{Result, ensure};
use bank_platform::domain::{AccountId, Currency, Money};
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;
#[derive(Debug)]
pub struct SnapshotLimitExceeded;
impl std::fmt::Display for SnapshotLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Financial snapshot source limit exceeded; narrow the reporting period")
    }
}
impl std::error::Error for SnapshotLimitExceeded {}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportRange {
    pub from_date: NaiveDate,
    pub to_date: NaiveDate,
}
impl ReportRange {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.from_date < self.to_date
                && (self.to_date - self.from_date).num_days() <= 366
                && self.to_date <= Utc::now().date_naive() + chrono::Duration::days(1),
            "Bounded UTC half-open reporting period required"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PositionSnapshot {
    pub as_of: DateTime<Utc>,
    pub customer_booked: String,
    pub customer_held: String,
    pub settlement_booked: String,
    pub ledger_control_total: String,
    pub reserved_outgoing_principal: String,
    pub submitted_outgoing_principal: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinancialSnapshot {
    pub range: ReportRange,
    pub positions: PositionSnapshot,
    pub batches: Vec<FinancialBatch>,
}
async fn position_in(tx: &mut Transaction<'_, Postgres>) -> Result<PositionSnapshot> {
    // First statement pins the MVCC snapshot and captures its observation time.
    let r=sqlx::query("SELECT clock_timestamp() AS as_of,coalesce(sum(booked::numeric) FILTER(WHERE kind='CUSTOMER'),0)::text AS customer_booked,coalesce(sum(held::numeric) FILTER(WHERE kind='CUSTOMER'),0)::text AS customer_held,coalesce(sum(booked::numeric) FILTER(WHERE kind='SETTLEMENT'),0)::text AS settlement_booked,coalesce(sum(booked::numeric),0)::text AS ledger_control_total FROM core.ledger_accounts").fetch_one(&mut **tx).await?;
    let h=sqlx::query("SELECT coalesce(sum(u.principal::numeric),0)::text AS reserved,coalesce(sum(u.principal::numeric) FILTER(WHERE h.submitted),0)::text AS submitted FROM core.external_reservations r JOIN core.payment_limit_usage u USING(operation_id) JOIN core.holds h USING(operation_id) WHERE h.state='ACTIVE' AND u.state='ACTIVE'").fetch_one(&mut **tx).await?;
    Ok(PositionSnapshot {
        as_of: r.get("as_of"),
        customer_booked: r.get("customer_booked"),
        customer_held: r.get("customer_held"),
        settlement_booked: r.get("settlement_booked"),
        ledger_control_total: r.get("ledger_control_total"),
        reserved_outgoing_principal: h.get("reserved"),
        submitted_outgoing_principal: h.get("submitted"),
    })
}
pub(crate) fn read_snapshot<'a>(
    core: &'a Core,
    range: ReportRange,
) -> BoxFuture<'a, Result<FinancialSnapshot>> {
    Box::pin(async move {
        range.validate()?;
        let mut tx = core.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let positions = position_in(&mut tx).await?;
        let rows=sqlx::query("SELECT * FROM core.journal_batches WHERE business_date>=$1 AND business_date<$2 ORDER BY created_at,id LIMIT 1001").bind(range.from_date).bind(range.to_date).fetch_all(&mut *tx).await?;
        ensure!(rows.len() <= 1000, SnapshotLimitExceeded);
        let ids = rows
            .iter()
            .map(|r| r.get::<Uuid, _>("id"))
            .collect::<Vec<_>>();
        let legs=sqlx::query("SELECT e.batch_id,e.account_id,e.delta,a.kind FROM core.journal_entries e JOIN core.ledger_accounts a ON a.id=e.account_id WHERE e.batch_id=ANY($1) ORDER BY e.batch_id,e.account_id").bind(&ids).fetch_all(&mut *tx).await?;
        let mut entries: BTreeMap<Uuid, Vec<JournalEntry>> = BTreeMap::new();
        let mut kinds: BTreeMap<Uuid, BTreeMap<Uuid, String>> = BTreeMap::new();
        for r in legs {
            let id: Uuid = r.get("batch_id");
            let account: Uuid = r.get("account_id");
            entries.entry(id).or_default().push(JournalEntry {
                account_id: AccountId(account),
                amount: Money::new(r.get("delta"), Currency::synthetic()),
            });
            kinds.entry(id).or_default().insert(account, r.get("kind"));
        }
        let mut batches = vec![];
        for r in rows {
            let id = r.get("id");
            let legs = entries.remove(&id).unwrap_or_default();
            ensure!(
                (2..=16).contains(&legs.len())
                    && legs
                        .iter()
                        .map(|e| i128::from(e.amount.minor_units()))
                        .sum::<i128>()
                        == 0,
                "Complete balanced Core snapshot required"
            );
            batches.push(FinancialBatch {
                journal: JournalBatch {
                    id,
                    operation_id: r.get("operation_id"),
                    posting_kind: r.get("posting_kind"),
                    business_date: r.get("business_date"),
                    original_batch_id: r.get("original_batch"),
                    created_at: r.get("created_at"),
                    entries: legs,
                },
                account_kinds: kinds.remove(&id).unwrap_or_default(),
                chart_code: "synthetic-chart-v1".into(),
            });
        }
        tx.commit().await?;
        Ok(FinancialSnapshot {
            range,
            positions,
            batches,
        })
    })
}
pub(crate) fn read_positions(core: &Core) -> BoxFuture<'_, Result<PositionSnapshot>> {
    Box::pin(async move {
        let mut tx = core.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let p = position_in(&mut tx).await?;
        tx.commit().await?;
        Ok(p)
    })
}
