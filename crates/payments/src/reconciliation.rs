//! Reconciliation observes independent financial/workflow records and owns breaks.
//! It has no command that releases funds or edits journals.
use crate::Payments;
use anyhow::{Result, ensure};
use bank_controls::cases::{CaseKind, Cases, NewCase};
use bank_platform::domain::{ActorContext, TrustContext, fingerprint};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

fn actor() -> ActorContext {
    ActorContext {
        issuer: "urn:piggybank:payments".into(),
        subject: "reconciliation".into(),
        trust: TrustContext::Service,
        client_id: "bank-worker".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationBreak {
    pub id: Uuid,
    pub payment_id: Uuid,
    pub code: String,
    pub state: String,
    pub case_id: Option<Uuid>,
    pub observation: Value,
    pub version: i64,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub matched_at: Option<DateTime<Utc>>,
}
impl Payments {
    pub async fn staff_reconciliation(
        &self,
        user: &ActorContext,
    ) -> Result<Vec<ReconciliationBreak>> {
        ensure!(
            self.controls.payment_inquiry_authorized(user).await?,
            "Payment inquiry role and queue required"
        );
        let rows=sqlx::query("SELECT * FROM payments.reconciliation_breaks ORDER BY (state='OPEN') DESC,last_seen_at DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| ReconciliationBreak {
                id: r.get("id"),
                payment_id: r.get("payment_id"),
                code: r.get("code"),
                state: r.get("state"),
                case_id: r.get("case_id"),
                observation: r.get("observation"),
                version: r.get("version"),
                first_seen_at: r.get("first_seen_at"),
                last_seen_at: r.get("last_seen_at"),
                matched_at: r.get("matched_at"),
            })
            .collect())
    }
    pub async fn reconcile_positions(&self) -> Result<()> {
        // Claim one bounded page, then release the cursor before cross-module work.
        // A crash is repaired on the next complete sweep; observations are idempotent.
        let mut tx = self.pool.begin().await?;
        let claimed: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(154,1)")
            .fetch_one(&mut *tx)
            .await?;
        if !claimed {
            return Ok(());
        }
        let after: Uuid = sqlx::query_scalar(
            "SELECT last_id FROM payments.reconciliation_cursor WHERE id FOR UPDATE",
        )
        .fetch_one(&mut *tx)
        .await?;
        let page = self.core.external_positions(after).await?;
        let next = page.last().map(|p| p.payment_id).unwrap_or(Uuid::nil());
        sqlx::query("UPDATE payments.reconciliation_cursor SET last_id=$1 WHERE id")
            .bind(next)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        for position in page {
            self.inspect_position(&position).await?;
        }
        Ok(())
    }
    async fn inspect_position(
        &self,
        p: &bank_core::reconciliation::ExternalPosition,
    ) -> Result<()> {
        let row=sqlx::query("SELECT i.state,i.batch_id,i.actor_context,q.instruction,EXISTS(SELECT 1 FROM payments.rail_submissions w WHERE w.payment_id=i.id) AS work_exists,(SELECT min(e.recorded_at) FROM payments.rail_receipts e WHERE e.payment_id=i.id AND e.document->'body'->>'status' IN ('SETTLED','REJECTED','CANCELLED')) AS provider_final_at FROM payments.intents i JOIN payments.quotes q ON q.id=i.quote_id WHERE i.id=$1").bind(p.payment_id).fetch_optional(&self.pool).await?;
        let mut codes = Vec::<String>::new();
        let mut customer = None;
        let mut workflow = Value::Null;
        if let Some(r) = row {
            let state: String = r.get("state");
            let batch: Option<Uuid> = r.get("batch_id");
            let owner: ActorContext = serde_json::from_value(r.get("actor_context"))?;
            customer = Some((owner.issuer, owner.subject));
            let instruction: bank_core::payments::InternalInstruction =
                serde_json::from_value(r.get("instruction"))?;
            let work: bool = r.get("work_exists");
            let provider_final_at: Option<DateTime<Utc>> = r.get("provider_final_at");
            let provider_final = provider_final_at.is_some();
            workflow = json!({"state":state,"batchId":batch,"providerWorkExists":work,"providerFinalEvidence":provider_final,"providerFinalReceivedAt":provider_final_at});
            let digest_matches = fingerprint(&instruction)? == p.request_digest;
            if digest_matches {
                // Re-read immutable finality after the workflow observation. Core may
                // have committed between the independent module reads; retry that
                // position on the next sweep instead of inventing a financial break.
                let current = self.core.external_finality(&instruction).await?;
                if current.as_ref().map(|f| f.outcome.as_str()) != p.final_outcome.as_deref() {
                    return Ok(());
                }
            }
            if !digest_matches
                || instruction
                    .details
                    .amount
                    .checked_add(instruction.details.fee)?
                    != p.total
            {
                codes.push("RESERVATION_SCOPE_MISMATCH".into());
            }
            if p.submitted
                && !work
                && p.final_outcome.is_none()
                && Utc::now() - p.hold_updated_at >= Duration::seconds(30)
            {
                codes.push("PROVIDER_WORK_MISSING".into());
            }
            if let Some(outcome) = &p.final_outcome {
                if (&state != outcome || batch != p.batch_id)
                    && Utc::now() - p.hold_updated_at >= Duration::seconds(30)
                {
                    codes.push("FINALITY_WORKFLOW_GAP".into());
                }
            } else {
                if state == "SETTLED" || batch.is_some() {
                    codes.push("BOOKING_WITHOUT_CORE_FINALITY".into());
                }
                if p.submitted
                    && ["DECLINED", "CANCELLED", "EXPIRED", "BLOCKED"].contains(&state.as_str())
                {
                    codes.push("SUBMITTED_HOLD_WITHOUT_FINALITY".into());
                }
                if provider_final_at.is_some_and(|at| Utc::now() - at >= Duration::seconds(30)) {
                    codes.push("PROVIDER_FINALITY_UNBOOKED".into());
                }
            }
        } else {
            codes.push("CORE_RESERVATION_WITHOUT_INTENT".into());
        }
        if p.account_held != p.computed_held {
            codes.push("ACCOUNT_HOLD_TOTAL_MISMATCH".into());
        }
        if p.hold_state == "ACTIVE"
            && p.submitted
            && Utc::now() - p.held_since >= Duration::minutes(5)
        {
            codes.push("SUBMITTED_OUTCOME_OVERDUE".into());
        }
        match p.final_outcome.as_deref() {
            Some("SETTLED")
                if p.hold_state != "CONSUMED"
                    || p.daily_usage_state != "POSTED"
                    || p.batch_id.is_none() =>
            {
                codes.push("SETTLEMENT_FINANCIAL_FACT_MISMATCH".into())
            }
            Some("DECLINED" | "CANCELLED")
                if p.hold_state != "RELEASED"
                    || p.daily_usage_state != "RELEASED"
                    || p.batch_id.is_some() =>
            {
                codes.push("RELEASE_FINANCIAL_FACT_MISMATCH".into())
            }
            None if p.submitted && p.hold_state != "ACTIVE" => {
                codes.push("HOLD_CHANGED_WITHOUT_FINALITY".into())
            }
            _ => {}
        }
        let observation = json!({"core":p,"workflow":workflow});
        for code in &codes {
            self.record_break(p.payment_id, code, &observation, customer.clone())
                .await?;
        }
        let mut tx = self.pool.begin().await?;
        let matched=sqlx::query("UPDATE payments.reconciliation_breaks SET state='MATCHED',matched_at=now(),last_seen_at=now(),version=version+1,observation=$3,fingerprint=$4 WHERE payment_id=$1 AND state='OPEN' AND NOT(code=ANY($2)) RETURNING id")
            .bind(p.payment_id).bind(&codes).bind(&observation).bind(fingerprint(&observation)?).fetch_all(&mut *tx).await?;
        for row in matched {
            Self::audit_in(
                &mut tx,
                &actor(),
                "payment.reconciliation-matched",
                row.get("id"),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn record_break(
        &self,
        payment: Uuid,
        code: &str,
        observation: &Value,
        customer: Option<(String, String)>,
    ) -> Result<()> {
        let digest = fingerprint(observation)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,155))")
            .bind(format!("{payment}:{code}"))
            .execute(&mut *tx)
            .await?;
        let prior=sqlx::query("SELECT fingerprint,state FROM payments.reconciliation_breaks WHERE payment_id=$1 AND code=$2").bind(payment).bind(code).fetch_optional(&mut *tx).await?;
        let changed = prior.is_none_or(|r| {
            r.get::<String, _>("state") != "OPEN" || r.get::<String, _>("fingerprint") != digest
        });
        let row=sqlx::query("INSERT INTO payments.reconciliation_breaks(id,payment_id,code,state,observation,fingerprint) VALUES($1,$2,$3,'OPEN',$4,$5) ON CONFLICT(payment_id,code) DO UPDATE SET state='OPEN',last_seen_at=now(),matched_at=NULL,observation=EXCLUDED.observation,fingerprint=EXCLUDED.fingerprint,version=payments.reconciliation_breaks.version+$6 RETURNING id,case_id")
            .bind(Uuid::new_v4()).bind(payment).bind(code).bind(observation).bind(digest).bind(i64::from(changed)).fetch_one(&mut *tx).await?;
        let id: Uuid = row.get("id");
        if changed {
            Self::audit_in(&mut tx, &actor(), "payment.reconciliation-break", id).await?;
        }
        tx.commit().await?;
        if row.get::<Option<Uuid>, _>("case_id").is_none() {
            let case = Cases::new(self.controls.pool.clone())
                .ensure(
                    &actor(),
                    NewCase {
                        kind: CaseKind::Payment,
                        origin_id: id,
                        resource_id: payment,
                        evidence_refs: vec![],
                        customer,
                    },
                )
                .await?;
            sqlx::query("UPDATE payments.reconciliation_breaks SET case_id=$2 WHERE id=$1 AND case_id IS NULL").bind(id).bind(case).execute(&self.pool).await?;
        }
        Ok(())
    }
}
