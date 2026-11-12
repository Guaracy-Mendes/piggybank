use crate::{
    Payments,
    types::{Intent, PaymentStatus},
};
use anyhow::{Result, ensure};
use bank_controls::entitlements::{Action, ApprovalRef};
use bank_platform::{
    database::Module,
    domain::{ActorContext, EventEnvelope, EventId, TrustContext},
    events::EventStore,
    operations::{JobSpec, OperationStatus},
};
use chrono::Utc;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
impl Payments {
    pub async fn create_intent(
        &self,
        actor: &ActorContext,
        key: Uuid,
        quote_id: Uuid,
    ) -> Result<PaymentStatus> {
        let q = self.owned_quote(actor, quote_id).await?;
        self.authorize(
            actor,
            q.instruction.details.payer_account_id,
            Action::ReadAccount,
        )
        .await?;
        let prior=sqlx::query("SELECT id,quote_id FROM payments.intents WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&self.pool).await?;
        if let Some(prior) = prior {
            ensure!(
                prior.get::<Uuid, _>("quote_id") == quote_id,
                "Payment request identity conflict"
            );
            return self.created_intent_status(actor, prior.get("id")).await;
        }
        self.current_quote(actor, &q).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,120))")
            .bind(format!("{}/{}/{}", actor.issuer, actor.subject, key))
            .execute(&mut *tx)
            .await?;
        if let Some(row)=sqlx::query("SELECT id,quote_id FROM payments.intents WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await? {
   ensure!(row.get::<Uuid,_>("quote_id")==quote_id,"Payment request identity conflict");let id=row.get("id");tx.commit().await?;return self.created_intent_status(actor,id).await;
  }
        let id = q.instruction.operation_id;
        let created=sqlx::query("INSERT INTO payments.intents(id,issuer,subject,client_id,request_key,quote_id,actor_context,state) VALUES($1,$2,$3,$4,$5,$6,$7,'DRAFT') ON CONFLICT(quote_id) DO NOTHING RETURNING id").bind(id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(quote_id).bind(serde_json::to_value(actor)?).fetch_optional(&mut *tx).await?;
        if created.is_some() {
            self.fact(&mut tx, actor, id, "payment.drafted").await?;
        }
        // One immutable quote has one economic intent, even across concurrent request keys.
        let saved = sqlx::query(
            "SELECT id,issuer,subject,client_id,request_key FROM payments.intents WHERE quote_id=$1",
        )
        .bind(quote_id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            saved.get::<String, _>("issuer") == actor.issuer
                && saved.get::<String, _>("subject") == actor.subject
                && saved.get::<String, _>("client_id") == actor.client_id
                && saved.get::<Uuid, _>("request_key") == key,
            "Quote already bound to another channel intent"
        );
        tx.commit().await?;
        self.challenge(actor, id).await
    }
    // The intent commits before its independently owned Controls challenge is linked.
    // A same-key retry can observe that boundary before the original creator resumes.
    async fn created_intent_status(&self, actor: &ActorContext, id: Uuid) -> Result<PaymentStatus> {
        let status = self.detail(actor, id).await?;
        if status.state == "DRAFT"
            && status.challenge_id.is_none()
            && status.expires_at > Utc::now()
        {
            self.challenge(actor, id).await
        } else {
            Ok(status)
        }
    }
    pub async fn cancel_draft(&self, actor: &ActorContext, id: Uuid) -> Result<PaymentStatus> {
        let intent = self.owned_intent(actor, id).await?;
        let q = self.quote_record(intent.quote_id).await?;
        let mut tx = self.pool.begin().await?;
        let state: String =
            sqlx::query_scalar("SELECT state FROM payments.intents WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if state == "CANCELLED" {
            tx.commit().await?;
            return self.detail(actor, id).await;
        }
        ensure!(
            state == "DRAFT"
                && self.core.payment_receipt(&q.instruction).await?.is_none()
                && self
                    .core
                    .external_reservation(&q.instruction)
                    .await?
                    .is_none(),
            "Only an unconfirmed unreserved draft can be cancelled"
        );
        self.transition(
            &mut tx,
            actor,
            id,
            "CANCELLED",
            "USER_CANCELLED_UNCONFIRMED",
            None,
        )
        .await?;
        tx.commit().await?;
        self.detail(actor, id).await
    }
    pub(crate) async fn intent(&self, id: Uuid) -> Result<Intent> {
        let r = sqlx::query(
            "SELECT id,actor_context,quote_id,challenge_id,state FROM payments.intents WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(Intent {
            id: r.get("id"),
            actor: serde_json::from_value(r.get("actor_context"))?,
            quote_id: r.get("quote_id"),
            challenge: r.get("challenge_id"),
            state: r.get("state"),
        })
    }
    pub(crate) async fn owned_intent(&self, actor: &ActorContext, id: Uuid) -> Result<Intent> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        let intent = self.intent(id).await?;
        ensure!(
            intent.actor.issuer == actor.issuer && intent.actor.subject == actor.subject,
            crate::ResourceUnavailable
        );
        let q = self.quote_record(intent.quote_id).await?;
        self.authorize(
            actor,
            q.instruction.details.payer_account_id,
            Action::ReadAccount,
        )
        .await?;
        Ok(intent)
    }
    pub async fn challenge(&self, actor: &ActorContext, id: Uuid) -> Result<PaymentStatus> {
        let intent = self.owned_intent(actor, id).await?;
        ensure!(intent.state == "DRAFT", "Only a draft can start approval");
        let q = self.owned_quote(actor, intent.quote_id).await?;
        self.current_quote(actor, &q).await?;
        let resource = self
            .core
            .authority(
                q.instruction.details.payer_account_id,
                Action::CreatePayment,
            )
            .await?;
        let challenge = self
            .controls
            .challenge(actor, id, &q.instruction.details, &resource, id)
            .await?;
        sqlx::query("UPDATE payments.intents SET challenge_id=$2 WHERE id=$1 AND state='DRAFT' AND (challenge_id IS NULL OR challenge_id=$2)").bind(id).bind(challenge.id).execute(&self.pool).await?;
        self.detail(actor, id).await
    }
    pub async fn confirm(&self, actor: &ActorContext, id: Uuid) -> Result<PaymentStatus> {
        let intent = self.owned_intent(actor, id).await?;
        if intent.state != "DRAFT" {
            return self.detail(actor, id).await;
        }
        let q = self.owned_quote(actor, intent.quote_id).await?;
        self.current_quote(actor, &q).await?;
        let challenge = intent
            .challenge
            .ok_or_else(|| anyhow::anyhow!("Action approval required"))?;
        let resource = self
            .core
            .authority(
                q.instruction.details.payer_account_id,
                Action::CreatePayment,
            )
            .await?;
        self.controls
            .approve_payment(
                &self.identity,
                actor,
                ApprovalRef {
                    challenge_id: challenge,
                    operation_id: id,
                },
                &q.instruction.details,
                &resource,
                id,
            )
            .await?;
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE payments.intents SET state='ACCEPTED',reason_code='VALIDATION_PENDING',confirmed_at=now(),version=version+1,updated_at=now() WHERE id=$1 AND state='DRAFT'").bind(id).execute(&mut *tx).await?;
        if changed.rows_affected() == 1 {
            self.fact(&mut tx, actor, id, "payment.confirmed").await?;
        }
        tx.commit().await?;
        self.schedule(id).await?;
        self.detail(actor, id).await
    }
    pub(crate) async fn schedule(&self, id: Uuid) -> Result<OperationStatus> {
        let intent = self.intent(id).await?;
        let q = self.quote_record(intent.quote_id).await?;
        let op = self
            .operations
            .accept(
                &intent.actor,
                "payment.execute",
                &id.to_string(),
                &q.instruction,
                JobSpec {
                    kind: "payment.execute".into(),
                    step_key: "initial".into(),
                    payload: serde_json::json!({"paymentId":id}),
                    delay_seconds: 0,
                },
            )
            .await?;
        sqlx::query("UPDATE payments.intents SET operation_id=$2 WHERE id=$1 AND (operation_id IS NULL OR operation_id=$2)").bind(id).bind(op.id).execute(&self.pool).await?;
        Ok(op)
    }
    pub async fn detail(&self, actor: &ActorContext, id: Uuid) -> Result<PaymentStatus> {
        self.owned_intent(actor, id).await?;
        self.status_record(id).await
    }
    pub async fn list(&self, actor: &ActorContext) -> Result<Vec<PaymentStatus>> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM payments.intents WHERE issuer=$1 AND subject=$2 ORDER BY created_at DESC,id LIMIT 100").bind(&actor.issuer).bind(&actor.subject).fetch_all(&self.pool).await?;
        let person = match self.controls.person(actor).await {
            Ok(p) => p,
            Err(e) if e.downcast_ref::<sqlx::Error>().is_none() => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let owned = self.core.accounts_for(person).await?;
        let mut result = vec![];
        for id in ids {
            let status = self.status_record(id).await?;
            if owned
                .iter()
                .any(|a| a.id == status.details.payer_account_id)
            {
                result.push(status);
            }
        }
        Ok(result)
    }
    pub(crate) async fn status_record(&self, id: Uuid) -> Result<PaymentStatus> {
        let r=sqlx::query("SELECT i.*,b.label,q.instruction,q.expires_at,q.total_minor FROM payments.intents i JOIN payments.quotes q ON q.id=i.quote_id JOIN payments.beneficiaries b ON b.id=q.beneficiary_id WHERE i.id=$1").bind(id).fetch_one(&self.pool).await?;
        let instruction: bank_core::payments::InternalInstruction =
            serde_json::from_value(r.get("instruction"))?;
        let challenge: Option<Uuid> = r.get("challenge_id");
        let state: String = r.get("state");
        if state == "SETTLED"
            && (r.get::<Option<Uuid>, _>("batch_id").is_none()
                || self.core.payment_receipt(&instruction).await?
                    != r.get::<Option<Uuid>, _>("batch_id"))
        {
            return Err(
                sqlx::Error::Protocol("Payment booking evidence is inconsistent".into()).into(),
            );
        }
        if instruction.details.rail == "SIMULATED"
            && ["DECLINED", "EXPIRED", "BLOCKED", "CANCELLED"].contains(&state.as_str())
            && self
                .core
                .external_reservation(&instruction)
                .await?
                .is_some_and(|r| r.submitted)
            && self
                .core
                .external_finality(&instruction)
                .await?
                .is_none_or(|f| f.outcome != state)
        {
            return Err(sqlx::Error::Protocol(
                "Submitted funds lack definitive nonexecution evidence".into(),
            )
            .into());
        }
        let actor: ActorContext = serde_json::from_value(r.get("actor_context"))?;
        let cancellation = self.cancellation_summary(id).await?;
        let ready = if state == "DRAFT" {
            if let Some(challenge) = challenge {
                self.identity
                    .step_up_ready(
                        &actor,
                        challenge,
                        id,
                        &bank_platform::domain::fingerprint(&instruction.details)?,
                    )
                    .await?
            } else {
                false
            }
        } else {
            false
        };
        Ok(PaymentStatus {
            id,
            quote_id: r.get("quote_id"),
            total: instruction
                .details
                .amount
                .checked_add(instruction.details.fee)?,
            details: instruction.details,
            beneficiary_label: r.get("label"),
            approval_url: challenge
                .filter(|_| state == "DRAFT")
                .map(|id| format!("/auth/customer/step-up/{id}")),
            challenge_id: challenge,
            approval_ready: ready,
            state,
            reason_code: r.get("reason_code"),
            returns: self.core.payment_returns(id).await?,
            cancellation: cancellation.clone(),
            allowed_actions: if r.get::<String, _>("state") == "DRAFT" {
                vec!["CANCEL_DRAFT".into()]
            } else if !["SETTLED", "DECLINED", "BLOCKED", "EXPIRED", "CANCELLED"]
                .contains(&r.get::<String, _>("state").as_str())
                && cancellation.is_none()
            {
                vec!["CANCEL_PAYMENT".into()]
            } else {
                vec![]
            },
            version: r.get("version"),
            operation_id: r.get("operation_id"),
            batch_id: r.get("batch_id"),
            expires_at: r.get("expires_at"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
    }
    pub(crate) async fn fact(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &ActorContext,
        id: Uuid,
        action: &str,
    ) -> Result<()> {
        let r = sqlx::query("SELECT version,state,reason_code FROM payments.intents WHERE id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
        EventStore::new(self.pool.clone(),Module::Payments).stage(tx,&EventEnvelope{event_id:EventId::new(),event_type:"payment.changed".into(),schema_version:1,producer:"payments".into(),aggregate_id:id,aggregate_version:r.get("version"),occurred_at:Utc::now(),correlation_id:id,causation_id:id.to_string(),data:serde_json::json!({"paymentId":id,"state":r.get::<String,_>("state"),"reasonCode":r.get::<String,_>("reason_code")})}).await?;
        Self::audit_in(tx, actor, action, id).await
    }
}
