use crate::{Core, payments::ExecutionPermit};
use anyhow::{Result, ensure};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{AccountId, ActorContext, TrustContext, fingerprint},
};
use bank_rails::{CreditEvidence, CreditKind};
use futures_util::future::BoxFuture;
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;
pub trait CreditPolicy: Send + Sync {
    fn validate<'a>(
        &'a self,
        evidence: &'a CreditEvidence,
    ) -> BoxFuture<'a, Result<ExecutionPermit>>;
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditRecord {
    pub movement_id: Uuid,
    pub request_digest: String,
    pub kind: CreditKind,
    pub original_payment_id: Option<Uuid>,
    pub target_account_id: Option<AccountId>,
    pub amount: bank_platform::domain::Money,
    pub suspense_batch_id: Uuid,
    pub allocation_batch_id: Option<Uuid>,
}
impl Core {
    pub async fn credit_destination(
        &self,
        e: &CreditEvidence,
    ) -> Result<Option<(AccountId, ActorContext)>> {
        let id:Option<Uuid>=match e.event_type {
            CreditKind::InboundSettled=>sqlx::query_scalar("SELECT account_id FROM core.receiving_references WHERE reference=$1").bind(&e.receiving_reference).fetch_optional(&self.pool).await?,
            CreditKind::ReturnSettled=>sqlx::query_scalar("SELECT (instruction->'details'->>'payerAccountId')::uuid FROM core.external_reservations WHERE operation_id=$1").bind(e.original_payment_id).fetch_optional(&self.pool).await?,
        };
        let Some(id) = id else { return Ok(None) };
        let r=sqlx::query("SELECT p.issuer,p.subject FROM core.account_registry a JOIN core.parties p ON p.id=a.party_id WHERE a.id=$1").bind(id).fetch_one(&self.pool).await?;
        Ok(Some((
            AccountId(id),
            ActorContext {
                issuer: r.get("issuer"),
                subject: r.get("subject"),
                trust: TrustContext::Customer,
                client_id: "provider-screening".into(),
                person_id: None,
                represented_party: None,
                roles: vec![],
            },
        )))
    }
    pub async fn return_original_state(&self, id: Uuid) -> Result<String> {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT outcome FROM core.external_finalities WHERE operation_id=$1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(state) = state {
            return Ok(state);
        }
        let reserved: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core.external_reservations WHERE operation_id=$1)",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(if reserved { "RESERVED" } else { "UNKNOWN" }.into())
    }
    pub async fn credit_target_ready(&self, id: AccountId) -> Result<(bool, i64)> {
        let r=sqlx::query("SELECT state='ACTIVE' AND NOT credit_blocked AS ready,version FROM core.ledger_accounts WHERE id=$1 AND kind='CUSTOMER'").bind(id.0).fetch_one(&self.pool).await?;
        Ok((r.get("ready"), r.get("version")))
    }
    pub async fn credit_record(&self, id: Uuid) -> Result<Option<CreditRecord>> {
        let r=sqlx::query("SELECT c.*,a.batch_id AS allocation FROM core.credit_movements c LEFT JOIN core.credit_allocations a USING(movement_id) WHERE c.movement_id=$1").bind(id).fetch_optional(&self.pool).await?;
        r.map(|r| {
            Ok(CreditRecord {
                movement_id: id,
                request_digest: r.get("request_digest"),
                kind: serde_json::from_value(serde_json::json!(r.get::<String, _>("kind")))?,
                original_payment_id: r.get("original_payment_id"),
                target_account_id: r.get::<Option<Uuid>, _>("target_account_id").map(AccountId),
                amount: bank_platform::domain::Money::new(
                    r.get("principal"),
                    bank_platform::domain::Currency::synthetic(),
                ),
                suspense_batch_id: r.get("batch_id"),
                allocation_batch_id: r.get("allocation"),
            })
        })
        .transpose()
    }
    pub async fn record_rail_credit(&self, packet: &SignedDocument) -> Result<CreditRecord> {
        packet.verify_integrity(bank_rails::signing_key().as_bytes())?;
        let e: CreditEvidence = serde_json::from_value(packet.body.clone())?;
        e.validate()?;
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("SELECT * FROM core.record_rail_credit($1,$2)")
            .bind(serde_json::to_value(packet)?)
            .bind(fingerprint(&e)?)
            .fetch_one(&mut *tx)
            .await?;
        if r.get::<bool, _>("created") {
            self.posted(&mut tx, r.get("batch_id"), e.movement_id)
                .await?;
        }
        tx.commit().await?;
        self.credit_record(e.movement_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Credit accounting acknowledgement missing"))
    }
    pub async fn allocate_rail_credit<P: CreditPolicy>(
        &self,
        e: &CreditEvidence,
        policy: &P,
    ) -> Result<CreditRecord> {
        if let Some(prior) = self.credit_record(e.movement_id).await? {
            let digest: String = sqlx::query_scalar(
                "SELECT request_digest FROM core.credit_movements WHERE movement_id=$1",
            )
            .bind(e.movement_id)
            .fetch_one(&self.pool)
            .await?;
            ensure!(
                digest == fingerprint(e)?,
                "Credit allocation scope conflict"
            );
            if prior.allocation_batch_id.is_some() {
                return Ok(prior);
            }
        }
        let permit = policy.validate(e).await?;
        ensure!(
            permit.request_digest == fingerprint(e)? && permit.valid_until > chrono::Utc::now(),
            "Current bound credit permit required"
        );
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("SELECT * FROM core.allocate_rail_credit($1,$2,$3)")
            .bind(e.movement_id)
            .bind(permit.request_digest)
            .bind(permit.valid_until)
            .fetch_one(&mut *tx)
            .await?;
        if r.get::<bool, _>("created") {
            self.posted(&mut tx, r.get("batch_id"), e.movement_id)
                .await?;
        }
        tx.commit().await?;
        self.credit_record(e.movement_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Credit allocation acknowledgement missing"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReturnSummary {
    pub status: String,
    pub received: bank_platform::domain::Money,
    pub credited: bank_platform::domain::Money,
    pub count: i64,
}
impl Core {
    pub async fn payment_returns(&self, id: Uuid) -> Result<ReturnSummary> {
        let r=sqlx::query("SELECT coalesce(sum(c.principal),0)::bigint AS received,coalesce(sum(c.principal) FILTER(WHERE a.movement_id IS NOT NULL),0)::bigint AS credited,count(*) AS count FROM core.credit_movements c LEFT JOIN core.credit_allocations a USING(movement_id) WHERE c.original_payment_id=$1").bind(id).fetch_one(&self.pool).await?;
        let received: i64 = r.get("received");
        let credited: i64 = r.get("credited");
        let principal: Option<i64> = sqlx::query_scalar(
            "SELECT principal FROM core.payment_executions WHERE operation_id=$1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(ReturnSummary {
            status: if received == 0 {
                "NONE"
            } else if credited == 0 {
                "RETURN_RECEIVED"
            } else if Some(credited) == principal {
                "RETURNED"
            } else {
                "PARTIALLY_RETURNED"
            }
            .into(),
            received: bank_platform::domain::Money::new(
                received,
                bank_platform::domain::Currency::synthetic(),
            ),
            credited: bank_platform::domain::Money::new(
                credited,
                bank_platform::domain::Currency::synthetic(),
            ),
            count: r.get("count"),
        })
    }
}
