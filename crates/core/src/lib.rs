//! Authoritative synthetic ledger. Channels must resolve current entitlements before invoking commands.
pub mod accounts;
pub mod accruals;
pub mod corrections;
pub mod credit;
pub mod credit_accounting;
pub mod finality;
pub mod finance;
pub mod finance_snapshot;
pub mod inquiry;
pub mod payment_accounting;
pub mod payments;
pub mod products;
pub mod profile;
pub mod rail;
pub mod reconciliation;
use anyhow::{Result, ensure};
use bank_platform::{
    database::Module,
    domain::{AccountId, Currency, EventEnvelope, EventId, Money},
    events::EventStore,
};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone)]
pub struct Core {
    pub pool: PgPool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    pub account_id: AccountId,
    pub booked: Money,
    pub held: Money,
    pub available: Money,
    pub version: i64,
    pub as_of: chrono::DateTime<chrono::Utc>,
    pub source: &'static str,
}
impl Core {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn balance(&self, account: AccountId) -> Result<Balance> {
        let row=sqlx::query("SELECT booked,held,version,clock_timestamp() AS as_of FROM core.ledger_accounts WHERE id=$1").bind(account.0).fetch_one(&self.pool).await?;
        let booked: i64 = row.get("booked");
        let held: i64 = row.get("held");
        Ok(Balance {
            account_id: account,
            booked: Money::new(booked, Currency::synthetic()),
            held: Money::new(held, Currency::synthetic()),
            available: Money::new(
                booked
                    .checked_sub(held)
                    .ok_or_else(|| anyhow::anyhow!("Balance overflow"))?,
                Currency::synthetic(),
            ),
            version: row.get("version"),
            as_of: row.get("as_of"),
            source: "CORE_AUTHORITATIVE",
        })
    }
    pub async fn reserve(&self, operation: Uuid, account: AccountId, amount: Money) -> Result<()> {
        amount.require_positive()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.reserve($1,$2,$3)")
            .bind(operation)
            .bind(account.0)
            .bind(amount.minor_units())
            .execute(&mut *tx)
            .await?;
        self.hold_fact(&mut tx, operation).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn mark_submitted(&self, operation: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.mark_generic_submitted($1)")
            .bind(operation)
            .execute(&mut *tx)
            .await?;
        self.hold_fact(&mut tx, operation).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Available only before submission. Evidence-backed post-submission release
    /// is exposed with the authenticated rail lifecycle, never by caller-supplied UUID.
    pub async fn release(&self, operation: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.release_unsubmitted($1)")
            .bind(operation)
            .execute(&mut *tx)
            .await?;
        self.hold_fact(&mut tx, operation).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn transfer(
        &self,
        operation: Uuid,
        from: AccountId,
        to: AccountId,
        amount: Money,
        hold: Option<Uuid>,
    ) -> Result<Uuid> {
        amount.require_positive()?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM core.transfer($1,$2,$3,$4,$5)")
            .bind(operation)
            .bind(from.0)
            .bind(to.0)
            .bind(amount.minor_units())
            .bind(hold)
            .fetch_one(&mut *tx)
            .await?;
        let id = row.get("batch_id");
        if row.get("created") {
            self.posted(&mut tx, id, operation).await?;
            if let Some(hold) = hold {
                self.hold_fact(&mut tx, hold).await?;
            }
        }
        tx.commit().await?;
        Ok(id)
    }
    /// Only the explicitly synthetic local profile may use this fixture command.
    pub async fn fund_synthetic(
        &self,
        operation: Uuid,
        account: AccountId,
        clearing: AccountId,
        amount: Money,
    ) -> Result<Uuid> {
        ensure!(
            std::env::var("BANK_PROFILE").unwrap_or_else(|_| "local".into()) == "local",
            "Synthetic funding is local only"
        );
        amount.require_positive()?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM core.fund_synthetic($1,$2,$3,$4)")
            .bind(operation)
            .bind(account.0)
            .bind(clearing.0)
            .bind(amount.minor_units())
            .fetch_one(&mut *tx)
            .await?;
        let id = row.get("batch_id");
        if row.get("created") {
            self.posted(&mut tx, id, operation).await?;
        }
        tx.commit().await?;
        Ok(id)
    }
    pub async fn reverse(
        &self,
        operation: Uuid,
        original: Uuid,
        controls: &bank_controls::entitlements::Entitlements,
        actor: &bank_platform::domain::ActorContext,
        proposal: Uuid,
    ) -> Result<Uuid> {
        let approval = controls
            .approved_change(
                actor,
                proposal,
                bank_controls::governance::ChangeKind::Correction,
                original,
                &correction_details(original),
            )
            .await?;
        let (maker, checker) = approval.people();
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM core.reverse_batch($1,$2,$3,$4,$5)")
            .bind(operation)
            .bind(original)
            .bind(approval.id())
            .bind(maker)
            .bind(checker)
            .fetch_one(&mut *tx)
            .await?;
        let id = row.get("batch_id");
        if row.get("created") {
            self.posted(&mut tx, id, operation).await?;
        }
        tx.commit().await?;
        Ok(id)
    }
    async fn hold_fact(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        operation: Uuid,
    ) -> Result<()> {
        let row=sqlx::query("SELECT h.* FROM core.holds h WHERE operation_id=$1 AND NOT EXISTS(SELECT 1 FROM core.outbox o WHERE o.aggregate_id=h.operation_id AND o.aggregate_version=h.version)").bind(operation).fetch_optional(&mut **tx).await?;
        if let Some(row) = row {
            EventStore::new(self.pool.clone(),Module::Core).stage(tx,&EventEnvelope{event_id:EventId::new(),event_type:"hold.changed".into(),schema_version:1,producer:"core".into(),aggregate_id:operation,aggregate_version:row.get("version"),occurred_at:chrono::Utc::now(),correlation_id:operation,causation_id:operation.to_string(),data:serde_json::json!({"accountId":row.get::<Uuid,_>("account_id"),"state":row.get::<String,_>("state"),"submitted":row.get::<bool,_>("submitted")})}).await?;
        }
        Ok(())
    }
    async fn posted(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        batch: Uuid,
        operation: Uuid,
    ) -> Result<()> {
        EventStore::new(self.pool.clone(), Module::Core)
            .stage(
                tx,
                &EventEnvelope {
                    event_id: EventId::new(),
                    event_type: "journal.batch-posted".into(),
                    schema_version: 1,
                    producer: "core".into(),
                    aggregate_id: batch,
                    aggregate_version: 1,
                    occurred_at: chrono::Utc::now(),
                    correlation_id: operation,
                    causation_id: operation.to_string(),
                    data: serde_json::json!({"batchId":batch,"operationId":operation}),
                },
            )
            .await
    }
    pub async fn posting_by_key(&self, operation: Uuid, kind: &str) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM core.journal_batches WHERE operation_id=$1 AND posting_kind=$2",
        )
        .bind(operation)
        .bind(kind)
        .fetch_optional(&self.pool)
        .await?)
    }
}

pub fn correction_details(original: Uuid) -> serde_json::Value {
    serde_json::json!({"originalBatchId":original,"kind":"FULL_REVERSAL"})
}
