//! Durable reservation and authenticated submission. Network calls occur after the claim commits.
use crate::{
    Payments,
    execution::{Gates, business_guard},
    types::Progress,
};
use anyhow::{Result, ensure};
use bank_controls::checks::{Outcome, Purpose};
use bank_core::payments::PaymentPolicy;
use bank_platform::domain::{AccountId, fingerprint};
use bank_rails::{RailInstruction, RailStatus, VerifiedEvidence};
use chrono::{Duration, Utc};
use sqlx::Row;
use uuid::Uuid;
impl Payments {
    pub(crate) async fn process_external(&self, id: Uuid) -> Result<Progress> {
        if let Some(done) = self.reconcile_external_with_ack_fault(id, false).await? {
            return Ok(done);
        }
        let mut intent = self.intent(id).await?;
        let q = self.quote_record(intent.quote_id).await?;
        let mut tx = self.pool.begin().await?;
        let claimed: bool =
            sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,130))")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        if !claimed {
            return Ok(Progress::Pending);
        }
        let current =
            sqlx::query("SELECT state,challenge_id FROM payments.intents WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        intent.state = current.get("state");
        intent.challenge = current.get("challenge_id");
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
        let prior = self.core.external_reservation(&q.instruction).await?;
        let already_submitted = prior.as_ref().is_some_and(|r| r.submitted);
        if already_submitted
            && prior.as_ref().is_some_and(|r| r.state == "ACTIVE")
            && ["SETTLED", "EXPIRED", "DECLINED", "BLOCKED", "CANCELLED"]
                .contains(&intent.state.as_str())
        {
            // Core still owns an uncertain submitted commitment. Repair only the
            // orchestration claim; no financial effect is authorized by this update.
            sqlx::query("UPDATE payments.intents SET state='OUTCOME_UNKNOWN',reason_code='SUBMITTED_OUTCOME_REQUIRES_EVIDENCE',batch_id=NULL,version=version+1,updated_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await?;
            self.fact(
                &mut tx,
                &intent.actor,
                id,
                "payment.uncertain-outcome-restored",
            )
            .await?;
            intent.state = "OUTCOME_UNKNOWN".into();
        }
        if ["EXPIRED", "DECLINED", "BLOCKED", "CANCELLED"].contains(&intent.state.as_str())
            && !already_submitted
        {
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
        let cancel_requested = Self::cancellation_requested_in(&mut tx, id).await?;
        ensure!(
            intent.state != "DRAFT",
            "Unconfirmed instruction cannot execute"
        );
        if !already_submitted {
            if q.expires_at <= Utc::now()
                || prior
                    .as_ref()
                    .is_some_and(|r| r.authority_until <= Utc::now())
            {
                if prior.is_some() {
                    self.core
                        .release_external_unsubmitted(&q.instruction)
                        .await?;
                }
                self.transition(&mut tx, &intent.actor, id, "EXPIRED", "QUOTE_EXPIRED", None)
                    .await?;
                tx.commit().await?;
                return Ok(Progress::Done {
                    state: "EXPIRED".into(),
                });
            }
            let snapshot = Self::rail_policy_in(&mut tx, true).await?;
            if snapshot.paused {
                self.transition(
                    &mut tx,
                    &intent.actor,
                    id,
                    "RAIL_PAUSED",
                    "NEW_SUBMISSIONS_PAUSED",
                    None,
                )
                .await?;
                tx.commit().await?;
                return Ok(Progress::Pending);
            }
            // WP-40: a Finance liquidity breach restricts new external submissions. Nothing is reserved or sent; the
            // payment resumes after an independently approved lift.
            let restricted: bool =
                sqlx::query_scalar("SELECT finance.liquidity_restricted('EXTERNAL_SUBMISSION')")
                    .fetch_one(&mut *tx)
                    .await?;
            if restricted {
                self.transition(
                    &mut tx,
                    &intent.actor,
                    id,
                    "RAIL_PAUSED",
                    "LIQUIDITY_RESTRICTED",
                    None,
                )
                .await?;
                tx.commit().await?;
                return Ok(Progress::Pending);
            }
            let row=sqlx::query("SELECT * FROM payments.beneficiaries WHERE id=$1 AND issuer=$2 AND subject=$3 FOR SHARE").bind(q.beneficiary_id).bind(&intent.actor.issuer).bind(&intent.actor.subject).fetch_one(&mut *tx).await?;
            let recipient = crate::recipients::record(&row);
            let validation = async {
                Self::validate_rail_quote(&q, &snapshot)?;
                self.current_bound_quote(
                    &intent.actor,
                    &q,
                    &recipient,
                    AccountId(row.get("payee_account_id")),
                )
                .await?;
                let digest = fingerprint(&q.instruction)?;
                for kind in Purpose::Payment.required() {
                    self.checks
                        .request(&intent.actor, id, *kind, &digest)
                        .await?;
                }
                self.checks.evaluate(id, Purpose::Payment, &digest).await
            }
            .await;
            let gate = match validation {
                Ok(g) => g,
                Err(e) if business_guard(&e) => {
                    if prior.is_some() {
                        self.core
                            .release_external_unsubmitted(&q.instruction)
                            .await?;
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
                Err(e) => return Err(e),
            };
            let (state, reason) = match gate.outcome {
                Outcome::Pending => ("CONTROLS_PENDING", "CURRENT_CONTROLS_PENDING"),
                Outcome::Review => ("CONTROL_REVIEW", "CONTROL_REVIEW_REQUIRED"),
                Outcome::Deny => ("DECLINED", "CURRENT_CONTROL_DENIAL"),
                Outcome::Allow => ("RESERVED", "CORE_RESERVED"),
            };
            if gate.outcome != Outcome::Allow {
                if gate.outcome == Outcome::Deny && prior.is_some() {
                    self.core
                        .release_external_unsubmitted(&q.instruction)
                        .await?;
                }
                self.transition(&mut tx, &intent.actor, id, state, reason, None)
                    .await?;
                tx.commit().await?;
                return Ok(match gate.outcome {
                    Outcome::Deny => Progress::Done {
                        state: state.into(),
                    },
                    Outcome::Review => Progress::Review {
                        code: reason.into(),
                    },
                    _ => Progress::Pending,
                });
            }
            let gates = Gates {
                payments: self,
                intent: &intent,
                quote: &q,
                recipient: &recipient,
            };
            let reserve = async {
                let permit=gates.validate(&q.instruction).await?;
                let r=self.core.reserve_external_payment(&q.instruction,&gates).await?;
                ensure!(r.state=="ACTIVE", "Active reservation required");
                // WP-39: the active routing policy chooses the provider; Core records the route (idempotently).
                let decision:serde_json::Value=sqlx::query_scalar("SELECT payments.route_decide($1,1,'{}')").bind(id).fetch_one(&mut *tx).await?;
                let chosen=decision["provider"].as_str().unwrap_or(bank_rails::PROVIDER).to_owned();
                let (provider,attempt)=self.core.route_external(&q.instruction,&chosen,None).await?;
                let instruction=RailInstruction{instruction_id:id,payment_id:id,beneficiary_reference:recipient.receiving_reference.clone(),amount:q.instruction.details.amount,requested_date:q.instruction.details.requested_date,input_digest:fingerprint(&q.instruction)?,rail:"SIMULATED".into(),authority_until:r.authority_until};
                sqlx::query("INSERT INTO payments.rail_submissions(payment_id,instruction_id,document,fingerprint,provider,route_attempt) VALUES($1,$1,$2,$3,$4,$5) ON CONFLICT(payment_id) DO NOTHING").bind(id).bind(serde_json::to_value(&instruction)?).bind(fingerprint(&instruction)?).bind(&provider).bind(attempt).execute(&mut *tx).await?;
                self.core.mark_external_submitted(&q.instruction,permit.valid_until.min(r.authority_until)).await?;
                Ok::<_,anyhow::Error>(())
            }.await;
            if let Err(e) = reserve {
                // Core may have committed before acknowledgement loss. Submitted holds stay owned and recoverable.
                if self
                    .core
                    .external_reservation(&q.instruction)
                    .await?
                    .is_some_and(|r| r.submitted)
                {
                    return Err(e);
                }
                if !business_guard(&e) {
                    return Err(e);
                }
                if self
                    .core
                    .external_reservation(&q.instruction)
                    .await?
                    .is_some()
                {
                    self.core
                        .release_external_unsubmitted(&q.instruction)
                        .await?;
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
                return Ok(Progress::Done {
                    state: "DECLINED".into(),
                });
            }
        } else {
            // Reconstruct durable work if the process died after Core marked the hold submitted.
            let recipient =
                sqlx::query("SELECT receiving_reference FROM payments.beneficiaries WHERE id=$1")
                    .bind(q.beneficiary_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let r = prior.as_ref().expect("submitted reservation");
            let instruction = RailInstruction {
                instruction_id: id,
                payment_id: id,
                beneficiary_reference: recipient.get("receiving_reference"),
                amount: q.instruction.details.amount,
                requested_date: q.instruction.details.requested_date,
                input_digest: fingerprint(&q.instruction)?,
                rail: "SIMULATED".into(),
                authority_until: r.authority_until,
            };
            // Core's route is authoritative (pre-WP-39 submissions resolve to the original provider).
            let (provider, attempt) = self
                .core
                .route_external(&q.instruction, bank_rails::PROVIDER, None)
                .await?;
            sqlx::query("INSERT INTO payments.rail_submissions(payment_id,instruction_id,document,fingerprint,provider,route_attempt) VALUES($1,$1,$2,$3,$4,$5) ON CONFLICT(payment_id) DO NOTHING").bind(id).bind(serde_json::to_value(&instruction)?).bind(fingerprint(&instruction)?).bind(&provider).bind(attempt).execute(&mut *tx).await?;
        }
        let work =
            sqlx::query("SELECT * FROM payments.rail_submissions WHERE payment_id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if work
            .get::<Option<chrono::DateTime<Utc>>, _>("lease_until")
            .is_some_and(|t| t > Utc::now())
        {
            return Ok(Progress::Pending);
        }
        let instruction: RailInstruction = serde_json::from_value(work.get("document"))?;
        ensure!(
            fingerprint(&instruction)? == work.get::<String, _>("fingerprint")
                && instruction.input_digest == fingerprint(&q.instruction)?,
            "Provider work integrity conflict"
        );
        let provider: String = work.get("provider");
        let route_attempt: i32 = work.get("route_attempt");
        // A re-routed attempt has never been sent to its new provider until attempted_at is set.
        let inquire = (already_submitted && route_attempt == 1)
            || work
                .get::<Option<chrono::DateTime<Utc>>, _>("attempted_at")
                .is_some();
        let lease = Uuid::new_v4();
        let fence = work.get::<i64, _>("lease_fence") + 1;
        sqlx::query("UPDATE payments.rail_submissions SET attempted_at=coalesce(attempted_at,now()),attempts=attempts+$2,inquiries=inquiries+$3,lease_id=$4,lease_until=$5,lease_fence=$6,cancellations=cancellations+$7 WHERE payment_id=$1").bind(id).bind(i32::from(!inquire && !cancel_requested)).bind(i32::from(inquire && !cancel_requested)).bind(lease).bind(Utc::now()+Duration::seconds(10)).bind(fence).bind(i32::from(cancel_requested)).execute(&mut *tx).await?;
        self.transition(
            &mut tx,
            &intent.actor,
            id,
            if cancel_requested {
                "CANCELLATION_PENDING"
            } else if inquire {
                "OUTCOME_UNKNOWN"
            } else {
                "SUBMITTING"
            },
            "AUTHENTICATED_OUTCOME_PENDING",
            None,
        )
        .await?;
        tx.commit().await?;
        let client = self.rail(&provider)?;
        let result = if cancel_requested {
            client.cancel(&instruction).await
        } else if inquire {
            client.inquire(&instruction).await
        } else {
            client.submit(&instruction).await
        };
        let mut tx = self.pool.begin().await?;
        // Match claim lock order and recheck fencing after waiting for locks.
        sqlx::query("SELECT id FROM payments.intents WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let active=sqlx::query("SELECT payment_id FROM payments.rail_submissions WHERE payment_id=$1 AND lease_id=$2 AND lease_fence=$3 AND lease_until>clock_timestamp() FOR UPDATE").bind(id).bind(lease).bind(fence).fetch_optional(&mut *tx).await?;
        if active.is_none() {
            return Ok(Progress::Pending);
        }
        let mut conflict = false;
        let state = match &result {
            Ok(verified) => {
                let state = self.record_rail_evidence(&mut tx, id, verified).await?;
                if state == "CONFLICT" {
                    conflict = true;
                    "OUTCOME_UNKNOWN"
                } else {
                    state
                }
            }
            Err(_) => "OUTCOME_UNKNOWN",
        };
        let state = if cancel_requested && state != "RECONCILING" {
            "CANCELLATION_PENDING"
        } else {
            state
        };
        sqlx::query("UPDATE payments.rail_submissions SET lease_id=NULL,lease_until=NULL,last_status=$2 WHERE payment_id=$1 AND lease_id=$3 AND lease_fence=$4").bind(id).bind(state).bind(lease).bind(fence).execute(&mut *tx).await?;
        self.transition(
            &mut tx,
            &intent.actor,
            id,
            state,
            if state == "CANCELLATION_PENDING" {
                "CANCELLATION_OUTCOME_PENDING"
            } else if state == "SUBMITTED" {
                "PROVIDER_ACKNOWLEDGED"
            } else if state == "RECONCILING" {
                "FINAL_EVIDENCE_REQUIRES_CORE"
            } else if state == "SUBMITTING" {
                "PROVIDER_NONEXECUTION_REROUTED"
            } else {
                "AUTHENTICATED_OUTCOME_PENDING"
            },
            None,
        )
        .await?;
        tx.commit().await?;
        if conflict {
            if let Ok(v) = &result {
                self.quarantine_rail(Some(id), &v.document, "IMMUTABLE_PROVIDER_CONFLICT")
                    .await?;
            }
            self.own_exception(id).await?;
        }
        Ok(Progress::Pending)
    }
    pub(crate) async fn record_rail_evidence(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        v: &VerifiedEvidence,
    ) -> Result<&'static str> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,152))")
            .bind(v.evidence.evidence_id.to_string())
            .execute(&mut **tx)
            .await?;
        let used: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM payments.credit_intents WHERE evidence_id=$1)",
        )
        .bind(v.evidence.evidence_id)
        .fetch_one(&mut **tx)
        .await?;
        if used {
            return Ok("CONFLICT");
        }
        let work = sqlx::query(
            "SELECT provider,route_attempt FROM payments.rail_submissions WHERE payment_id=$1",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
        let current: String = work.get("provider");
        let attempt: i32 = work.get("route_attempt");
        if v.evidence.provider != current {
            // A provider this payment was re-routed away from. Its proven non-execution may repeat; any other
            // claim contradicts that proof and is quarantined (Core books only the current route's provider).
            let known: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM payments.rail_receipts WHERE evidence_id=$1)",
            )
            .bind(v.evidence.evidence_id)
            .fetch_one(&mut **tx)
            .await?;
            return Ok(if known || v.evidence.status == RailStatus::Rejected {
                "STALE"
            } else {
                "CONFLICT"
            });
        }
        if v.evidence.status == RailStatus::NotFound {
            return Ok("OUTCOME_UNKNOWN");
        }
        let previous:Option<serde_json::Value>=sqlx::query_scalar("SELECT document FROM payments.rail_receipts WHERE payment_id=$1 AND route_attempt=$2 ORDER BY provider_version DESC LIMIT 1").bind(id).bind(attempt).fetch_optional(&mut **tx).await?;
        if let Some(previous) = previous {
            let packet: bank_platform::authenticity::SignedDocument =
                serde_json::from_value(previous)?;
            let old: bank_rails::RailEvidence = serde_json::from_value(packet.body)?;
            if matches!(
                old.status,
                RailStatus::Settled | RailStatus::Rejected | RailStatus::Cancelled
            ) && v.evidence.status != old.status
                && (v.evidence.version >= old.version
                    || matches!(
                        v.evidence.status,
                        RailStatus::Settled | RailStatus::Rejected | RailStatus::Cancelled
                    ))
            {
                return Ok("CONFLICT");
            }
        }
        sqlx::query("INSERT INTO payments.rail_receipts(evidence_id,payment_id,provider_version,digest,document,route_attempt,provider) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING").bind(v.evidence.evidence_id).bind(id).bind(v.evidence.version).bind(&v.digest).bind(serde_json::to_value(&v.document)?).bind(attempt).bind(&current).execute(&mut **tx).await?;
        let rows=sqlx::query("SELECT * FROM payments.rail_receipts WHERE evidence_id=$1 OR (payment_id=$2 AND route_attempt=$4 AND provider_version=$3)").bind(v.evidence.evidence_id).bind(id).bind(v.evidence.version).bind(attempt).fetch_all(&mut **tx).await?;
        if rows.len() != 1
            || rows.iter().any(|r| {
                r.get::<Uuid, _>("evidence_id") != v.evidence.evidence_id
                    || r.get::<Uuid, _>("payment_id") != id
                    || r.get::<i64, _>("provider_version") != v.evidence.version
                    || r.get::<String, _>("digest") != v.digest
            })
        {
            sqlx::query("INSERT INTO payments.rail_rejections VALUES($1,$2,$3,'PROVIDER_VERSION_CONFLICT',now()) ON CONFLICT DO NOTHING").bind(Uuid::new_v4()).bind(id).bind(&v.digest).execute(&mut **tx).await?;
            return Ok("CONFLICT");
        }
        let latest:serde_json::Value=sqlx::query_scalar("SELECT document FROM payments.rail_receipts WHERE payment_id=$1 AND route_attempt=$2 ORDER BY provider_version DESC LIMIT 1").bind(id).bind(attempt).fetch_one(&mut **tx).await?;
        let packet: bank_platform::authenticity::SignedDocument = serde_json::from_value(latest)?;
        let e: bank_rails::RailEvidence = serde_json::from_value(packet.body.clone())?;
        if e.status == RailStatus::Rejected && self.reroute(tx, id, attempt, &e, &packet).await? {
            return Ok("SUBMITTING");
        }
        Ok(
            if matches!(e.status, RailStatus::Accepted | RailStatus::Pending) {
                "SUBMITTED"
            } else {
                "RECONCILING"
            },
        )
    }
}
impl Payments {
    /// WP-39 safe fallback: only after the current provider's signed non-execution, only when the payment's routing
    /// policy allows it, and only to a provider not yet tried. Core verifies the proof and records the new route before
    /// the work row moves; the same instruction (same economic identity and hold) is then submitted to the next provider.
    async fn reroute(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        attempt: i32,
        evidence: &bank_rails::RailEvidence,
        packet: &bank_platform::authenticity::SignedDocument,
    ) -> Result<bool> {
        let allowed: bool =
            sqlx::query_scalar("SELECT coalesce(payments.route_fallback_allowed($1,$2,$3),false)")
                .bind(id)
                .bind(&evidence.reason_code)
                .bind(attempt)
                .fetch_one(&mut **tx)
                .await?;
        if !allowed {
            return Ok(false);
        }
        let tried: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT provider FROM payments.rail_receipts WHERE payment_id=$1 UNION SELECT provider FROM payments.rail_submissions WHERE payment_id=$1",
        )
        .bind(id)
        .fetch_all(&mut **tx)
        .await?;
        let decision: serde_json::Value =
            sqlx::query_scalar("SELECT payments.route_decide($1,$2,$3)")
                .bind(id)
                .bind(attempt + 1)
                .bind(&tried)
                .fetch_one(&mut **tx)
                .await?;
        let Some(next) = decision["provider"].as_str() else {
            return Ok(false);
        };
        let intent = self.intent(id).await?;
        let q = self.quote_record(intent.quote_id).await?;
        let (provider, routed) = match self
            .core
            .route_external(&q.instruction, next, Some(packet))
            .await
        {
            Ok(route) => route,
            Err(_) => return Ok(false),
        };
        if routed != attempt + 1 {
            return Ok(false);
        }
        sqlx::query("UPDATE payments.rail_submissions SET provider=$2,route_attempt=$3,attempted_at=NULL,lease_id=NULL,lease_until=NULL,last_status='REROUTED' WHERE payment_id=$1 AND route_attempt=$4")
            .bind(id)
            .bind(&provider)
            .bind(routed)
            .bind(attempt)
            .execute(&mut **tx)
            .await?;
        Ok(true)
    }
}
