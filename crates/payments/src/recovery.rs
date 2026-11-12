//! Explicit, durable recovery commands preserve the original economic identity.
use crate::{Payments, Progress};
use anyhow::{Result, ensure};
use bank_controls::entitlements::Action;
use bank_platform::{
    domain::{ActorContext, TrustContext, fingerprint},
    operations::JobSpec,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
pub(crate) enum RecoveryPage {
    Intents,
    Requests,
    Credits,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryAction {
    Inquire,
    Cancel,
}
impl RecoveryAction {
    fn code(self) -> &'static str {
        match self {
            Self::Inquire => "INQUIRE",
            Self::Cancel => "CANCEL",
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRequest {
    pub id: Uuid,
    pub payment_id: Uuid,
    pub action: RecoveryAction,
    pub state: String,
    pub reason_code: String,
    pub operation_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
fn record(r: &sqlx::postgres::PgRow) -> Result<RecoveryRequest> {
    Ok(RecoveryRequest {
        id: r.get("id"),
        payment_id: r.get("payment_id"),
        action: serde_json::from_value(serde_json::json!(r.get::<String, _>("action")))?,
        state: r.get("state"),
        reason_code: r.get("reason_code"),
        operation_id: r.get("operation_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}
impl Payments {
    pub async fn customer_cancellation(
        &self,
        actor: &ActorContext,
        id: Uuid,
        key: Uuid,
    ) -> Result<RecoveryRequest> {
        self.owned_intent(actor, id).await?;
        self.accept_recovery(
            actor,
            id,
            key,
            RecoveryAction::Cancel,
            "CUSTOMER_CANCELLATION_REQUEST",
        )
        .await
    }
    pub async fn staff_recovery(
        &self,
        actor: &ActorContext,
        id: Uuid,
        key: Uuid,
        action: RecoveryAction,
        reason: &str,
    ) -> Result<RecoveryRequest> {
        self.recovery_authorized(actor).await?;
        ensure!(
            (4..=240).contains(&reason.chars().count()) && reason.chars().all(|c| !c.is_control()),
            "A bounded operator reason is required"
        );
        self.accept_recovery(actor, id, key, action, reason).await
    }
    pub async fn recovery_authorized(&self, actor: &ActorContext) -> Result<()> {
        ensure!(
            actor.trust == TrustContext::Workforce
                && self.controls.payment_inquiry_authorized(actor).await?
                && self
                    .controls
                    .authorize(
                        actor,
                        Action::ManageRails,
                        None,
                        crate::policy::SIMULATED_CLEARING
                    )
                    .await?,
            "Current payment operator role and payments queue required"
        );
        Ok(())
    }
    async fn accept_recovery(
        &self,
        actor: &ActorContext,
        id: Uuid,
        key: Uuid,
        action: RecoveryAction,
        reason: &str,
    ) -> Result<RecoveryRequest> {
        let intent = self.intent(id).await?;
        let quote = self.quote_record(intent.quote_id).await?;
        let digest =
            fingerprint(&serde_json::json!({"paymentId":id,"action":action,"reason":reason}))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT id FROM payments.intents WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        // Recheck after waiting for execution's lock, before accepting a new command.
        match actor.trust {
            TrustContext::Customer => {
                self.authorize(
                    actor,
                    quote.instruction.details.payer_account_id,
                    Action::ReadAccount,
                )
                .await?;
            }
            TrustContext::Workforce => {
                self.recovery_authorized(actor).await?;
            }
            _ => anyhow::bail!("Customer or workforce recovery authority required"),
        }
        let prior=sqlx::query("SELECT * FROM payments.recovery_requests WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await?;
        if let Some(prior) = prior {
            ensure!(
                prior.get::<String, _>("fingerprint") == digest,
                "Recovery key reused with changed input"
            );
            let saved = record(&prior)?;
            tx.commit().await?;
            self.schedule_recovery(saved.id).await?;
            return self.recovery_request(saved.id).await;
        }
        if action == RecoveryAction::Inquire {
            ensure!(
                quote.instruction.details.rail == "SIMULATED"
                    && self
                        .core
                        .external_reservation(&quote.instruction)
                        .await?
                        .is_some_and(|r| r.submitted),
                "Inquiry requires an original submitted external instruction"
            );
            let recent:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payments.recovery_requests WHERE payment_id=$1 AND action='INQUIRE' AND created_at>now()-interval '30 seconds')").bind(id).fetch_one(&mut *tx).await?;
            ensure!(!recent, "Recovery inquiry cooldown is active");
        }
        let request = Uuid::new_v4();
        sqlx::query("INSERT INTO payments.recovery_requests(id,payment_id,issuer,subject,client_id,request_key,actor_context,action,operator_reason,fingerprint) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(request).bind(id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(serde_json::to_value(actor)?).bind(action.code()).bind(reason).bind(digest).execute(&mut *tx).await?;
        Self::audit_in(&mut tx, actor, "payment.recovery-requested", request).await?;
        sqlx::query("UPDATE payments.intents SET version=version+1,updated_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        self.fact(&mut tx, actor, id, "payment.recovery-requested")
            .await?;
        tx.commit().await?;
        self.schedule_recovery(request).await?;
        self.recovery_request(request).await
    }
    async fn recovery_request(&self, id: Uuid) -> Result<RecoveryRequest> {
        record(
            &sqlx::query("SELECT * FROM payments.recovery_requests WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    async fn schedule_recovery(&self, id: Uuid) -> Result<()> {
        let r = sqlx::query("SELECT * FROM payments.recovery_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        if r.get::<Option<Uuid>, _>("operation_id").is_some() {
            return Ok(());
        }
        let actor: ActorContext = serde_json::from_value(r.get("actor_context"))?;
        let op = self
            .operations
            .accept(
                &actor,
                "payment.recovery",
                &id.to_string(),
                &serde_json::json!({"requestId":id}),
                JobSpec {
                    kind: "payment.recover".into(),
                    step_key: "recover-original".into(),
                    payload: serde_json::json!({"requestId":id}),
                    delay_seconds: 0,
                },
            )
            .await?;
        sqlx::query("UPDATE payments.recovery_requests SET operation_id=$2 WHERE id=$1 AND operation_id IS NULL").bind(id).bind(op.id).execute(&self.pool).await?;
        Ok(())
    }
    pub(crate) async fn cancellation_requested_in(
        tx: &mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payments.recovery_requests WHERE payment_id=$1 AND action='CANCEL' AND state<>'COMPLETED')").bind(id).fetch_one(&mut **tx).await?)
    }
    pub(crate) async fn cancel_unsubmitted_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &ActorContext,
        q: &crate::types::QuoteRecord,
    ) -> Result<bool> {
        let id = q.instruction.operation_id;
        if !Self::cancellation_requested_in(tx, id).await? {
            return Ok(false);
        }
        if let Some(reservation) = self.core.external_reservation(&q.instruction).await? {
            if reservation.submitted {
                return Ok(false);
            }
            self.core
                .release_external_unsubmitted(&q.instruction)
                .await?;
        }
        self.transition(
            tx,
            actor,
            id,
            "CANCELLED",
            "CANCELLED_BEFORE_SUBMISSION",
            None,
        )
        .await?;
        Ok(true)
    }
    pub async fn process_recovery(&self, request: Uuid) -> Result<Progress> {
        let command = self.recovery_request(request).await?;
        if command.state == "COMPLETED" {
            return Ok(Progress::Done {
                state: command.reason_code,
            });
        }
        let progress = self.process(command.payment_id).await?;
        let (state, reason) = match &progress {
            Progress::Done { state } => (
                "COMPLETED",
                if command.action == RecoveryAction::Cancel {
                    if state == "CANCELLED" {
                        "CANCELLATION_CONFIRMED"
                    } else {
                        "CANCELLATION_TOO_LATE_OR_NOT_REQUIRED"
                    }
                } else {
                    "ORIGINAL_FINANCIAL_OUTCOME_RECOVERED"
                },
            ),
            Progress::Pending => ("WAITING", "ORIGINAL_OUTCOME_PENDING"),
            Progress::Review { .. } => ("REVIEW_REQUIRED", "OWNED_RECOVERY_REVIEW"),
        };
        self.recovery_state(request, state, reason).await?;
        if matches!(progress, Progress::Review { .. }) {
            self.own_exception(command.payment_id).await?;
        }
        Ok(progress)
    }
    async fn recovery_state(&self, id: Uuid, state: &str, reason: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let r=sqlx::query("UPDATE payments.recovery_requests SET state=$2,reason_code=$3,version=version+1,updated_at=now() WHERE id=$1 AND state<>'COMPLETED' AND (state<>$2 OR reason_code<>$3) RETURNING actor_context").bind(id).bind(state).bind(reason).fetch_optional(&mut *tx).await?;
        if let Some(r) = r {
            let actor: ActorContext = serde_json::from_value(r.get("actor_context"))?;
            Self::audit_in(&mut tx, &actor, "payment.recovery-state-recorded", id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn recover_requests(&self) -> Result<()> {
        let rows = self.recovery_page(RecoveryPage::Requests).await?;
        for r in rows {
            let id = r.get("id");
            if let Some(op) = r.get::<Option<Uuid>, _>("operation_id") {
                let actor: ActorContext = serde_json::from_value(r.get("actor_context"))?;
                if self.operations.status(&actor, op).await?.state == "REVIEW_REQUIRED" {
                    let payment = r.get("payment_id");
                    self.recovery_state(id, "REVIEW_REQUIRED", "RECOVERY_BUDGET_EXHAUSTED")
                        .await?;
                    self.own_exception(payment).await?;
                    let intent = self.intent(payment).await?;
                    let q = self.quote_record(intent.quote_id).await?;
                    if self.core.payment_receipt(&q.instruction).await?.is_some()
                        || self.core.external_finality(&q.instruction).await?.is_some()
                    {
                        self.process_recovery(id).await?;
                    }
                }
            } else {
                self.schedule_recovery(id).await?;
            }
        }
        Ok(())
    }
    pub async fn recovery_history(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<Vec<RecoveryRequest>> {
        if actor.trust == TrustContext::Customer {
            self.owned_intent(actor, id).await?;
        } else {
            ensure!(
                self.controls.payment_inquiry_authorized(actor).await?,
                "Payment inquiry role and queue required"
            );
        }
        let rows=sqlx::query("SELECT * FROM payments.recovery_requests WHERE payment_id=$1 ORDER BY created_at DESC,id LIMIT 50").bind(id).fetch_all(&self.pool).await?;
        rows.iter().map(record).collect()
    }
    pub(crate) async fn cancellation_summary(&self, id: Uuid) -> Result<Option<RecoveryRequest>> {
        sqlx::query("SELECT * FROM payments.recovery_requests WHERE payment_id=$1 AND action='CANCEL' ORDER BY created_at DESC,id LIMIT 1").bind(id).fetch_optional(&self.pool).await?.map(|r|record(&r)).transpose()
    }
    pub(crate) async fn recovery_page(
        &self,
        page: RecoveryPage,
    ) -> Result<Vec<sqlx::postgres::PgRow>> {
        let (lock, read, query, write, id_column) = match page {
            RecoveryPage::Intents => (
                2,
                "SELECT last_intent_id FROM payments.reconciliation_cursor WHERE id FOR UPDATE",
                "SELECT id,operation_id FROM payments.intents WHERE id>$1 AND state IN ('ACCEPTED','CONTROLS_PENDING','CONTROL_REVIEW','RESERVED','SUBMITTING','SUBMITTED','OUTCOME_UNKNOWN','RAIL_PAUSED','RECONCILING','CANCELLATION_PENDING') ORDER BY id LIMIT 50",
                "UPDATE payments.reconciliation_cursor SET last_intent_id=$1 WHERE id",
                "id",
            ),
            RecoveryPage::Requests => (
                3,
                "SELECT last_request_id FROM payments.reconciliation_cursor WHERE id FOR UPDATE",
                "SELECT id,payment_id,operation_id,actor_context FROM payments.recovery_requests WHERE id>$1 AND state<>'COMPLETED' ORDER BY id LIMIT 50",
                "UPDATE payments.reconciliation_cursor SET last_request_id=$1 WHERE id",
                "id",
            ),
            RecoveryPage::Credits => (
                4,
                "SELECT last_credit_id FROM payments.reconciliation_cursor WHERE id FOR UPDATE",
                "SELECT movement_id,operation_id,state FROM payments.credit_intents WHERE movement_id>$1 AND state<>'CREDITED' ORDER BY movement_id LIMIT 50",
                "UPDATE payments.reconciliation_cursor SET last_credit_id=$1 WHERE id",
                "movement_id",
            ),
        };
        let mut tx = self.pool.begin().await?;
        let claimed: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(154,$1)")
            .bind(lock)
            .fetch_one(&mut *tx)
            .await?;
        if !claimed {
            return Ok(vec![]);
        }
        let after: Uuid = sqlx::query_scalar(read).fetch_one(&mut *tx).await?;
        let rows = sqlx::query(query).bind(after).fetch_all(&mut *tx).await?;
        let next = rows
            .last()
            .map(|r| r.get::<Uuid, _>(id_column))
            .unwrap_or(Uuid::nil());
        sqlx::query(write).bind(next).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(rows)
    }
}
