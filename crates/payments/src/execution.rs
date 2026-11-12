use crate::{
    Payments,
    types::{Intent, Progress, QuoteRecord},
};
use anyhow::{Result, ensure};
use bank_controls::{
    cases::{CaseKind, Cases, NewCase},
    checks::{Outcome, Purpose},
    entitlements::Action,
};
use bank_core::payments::{ExecutionPermit, InternalInstruction, PaymentPolicy};
use bank_platform::{
    domain::{ActorContext, fingerprint},
    operations::JobSpec,
};
use chrono::Utc;
use futures_util::future::BoxFuture;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
pub(crate) struct Gates<'a> {
    pub(crate) payments: &'a Payments,
    pub(crate) intent: &'a Intent,
    pub(crate) quote: &'a QuoteRecord,
    pub(crate) recipient: &'a crate::types::Beneficiary,
}
impl PaymentPolicy for Gates<'_> {
    fn validate<'a>(
        &'a self,
        instruction: &'a InternalInstruction,
    ) -> BoxFuture<'a, Result<ExecutionPermit>> {
        Box::pin(async move {
            let p = self.payments;
            let actor = &self.intent.actor;
            let q = self.quote;
            ensure!(
                fingerprint(instruction)? == fingerprint(&q.instruction)?
                    && self.intent.id == instruction.operation_id
                    && q.id.to_string() == instruction.details.quote_id,
                "Execution details changed"
            );
            p.current_bound_quote(actor, q, self.recipient, q.instruction.payee_account_id)
                .await?;
            let challenge = self
                .intent
                .challenge
                .ok_or_else(|| anyhow::anyhow!("Action approval missing"))?;
            let authority = p
                .core
                .authority(instruction.details.payer_account_id, Action::CreatePayment)
                .await?;
            let approval = p
                .controls
                .approved_payment_until(
                    actor,
                    challenge,
                    instruction.operation_id,
                    &instruction.details,
                    &authority,
                )
                .await?;
            let proof = p
                .identity
                .bound_step_up_until(
                    actor,
                    challenge,
                    instruction.operation_id,
                    &fingerprint(&instruction.details)?,
                )
                .await?;
            let controls = p
                .checks
                .evaluate(
                    instruction.operation_id,
                    Purpose::Payment,
                    &fingerprint(instruction)?,
                )
                .await?;
            ensure!(
                controls.outcome == Outcome::Allow,
                "Current payment controls required"
            );
            Ok(ExecutionPermit {
                request_digest: fingerprint(instruction)?,
                valid_until: q
                    .expires_at
                    .min(approval)
                    .min(proof)
                    .min(controls.valid_until),
            })
        })
    }
}
pub(crate) fn business_guard(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<sqlx::Error>() {
        None | Some(sqlx::Error::RowNotFound) => true,
        Some(sqlx::Error::Database(db)) => {
            matches!(db.code().as_deref(), Some("P0001" | "22003" | "23514"))
        }
        _ => false,
    }
}
impl Payments {
    pub async fn process(&self, id: Uuid) -> Result<Progress> {
        self.process_with_ack_fault(id, false).await
    }
    /// Deterministic crash boundary for the local integration harness: the committed Core receipt remains authoritative.
    pub async fn process_with_ack_fault(&self, id: Uuid, lose_core_ack: bool) -> Result<Progress> {
        let mut intent = self.intent(id).await?;
        let q = self.quote_record(intent.quote_id).await?;
        if q.instruction.details.rail == "SIMULATED" {
            return self.process_external(id).await;
        }
        let mut tx = self.pool.begin().await?;
        let claimed: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,130))")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        if !claimed {
            tx.commit().await?;
            return Ok(Progress::Pending);
        }
        let current =
            sqlx::query("SELECT state,challenge_id FROM payments.intents WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        intent.state = current.get("state");
        intent.challenge = current.get("challenge_id");
        let recipient_row=sqlx::query("SELECT * FROM payments.beneficiaries WHERE id=$1 AND issuer=$2 AND subject=$3 FOR SHARE").bind(q.beneficiary_id).bind(&intent.actor.issuer).bind(&intent.actor.subject).fetch_one(&mut *tx).await?;
        let recipient = crate::recipients::record(&recipient_row);
        if let Some(batch) = self.core.payment_receipt(&q.instruction).await? {
            self.transition(
                &mut tx,
                &intent.actor,
                id,
                "SETTLED",
                "CORE_BOOKED",
                Some(batch),
            )
            .await?;
            tx.commit().await?;
            return Ok(Progress::Done {
                state: "SETTLED".into(),
            });
        }
        if ["EXPIRED", "DECLINED", "BLOCKED", "CANCELLED"].contains(&intent.state.as_str()) {
            tx.commit().await?;
            return Ok(Progress::Done {
                state: intent.state,
            });
        }
        if self
            .cancel_unsubmitted_in(&mut tx, &intent.actor, &q)
            .await?
        {
            tx.commit().await?;
            return Ok(Progress::Done {
                state: "CANCELLED".into(),
            });
        }
        ensure!(
            intent.state != "DRAFT",
            "Unconfirmed instruction cannot execute"
        );
        if q.expires_at <= Utc::now() {
            self.transition(&mut tx, &intent.actor, id, "EXPIRED", "QUOTE_EXPIRED", None)
                .await?;
            tx.commit().await?;
            return Ok(Progress::Done {
                state: "EXPIRED".into(),
            });
        }
        if let Err(error) = self
            .current_bound_quote(
                &intent.actor,
                &q,
                &recipient,
                q.instruction.payee_account_id,
            )
            .await
        {
            if !business_guard(&error) {
                return Err(error);
            }
            self.transition(
                &mut tx,
                &intent.actor,
                id,
                "BLOCKED",
                "CURRENT_INSTRUCTION_GUARD",
                None,
            )
            .await?;
            tx.commit().await?;
            return Ok(Progress::Done {
                state: "BLOCKED".into(),
            });
        }
        let digest = fingerprint(&q.instruction)?;
        for kind in Purpose::Payment.required() {
            self.checks
                .request(&intent.actor, id, *kind, &digest)
                .await?;
        }
        let gate = self.checks.evaluate(id, Purpose::Payment, &digest).await?;
        match gate.outcome {
            Outcome::Pending => {
                self.transition(
                    &mut tx,
                    &intent.actor,
                    id,
                    "CONTROLS_PENDING",
                    "CURRENT_CONTROLS_PENDING",
                    None,
                )
                .await?;
                tx.commit().await?;
                return Ok(Progress::Pending);
            }
            Outcome::Review => {
                self.transition(
                    &mut tx,
                    &intent.actor,
                    id,
                    "CONTROL_REVIEW",
                    "CONTROL_REVIEW_REQUIRED",
                    None,
                )
                .await?;
                tx.commit().await?;
                return Ok(Progress::Review {
                    code: "CONTROL_REVIEW_REQUIRED".into(),
                });
            }
            Outcome::Deny => {
                self.transition(
                    &mut tx,
                    &intent.actor,
                    id,
                    "DECLINED",
                    "CURRENT_CONTROL_DENIAL",
                    None,
                )
                .await?;
                tx.commit().await?;
                return Ok(Progress::Done {
                    state: "DECLINED".into(),
                });
            }
            Outcome::Allow => {}
        }
        let policy = Gates {
            payments: self,
            intent: &intent,
            quote: &q,
            recipient: &recipient,
        };
        match self
            .core
            .execute_internal_payment(&q.instruction, &policy)
            .await
        {
            Ok(batch) => {
                ensure!(
                    !lose_core_ack,
                    "Simulated acknowledgement loss after Core commit"
                );
                self.transition(
                    &mut tx,
                    &intent.actor,
                    id,
                    "SETTLED",
                    "CORE_BOOKED",
                    Some(batch),
                )
                .await?;
                tx.commit().await?;
                Ok(Progress::Done {
                    state: "SETTLED".into(),
                })
            }
            Err(error) => {
                if !business_guard(&error) {
                    return Err(error);
                }
                self.transition(
                    &mut tx,
                    &intent.actor,
                    id,
                    "DECLINED",
                    "CURRENT_EXECUTION_GUARD",
                    None,
                )
                .await?;
                tx.commit().await?;
                Ok(Progress::Done {
                    state: "DECLINED".into(),
                })
            }
        }
    }
    pub(crate) async fn transition(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &ActorContext,
        id: Uuid,
        state: &str,
        reason: &str,
        batch: Option<Uuid>,
    ) -> Result<()> {
        let updated=sqlx::query("UPDATE payments.intents SET state=$2,reason_code=$3,batch_id=$4,version=version+1,updated_at=now() WHERE id=$1 AND (state NOT IN ('SETTLED','DECLINED','BLOCKED','EXPIRED','CANCELLED') OR state=$2 OR ($2='SETTLED' AND $4 IS NOT NULL)) AND (state<>$2 OR reason_code<>$3 OR batch_id IS DISTINCT FROM $4)").bind(id).bind(state).bind(reason).bind(batch).execute(&mut **tx).await?;
        if updated.rows_affected() == 1 {
            self.fact(tx, actor, id, "payment.state-recorded").await?;
        }
        Ok(())
    }
    pub async fn own_exception(&self, id: Uuid) -> Result<()> {
        let intent = self.intent(id).await?;
        Cases::new(self.controls.pool.clone())
            .ensure(
                &intent.actor,
                NewCase {
                    kind: CaseKind::Payment,
                    origin_id: id,
                    resource_id: id,
                    evidence_refs: vec![],
                    customer: Some((intent.actor.issuer.clone(), intent.actor.subject.clone())),
                },
            )
            .await?;
        Ok(())
    }
    pub async fn recover(&self) -> Result<()> {
        self.relay_audit().await?;
        self.recover_credits().await?;
        self.recover_requests().await?;
        self.reconcile_positions().await?;
        let mut tx = self.pool.begin().await?;
        let drafts=sqlx::query("UPDATE payments.intents SET state='EXPIRED',reason_code='QUOTE_EXPIRED',version=version+1,updated_at=now() WHERE id IN(SELECT i.id FROM payments.intents i JOIN payments.quotes q ON q.id=i.quote_id WHERE i.state='DRAFT' AND q.expires_at<=now() ORDER BY i.created_at LIMIT 50 FOR UPDATE OF i SKIP LOCKED) RETURNING id,actor_context").fetch_all(&mut *tx).await?;
        for row in drafts {
            let actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
            self.fact(&mut tx, &actor, row.get("id"), "payment.expired")
                .await?;
        }
        tx.commit().await?;
        let rows = self
            .recovery_page(crate::recovery::RecoveryPage::Intents)
            .await?;
        for row in rows {
            let id: Uuid = row.get("id");
            let intent = self.intent(id).await?;
            let op = if let Some(op) = row.get::<Option<Uuid>, _>("operation_id") {
                op
            } else {
                self.schedule(id).await?.id
            };
            if self.operations.status(&intent.actor, op).await?.state != "REVIEW_REQUIRED" {
                continue;
            }
            self.own_exception(id).await?;
            let q = self.quote_record(intent.quote_id).await?;
            // A submitted instruction retains its hold and is owned by operations on exhausted retries.
            if q.instruction.details.rail == "SIMULATED"
                && self
                    .core
                    .external_reservation(&q.instruction)
                    .await?
                    .is_some_and(|r| r.submitted)
            {
                let final_version:Option<i64>=sqlx::query_scalar("SELECT max(provider_version) FROM payments.rail_receipts WHERE payment_id=$1 AND document->'body'->>'status' IN ('SETTLED','REJECTED','CANCELLED')").bind(id).fetch_one(&self.pool).await?;
                if let Some(version) = final_version {
                    self.operations
                        .resume_review(
                            &intent.actor,
                            op,
                            JobSpec {
                                kind: "payment.execute".into(),
                                step_key: format!("finality-{version}"),
                                payload: serde_json::json!({"paymentId":id}),
                                delay_seconds: 0,
                            },
                        )
                        .await?;
                }
                continue;
            }
            let rail_revision = if q.instruction.details.rail == "SIMULATED" {
                let snapshot = self.rail_policy().await?;
                if snapshot.paused && q.expires_at > Utc::now() {
                    continue;
                }
                snapshot.revision
            } else {
                0
            };
            let gate = self
                .checks
                .evaluate(id, Purpose::Payment, &fingerprint(&q.instruction)?)
                .await?;
            if q.expires_at <= Utc::now() || matches!(gate.outcome, Outcome::Allow | Outcome::Deny)
            {
                self.operations
                    .resume_review(
                        &intent.actor,
                        op,
                        JobSpec {
                            kind: "payment.execute".into(),
                            step_key: format!(
                                "recheck-{}-{}-{}",
                                gate.revision,
                                rail_revision,
                                q.expires_at <= Utc::now()
                            ),
                            payload: serde_json::json!({"paymentId":id}),
                            delay_seconds: 0,
                        },
                    )
                    .await?;
            }
        }
        Ok(())
    }
}
