//! Essential R1 activation and due-period jobs. Core receipts, never job states, prove completion.
use crate::Finance;
use anyhow::{Result, ensure};
use bank_controls::{
    cases::{CaseKind, Cases, NewCase},
    governance::ChangeKind,
};
use bank_core::accruals::execution::{ActivationDetails, ActivationReceipt, PeriodReceipt};
use bank_platform::{
    domain::{ActorContext, fingerprint},
    operations::JobSpec,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationInput {
    pub consent_id: Uuid,
    pub counter_account_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationRequest {
    pub id: Uuid,
    pub maker_person: Uuid,
    pub checker_person: Option<Uuid>,
    pub details: ActivationDetails,
    pub enrollment: serde_json::Value,
    pub digest: String,
    pub state: String,
    pub receipt: Option<ActivationReceipt>,
    pub expires_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeriodInput {
    pub activation_id: Uuid,
    pub period_start: NaiveDate,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodRun {
    pub id: Uuid,
    pub activation_id: Uuid,
    pub period_start: NaiveDate,
    pub operation_id: Option<Uuid>,
    pub last_error: String,
    pub receipt: Option<PeriodReceipt>,
}
impl Finance {
    pub async fn propose_periodic_activation(
        &self,
        a: &ActorContext,
        key: Uuid,
        i: &ActivationInput,
    ) -> Result<ActivationRequest> {
        self.authorize_staff(a, true).await?;
        ensure!(
            !key.is_nil() && !i.consent_id.is_nil() && !i.counter_account_id.is_nil(),
            "Valid activation scope required"
        );
        let person = self.controls.person(a).await?.0;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,205))")
            .bind(format!(
                "{}/{}/{}/{}",
                a.issuer, a.subject, a.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(a, true).await?;
        if let Some(r)=sqlx::query("SELECT id,details FROM finance.periodic_activation_requests WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&a.issuer).bind(&a.subject).bind(&a.client_id).bind(key).fetch_optional(&mut *tx).await? {let d:ActivationDetails=serde_json::from_value(r.get("details"))?;ensure!(d.consent_id==i.consent_id && d.counter_account_id==i.counter_account_id,"Activation key conflict");let id=r.get("id");tx.commit().await?;return self.periodic_activation(a,id).await;}
        let source = self.periodic_commands.source(i.consent_id).await?;
        ensure!(
            source["status"] == "PENDING_ACTIVATION",
            "Unactivated consent required"
        );
        let id = Uuid::new_v4();
        let d = ActivationDetails {
            id,
            consent_id: i.consent_id,
            consent_digest: source["consentDigest"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Consent proof required"))?
                .into(),
            counter_account_id: i.counter_account_id,
        };
        sqlx::query("INSERT INTO finance.periodic_activation_requests(id,issuer,subject,client_id,request_key,actor_context,maker_person,details,digest,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,clock_timestamp()+interval '24 hours')").bind(id).bind(&a.issuer).bind(&a.subject).bind(&a.client_id).bind(key).bind(serde_json::to_value(a)?).bind(person).bind(serde_json::to_value(&d)?).bind(fingerprint(&d)?).execute(&mut *tx).await?;
        tx.commit().await?;
        self.periodic_activation(a, id).await
    }
    pub async fn periodic_activation(
        &self,
        a: &ActorContext,
        id: Uuid,
    ) -> Result<ActivationRequest> {
        self.authorize_staff(a, false).await?;
        let r = sqlx::query("SELECT * FROM finance.periodic_activation_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let d: ActivationDetails = serde_json::from_value(r.get("details"))?;
        let digest: String = r.get("digest");
        ensure!(
            d.id == id && fingerprint(&d)? == digest,
            "Activation draft integrity conflict"
        );
        let receipt = match self.periodic_commands.activation(id).await {
            Ok(r) => Some(r),
            Err(e) if matches!(e.downcast_ref::<sqlx::Error>(),Some(sqlx::Error::Database(db)) if db.code().as_deref()==Some("P0002")) => {
                None
            }
            Err(e) => return Err(e),
        };
        let expires = r.get("expires_at");
        let c = self
            .controls
            .change_record(id, ChangeKind::PeriodicActivation)
            .await?;
        if let Some(c) = &c {
            ensure!(
                c.resource_id == d.consent_id
                    && c.digest == digest
                    && c.maker_person == r.get::<Uuid, _>("maker_person"),
                "Activation approval scope conflict"
            );
        }
        let checker_person = c.as_ref().and_then(|c| c.checker_person);
        let state = if receipt.as_ref().is_some_and(|r| r.stopped) {
            "PAUSED".into()
        } else if receipt.is_some() {
            "ACTIVE".into()
        } else if expires <= Utc::now() {
            "EXPIRED".into()
        } else {
            c.map(|c| c.state).unwrap_or("STAGED".into())
        };
        Ok(ActivationRequest {
            id,
            maker_person: r.get("maker_person"),
            checker_person,
            enrollment: self.periodic_commands.source(d.consent_id).await?,
            details: d,
            digest,
            state,
            receipt,
            expires_at: expires,
        })
    }
    pub async fn periodic_activations(&self, a: &ActorContext) -> Result<Vec<ActivationRequest>> {
        self.authorize_staff(a, false).await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.periodic_activation_requests ORDER BY created_at DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.periodic_activation(a, id).await?);
        }
        Ok(out)
    }
    pub async fn approve_periodic_activation(
        &self,
        a: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<ActivationRequest> {
        ensure!(
            self.controls.finance_approval_authorized(a).await?,
            "Current independent checker required"
        );
        let r = self.periodic_activation(a, id).await?;
        ensure!(r.digest == digest, "Exact activation digest required");
        let row=sqlx::query("SELECT actor_context,maker_person FROM finance.periodic_activation_requests WHERE id=$1").bind(id).fetch_one(&self.pool).await?;
        let maker: ActorContext = serde_json::from_value(row.get("actor_context"))?;
        self.authorize_staff(&maker, true).await?;
        ensure!(
            self.controls.person(&maker).await?.0 == row.get::<Uuid, _>("maker_person"),
            "Original maker required"
        );
        self.controls
            .propose_change_id(
                &maker,
                id,
                ChangeKind::PeriodicActivation,
                r.details.consent_id,
                &r.details,
                r.expires_at,
            )
            .await?;
        self.controls.approve_change(a, id).await?;
        self.periodic_activation(a, id).await
    }
    pub async fn execute_periodic_activation(
        &self,
        a: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<ActivationRequest> {
        self.authorize_staff(a, true).await?;
        let r = self.periodic_activation(a, id).await?;
        ensure!(r.digest == digest, "Exact activation digest required");
        self.periodic_commands
            .activate(&self.controls, a, &r.details)
            .await?;
        self.periodic_activation(a, id).await
    }
    pub async fn queue_periodic_run(
        &self,
        a: &ActorContext,
        key: Uuid,
        i: &PeriodInput,
    ) -> Result<PeriodRun> {
        self.authorize_staff(a, true).await?;
        ensure!(!key.is_nil(), "Request key required");
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,206))")
            .bind(format!(
                "{}/{}/{}/{}",
                a.issuer, a.subject, a.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        if let Some(r)=sqlx::query("SELECT id,activation_id,period_start FROM finance.periodic_runs WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&a.issuer).bind(&a.subject).bind(&a.client_id).bind(key).fetch_optional(&mut *tx).await?{ensure!(r.get::<Uuid,_>("activation_id")==i.activation_id && r.get::<NaiveDate,_>("period_start")==i.period_start,"Period key conflict");let id=r.get("id");tx.commit().await?;return self.periodic_run(a,id).await;}
        ensure!(
            self.periodic_commands.next_due(i.activation_id).await? == Some(i.period_start),
            "Only the next closed due period may be queued"
        );
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO finance.periodic_runs(id,issuer,subject,client_id,request_key,actor_context,activation_id,period_start) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(id).bind(&a.issuer).bind(&a.subject).bind(&a.client_id).bind(key).bind(serde_json::to_value(a)?).bind(i.activation_id).bind(i.period_start).execute(&mut *tx).await?;
        tx.commit().await?;
        self.schedule_periodic_run(id).await?;
        self.periodic_run(a, id).await
    }
    pub async fn schedule_periodic_run(&self, id: Uuid) -> Result<()> {
        let r = sqlx::query("SELECT * FROM finance.periodic_runs WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let a: ActorContext = serde_json::from_value(r.get("actor_context"))?;
        self.authorize_staff(&a, true).await?;
        let op = self
            .operations
            .accept(
                &a,
                "finance.periodic",
                &id.to_string(),
                &json!({"runId":id}),
                JobSpec {
                    kind: "finance.periodic".into(),
                    step_key: "post-period".into(),
                    payload: json!({"runId":id}),
                    delay_seconds: 0,
                },
            )
            .await?;
        sqlx::query(
            "UPDATE finance.periodic_runs SET operation_id=$2 WHERE id=$1 AND operation_id IS NULL",
        )
        .bind(id)
        .bind(op.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    pub async fn periodic_run(&self, a: &ActorContext, id: Uuid) -> Result<PeriodRun> {
        self.authorize_staff(a, false).await?;
        let r = sqlx::query("SELECT * FROM finance.periodic_runs WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let activation = r.get("activation_id");
        let start = r.get("period_start");
        Ok(PeriodRun {
            id,
            activation_id: activation,
            period_start: start,
            operation_id: r.get("operation_id"),
            last_error: r.get("last_error"),
            receipt: self.periodic_commands.period(activation, start).await?,
        })
    }
    pub async fn periodic_runs(&self, a: &ActorContext) -> Result<Vec<PeriodRun>> {
        self.authorize_staff(a, false).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM finance.periodic_runs ORDER BY created_at DESC,id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.periodic_run(a, id).await?);
        }
        Ok(out)
    }
    pub async fn process_periodic_run(&self, id: Uuid) -> Result<PeriodReceipt> {
        let r = sqlx::query("SELECT * FROM finance.periodic_runs WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let a: ActorContext = serde_json::from_value(r.get("actor_context"))?;
        match self
            .periodic_commands
            .post(
                &self.controls,
                &a,
                r.get("activation_id"),
                r.get("period_start"),
            )
            .await
        {
            Ok(receipt) => {
                sqlx::query("UPDATE finance.periodic_runs SET last_error='' WHERE id=$1")
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                Ok(receipt)
            }
            Err(e) => {
                sqlx::query("UPDATE finance.periodic_runs SET last_error='NATIVE_PERIOD_REQUIRES_REVIEW' WHERE id=$1").bind(id).execute(&self.pool).await?;
                self.periodic_issue(r.get("activation_id"), "NATIVE_PERIOD_REQUIRES_REVIEW")
                    .await?;
                Err(e)
            }
        }
    }
    async fn periodic_issue(&self, id: Uuid, code: &str) -> Result<()> {
        sqlx::query("INSERT INTO finance.periodic_issues(activation_id,reason) VALUES($1,$2) ON CONFLICT(activation_id) DO UPDATE SET reason=EXCLUDED.reason,updated_at=clock_timestamp()").bind(id).bind(code).execute(&self.pool).await?;
        let case = Cases::new(self.controls.pool.clone())
            .ensure(
                &crate::service_actor(),
                NewCase {
                    kind: CaseKind::Finance,
                    origin_id: id,
                    resource_id: id,
                    evidence_refs: vec![],
                    customer: None,
                },
            )
            .await?;
        sqlx::query("UPDATE finance.periodic_issues SET case_id=$2 WHERE activation_id=$1")
            .bind(id)
            .bind(case)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn recover_periodic_execution(&self) -> Result<()> {
        let after: Uuid =
            sqlx::query_scalar("SELECT last_activation FROM finance.periodic_run_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let active = self.periodic_commands.active_page(after).await?;
        sqlx::query("UPDATE finance.periodic_run_cursor SET last_activation=$1 WHERE id")
            .bind(active.last().map(|(id, _)| *id).unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        for (id, actor) in active {
            match self.periodic_commands.next_due(id).await {
                Ok(Some(start)) => {
                    let hash = fingerprint(&json!([id, start]))?;
                    let key = Uuid::parse_str(&hash[..32])?;
                    if self
                        .queue_periodic_run(
                            &actor,
                            key,
                            &PeriodInput {
                                activation_id: id,
                                period_start: start,
                            },
                        )
                        .await
                        .is_err()
                    {
                        self.periodic_issue(id, "PERIODIC_SCHEDULING_REQUIRES_REVIEW")
                            .await?;
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    self.periodic_issue(id, "PERIODIC_SOURCE_REQUIRES_REVIEW")
                        .await?;
                }
            }
        }
        let after: Uuid =
            sqlx::query_scalar("SELECT last_id FROM finance.periodic_run_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.periodic_runs WHERE id>$1 AND operation_id IS NULL ORDER BY id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        sqlx::query("UPDATE finance.periodic_run_cursor SET last_id=$1 WHERE id")
            .bind(ids.last().copied().unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        for id in ids {
            if self.schedule_periodic_run(id).await.is_err() {
                let a: Uuid = sqlx::query_scalar(
                    "SELECT activation_id FROM finance.periodic_runs WHERE id=$1",
                )
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
                self.periodic_issue(a, "PERIODIC_SCHEDULING_REQUIRES_REVIEW")
                    .await?;
            }
        }
        Ok(())
    }
}
