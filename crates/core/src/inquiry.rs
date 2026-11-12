use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::entitlements::{Action, Entitlements};
use bank_platform::domain::{AccountId, ActorContext, Currency, Money, PartyId};
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    pub account_id: AccountId,
    pub amount: Money,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalBatch {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub posting_kind: String,
    pub business_date: NaiveDate,
    pub original_batch_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub entries: Vec<JournalEntry>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDescriptor {
    pub account_id: AccountId,
    pub party_id: PartyId,
    pub product_version_id: Uuid,
}
/// A narrow immutable read capability for Data/Finance; it cannot reserve or post.
pub trait Records: Send + Sync {
    fn journal<'a>(&'a self, id: Uuid) -> BoxFuture<'a, Result<JournalBatch>>;
    fn account<'a>(&'a self, id: AccountId) -> BoxFuture<'a, Result<AccountDescriptor>>;
}
impl Records for Core {
    fn journal<'a>(&'a self, id: Uuid) -> BoxFuture<'a, Result<JournalBatch>> {
        Box::pin(async move {
            let row = sqlx::query("SELECT * FROM core.journal_batches WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
            let entries=sqlx::query("SELECT account_id,delta FROM core.journal_entries WHERE batch_id=$1 ORDER BY account_id").bind(id).fetch_all(&self.pool).await?.into_iter().map(|r|JournalEntry{account_id:AccountId(r.get("account_id")),amount:Money::new(r.get("delta"),Currency::synthetic())}).collect();
            Ok(JournalBatch {
                id,
                operation_id: row.get("operation_id"),
                posting_kind: row.get("posting_kind"),
                business_date: row.get("business_date"),
                original_batch_id: row.get("original_batch"),
                created_at: row.get("created_at"),
                entries,
            })
        })
    }
    fn account<'a>(&'a self, id: AccountId) -> BoxFuture<'a, Result<AccountDescriptor>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT party_id,product_version_id FROM core.account_registry WHERE id=$1",
            )
            .bind(id.0)
            .fetch_one(&self.pool)
            .await?;
            Ok(AccountDescriptor {
                account_id: id,
                party_id: PartyId(row.get("party_id")),
                product_version_id: row.get("product_version_id"),
            })
        })
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryCursor {
    pub snapshot_id: Uuid,
    pub after: i32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Movement {
    pub id: String,
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub amount: Money,
    pub posting_kind: String,
    pub business_date: NaiveDate,
    pub status: &'static str,
    pub original_batch_id: Option<Uuid>,
    pub posted_at: DateTime<Utc>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<Movement>,
    pub next: Option<HistoryCursor>,
    pub total: i32,
    pub as_of: DateTime<Utc>,
    pub source: &'static str,
}
impl Core {
    pub async fn cleanup_history(&self) -> Result<u64> {
        Ok(
            sqlx::query("DELETE FROM core.history_snapshots WHERE expires_at<=now()")
                .execute(&self.pool)
                .await?
                .rows_affected(),
        )
    }
    pub async fn history(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        account: AccountId,
        range: (NaiveDate, NaiveDate),
        limit: i64,
        cursor: Option<HistoryCursor>,
    ) -> Result<HistoryPage> {
        ensure!(
            (1..=100).contains(&limit)
                && range.0 < range.1
                && (range.1 - range.0).num_days() <= 366,
            "Bounded history range/page required"
        );
        let authority = self.authority(account, Action::ReadAccount).await?;
        ensure!(
            controls
                .authorize(actor, Action::ReadAccount, Some(&authority), account.0)
                .await?,
            "History inquiry denied"
        );
        let cursor = if let Some(cursor) = cursor {
            ensure!(cursor.after >= 0, "Invalid history cursor");
            cursor
        } else {
            let mut tx = self.pool.begin().await?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
                .execute(&mut *tx)
                .await?;
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO core.history_snapshots(id,account_id,issuer,subject,from_date,to_date) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(account.0).bind(&actor.issuer).bind(&actor.subject).bind(range.0).bind(range.1).execute(&mut *tx).await?;
            let count:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT 1 FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id WHERE e.account_id=$1 AND b.business_date>=$2 AND b.business_date<$3 LIMIT 10001) bounded").bind(account.0).bind(range.0).bind(range.1).fetch_one(&mut *tx).await?;
            ensure!(
                count <= 10000,
                "Narrow the history period before creating a snapshot"
            );
            sqlx::query("INSERT INTO core.history_items(snapshot_id,ordinal,batch_id) SELECT $1,row_number() OVER(ORDER BY b.created_at,b.id)::integer,b.id FROM core.journal_entries e JOIN core.journal_batches b ON b.id=e.batch_id WHERE e.account_id=$2 AND b.business_date>=$3 AND b.business_date<$4").bind(id).bind(account.0).bind(range.0).bind(range.1).execute(&mut *tx).await?;
            sqlx::query("UPDATE core.history_snapshots SET item_count=$2 WHERE id=$1")
                .bind(id)
                .bind(count as i32)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            HistoryCursor {
                snapshot_id: id,
                after: 0,
            }
        };
        let snapshot=sqlx::query("SELECT * FROM core.history_snapshots WHERE id=$1 AND account_id=$2 AND issuer=$3 AND subject=$4 AND from_date=$5 AND to_date=$6 AND expires_at>now()").bind(cursor.snapshot_id).bind(account.0).bind(&actor.issuer).bind(&actor.subject).bind(range.0).bind(range.1).fetch_one(&self.pool).await?;
        let rows=sqlx::query("SELECT i.ordinal,b.*,e.delta FROM core.history_items i JOIN core.journal_batches b ON b.id=i.batch_id JOIN core.journal_entries e ON e.batch_id=b.id AND e.account_id=$2 WHERE i.snapshot_id=$1 AND i.ordinal>$3 ORDER BY i.ordinal LIMIT $4").bind(cursor.snapshot_id).bind(account.0).bind(cursor.after).bind(limit).fetch_all(&self.pool).await?;
        let after = rows
            .last()
            .map(|r| r.get("ordinal"))
            .unwrap_or(cursor.after);
        let total: i32 = snapshot.get("item_count");
        let items = rows
            .into_iter()
            .map(|r| {
                let batch: Uuid = r.get("id");
                Movement {
                    id: format!("{batch}:{}", account.0),
                    batch_id: batch,
                    operation_id: r.get("operation_id"),
                    amount: Money::new(r.get("delta"), Currency::synthetic()),
                    posting_kind: r.get("posting_kind"),
                    business_date: r.get("business_date"),
                    status: "POSTED",
                    original_batch_id: r.get("original_batch"),
                    posted_at: r.get("created_at"),
                }
            })
            .collect();
        Ok(HistoryPage {
            items,
            next: if after < total {
                Some(HistoryCursor {
                    snapshot_id: cursor.snapshot_id,
                    after,
                })
            } else {
                None
            },
            total,
            as_of: snapshot.get("as_of"),
            source: "CORE_IMMUTABLE_SNAPSHOT",
        })
    }
}
