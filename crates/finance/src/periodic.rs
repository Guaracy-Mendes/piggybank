//! Durable approval workflow for preview-only Core policy configuration.
use crate::Finance;
use anyhow::{Result, ensure};
use bank_controls::governance::ChangeKind;
use bank_core::accruals::{
    PeriodicConflict, PeriodicInputError, PeriodicPolicy, PeriodicPreview, PreviewInput,
    PublishedPolicy,
};
use bank_platform::domain::{ActorContext, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyInput {
    pub policy: PeriodicPolicy,
    pub reason: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRequest {
    pub id: Uuid,
    pub policy: PeriodicPolicy,
    pub policy_digest: String,
    pub reason: String,
    pub maker_person: Uuid,
    pub checker_person: Option<Uuid>,
    pub state: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub registered_at: Option<DateTime<Utc>>,
    pub registration_reason: String,
    pub publication: Option<PublishedPolicy>,
}
struct Draft {
    id: Uuid,
    policy: PeriodicPolicy,
    digest: String,
    reason: String,
    maker: Uuid,
    actor: ActorContext,
    expires: DateTime<Utc>,
}
fn draft(r: &sqlx::postgres::PgRow) -> Result<Draft> {
    let policy: PeriodicPolicy = serde_json::from_value(r.get("policy"))?;
    policy.validate()?;
    let digest: String = r.get("policy_digest");
    ensure!(
        policy.id == r.get::<Uuid, _>("policy_id") && fingerprint(&policy)? == digest,
        PeriodicConflict
    );
    Ok(Draft {
        id: r.get("id"),
        policy,
        digest,
        reason: r.get("reason"),
        maker: r.get("maker_person"),
        actor: serde_json::from_value(r.get("actor_context"))?,
        expires: r.get("expires_at"),
    })
}
impl Finance {
    pub async fn propose_periodic_policy(
        &self,
        actor: &ActorContext,
        key: Uuid,
        input: &PolicyInput,
    ) -> Result<PolicyRequest> {
        self.authorize_staff(actor, true).await?;
        ensure!(
            !key.is_nil()
                && (4..=240).contains(&input.reason.chars().count())
                && input.reason.trim() == input.reason
                && !input.reason.chars().any(char::is_control),
            PeriodicInputError
        );
        self.periodic.validate_product(&input.policy).await?;
        let maker = self.controls.person(actor).await?.0;
        let digest = fingerprint(&input.policy)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,177))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        if let Some(r)=sqlx::query("SELECT * FROM finance.periodic_policy_requests WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await?{
   let d=draft(&r)?;ensure!(d.digest==digest && d.reason==input.reason && d.maker==maker,PeriodicConflict);tx.commit().await?;return self.periodic_policy_request(actor,d.id).await
  }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO finance.periodic_policy_requests(id,policy_id,issuer,subject,client_id,request_key,actor_context,maker_person,policy,policy_digest,reason,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,now()+interval '24 hours')").bind(id).bind(input.policy.id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(serde_json::to_value(actor)?).bind(maker).bind(serde_json::to_value(&input.policy)?).bind(&digest).bind(&input.reason).execute(&mut *tx).await?;
        Self::audit_in(&mut tx, actor, "finance.periodic-policy.requested", id).await?;
        tx.commit().await?;
        self.periodic_policy_request(actor, id).await
    }
    pub async fn register_periodic_policy(&self, id: Uuid) -> Result<()> {
        let r = sqlx::query("SELECT * FROM finance.periodic_policy_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let d = draft(&r)?;
        self.authorize_staff(&d.actor, true).await?;
        ensure!(
            self.controls.person(&d.actor).await?.0 == d.maker && d.expires > Utc::now(),
            PeriodicConflict
        );
        self.controls
            .propose_change_id(
                &d.actor,
                id,
                ChangeKind::PeriodicPolicy,
                d.policy.id,
                &d.policy,
                d.expires,
            )
            .await?;
        sqlx::query("UPDATE finance.periodic_policy_requests SET registered_at=coalesce(registered_at,now()),registration_reason='REGISTERED' WHERE id=$1").bind(id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn periodic_policy_request(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<PolicyRequest> {
        self.authorize_staff(actor, false).await?;
        let r = sqlx::query("SELECT * FROM finance.periodic_policy_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let d = draft(&r)?;
        let control = self
            .controls
            .change_record(id, ChangeKind::PeriodicPolicy)
            .await?;
        if let Some(c) = &control {
            ensure!(
                c.resource_id == d.policy.id
                    && c.digest == d.digest
                    && c.maker_person == d.maker
                    && c.expires_at == d.expires,
                PeriodicConflict
            );
        }
        let publication = self
            .periodic
            .get(d.policy.id)
            .await?
            .filter(|p| p.proposal_id == id);
        if let Some(p) = &publication {
            ensure!(
                p.digest == d.digest && p.maker_person == d.maker && p.checker_person != d.maker,
                PeriodicConflict
            );
        }
        let state = if publication.is_some() {
            "PUBLISHED".into()
        } else if d.expires <= Utc::now() {
            "EXPIRED".into()
        } else {
            control
                .as_ref()
                .map(|c| c.state.clone())
                .unwrap_or_else(|| {
                    if r.get::<Option<DateTime<Utc>>, _>("registered_at").is_some() {
                        "CONTROL_RECORD_UNAVAILABLE".into()
                    } else {
                        "STAGED".into()
                    }
                })
        };
        Ok(PolicyRequest {
            id,
            policy: d.policy,
            policy_digest: d.digest,
            reason: d.reason,
            maker_person: d.maker,
            checker_person: publication
                .as_ref()
                .map(|p| p.checker_person)
                .or_else(|| control.and_then(|c| c.checker_person)),
            state,
            expires_at: d.expires,
            created_at: r.get("created_at"),
            registered_at: r.get("registered_at"),
            registration_reason: r.get("registration_reason"),
            publication,
        })
    }
    pub async fn periodic_policy_requests(
        &self,
        actor: &ActorContext,
    ) -> Result<Vec<PolicyRequest>> {
        self.authorize_staff(actor, false).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM finance.periodic_policy_requests ORDER BY created_at DESC,id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.periodic_policy_request(actor, id).await?);
        }
        Ok(out)
    }
    pub async fn approve_periodic_policy(
        &self,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<PolicyRequest> {
        ensure!(
            self.controls.finance_approval_authorized(actor).await?,
            "Current Finance checker authority required"
        );
        let r = self.periodic_policy_request(actor, id).await?;
        ensure!(
            r.policy_digest == digest
                && ["STAGED", "PENDING", "APPROVED"].contains(&r.state.as_str()),
            PeriodicConflict
        );
        self.register_periodic_policy(id).await?;
        self.controls.approve_change(actor, id).await?;
        self.periodic_policy_request(actor, id).await
    }
    pub async fn publish_periodic_policy(
        &self,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<PolicyRequest> {
        self.authorize_staff(actor, true).await?;
        let r = self.periodic_policy_request(actor, id).await?;
        ensure!(
            r.policy_digest == digest
                && r.maker_person == self.controls.person(actor).await?.0
                && ["APPROVED", "PUBLISHED"].contains(&r.state.as_str()),
            PeriodicConflict
        );
        self.periodic
            .publish(&self.controls, actor, id, &r.policy)
            .await?;
        // Core's immutable publication is the receipt, even when this response is lost.
        self.periodic_policy_request(actor, id).await
    }
    pub async fn preview_periodic_policy(
        &self,
        actor: &ActorContext,
        id: Uuid,
        input: &PreviewInput,
    ) -> Result<PeriodicPreview> {
        self.authorize_staff(actor, false).await?;
        let r = self.periodic_policy_request(actor, id).await?;
        let publication = r.publication.ok_or(PeriodicConflict)?;
        publication.policy.preview(input)
    }
    pub async fn recover_periodic_policies(&self) -> Result<()> {
        let after: Uuid =
            sqlx::query_scalar("SELECT last_id FROM finance.periodic_policy_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.periodic_policy_requests WHERE id>$1 AND registered_at IS NULL AND expires_at>now() ORDER BY id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        sqlx::query("UPDATE finance.periodic_policy_cursor SET last_id=$1 WHERE id")
            .bind(ids.last().copied().unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        for id in ids {
            let result = self.register_periodic_policy(id).await;
            sqlx::query("UPDATE finance.periodic_policy_requests SET registration_attempts=registration_attempts+1,registration_reason=CASE WHEN registered_at IS NOT NULL THEN 'REGISTERED' ELSE $2 END WHERE id=$1").bind(id).bind(if result.is_ok(){"REGISTERED"}else{"CURRENT_AUTHORITY_OR_DEPENDENCY_UNAVAILABLE"}).execute(&self.pool).await?;
        }
        Ok(())
    }
}
