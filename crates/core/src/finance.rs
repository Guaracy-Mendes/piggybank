//! Immutable ledger sources for Finance. This capability exposes no money commands.
use crate::{
    Core,
    inquiry::{JournalBatch, Records},
};
use anyhow::{Result, ensure};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialBatch {
    pub journal: JournalBatch,
    pub chart_code: String,
    pub account_kinds: BTreeMap<Uuid, String>,
}
pub trait FinanceRecords: Send + Sync {
    fn credit_accounting<'a>(
        &'a self,
        range: super::finance_snapshot::ReportRange,
        batches: &'a [Uuid],
        ids: &'a [Uuid],
    ) -> BoxFuture<'a, Result<super::credit_accounting::CreditAccountingSnapshot>>;
    fn payment_accounting<'a>(
        &'a self,
        range: super::finance_snapshot::ReportRange,
        batches: &'a [Uuid],
        operations: &'a [Uuid],
    ) -> BoxFuture<'a, Result<super::payment_accounting::PaymentAccountingSnapshot>>;
    fn batch_ids<'a>(&'a self, after: Uuid) -> BoxFuture<'a, Result<Vec<Uuid>>>;
    fn financial_batch<'a>(&'a self, id: Uuid) -> BoxFuture<'a, Result<FinancialBatch>>;
    fn snapshot<'a>(
        &'a self,
        range: super::finance_snapshot::ReportRange,
    ) -> BoxFuture<'a, Result<super::finance_snapshot::FinancialSnapshot>>;
    fn positions<'a>(&'a self) -> BoxFuture<'a, Result<super::finance_snapshot::PositionSnapshot>>;
}
impl FinanceRecords for Core {
    fn credit_accounting<'a>(
        &'a self,
        range: super::finance_snapshot::ReportRange,
        batches: &'a [Uuid],
        ids: &'a [Uuid],
    ) -> BoxFuture<'a, Result<super::credit_accounting::CreditAccountingSnapshot>> {
        Box::pin(Core::credit_accounting(self, range, batches, ids))
    }
    fn payment_accounting<'a>(
        &'a self,
        range: super::finance_snapshot::ReportRange,
        batches: &'a [Uuid],
        operations: &'a [Uuid],
    ) -> BoxFuture<'a, Result<super::payment_accounting::PaymentAccountingSnapshot>> {
        Box::pin(Core::payment_accounting(self, range, batches, operations))
    }
    fn snapshot<'a>(
        &'a self,
        range: super::finance_snapshot::ReportRange,
    ) -> BoxFuture<'a, Result<super::finance_snapshot::FinancialSnapshot>> {
        super::finance_snapshot::read_snapshot(self, range)
    }
    fn positions<'a>(&'a self) -> BoxFuture<'a, Result<super::finance_snapshot::PositionSnapshot>> {
        super::finance_snapshot::read_positions(self)
    }
    fn batch_ids<'a>(&'a self, after: Uuid) -> BoxFuture<'a, Result<Vec<Uuid>>> {
        Box::pin(async move {
            Ok(sqlx::query_scalar(
                "SELECT id FROM core.journal_batches WHERE id>$1 ORDER BY id LIMIT 50",
            )
            .bind(after)
            .fetch_all(&self.pool)
            .await?)
        })
    }
    fn financial_batch<'a>(&'a self, id: Uuid) -> BoxFuture<'a, Result<FinancialBatch>> {
        Box::pin(async move {
            let journal = self.journal(id).await?;
            let rows=sqlx::query("SELECT a.id,a.kind FROM core.journal_entries e JOIN core.ledger_accounts a ON a.id=e.account_id WHERE e.batch_id=$1 ORDER BY a.id").bind(id).fetch_all(&self.pool).await?;
            let account_kinds = rows
                .into_iter()
                .map(|r| (r.get("id"), r.get("kind")))
                .collect::<BTreeMap<_, _>>();
            ensure!(
                account_kinds.len() == journal.entries.len(),
                "Complete Core journal source required"
            );
            ensure!(
                journal
                    .entries
                    .iter()
                    .map(|e| i128::from(e.amount.minor_units()))
                    .sum::<i128>()
                    == 0,
                "Balanced Core journal required"
            );
            // Every enabled Core product is currently bound to this explicit synthetic chart.
            let source = FinancialBatch {
                journal,
                chart_code: "synthetic-chart-v1".into(),
                account_kinds,
            };
            Ok(source)
        })
    }
}
