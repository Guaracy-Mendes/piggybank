use crate::Payments;
use anyhow::{Result, ensure};
use bank_controls::{entitlements::Action, governance::ChangeKind};
use bank_platform::domain::{ActorContext, Money, TrustContext};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
pub const SIMULATED_CLEARING: Uuid = Uuid::from_u128(0x4f1a780b_3e31_48ef_a294_000000000003);
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RailPolicyVersion {
    pub id: Uuid,
    pub rail: String,
    pub version: i32,
    pub label: String,
    pub enabled: bool,
    pub effective_from: NaiveDate,
    pub effective_until: NaiveDate,
    pub payment_limit: Money,
    pub calendar: String,
}
impl RailPolicyVersion {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.rail == "SIMULATED"
                && self.label == "SYNTHETIC"
                && self.version > 0
                && self.effective_until > self.effective_from
                && self.calendar == "SYNTHETIC_UTC_24X7",
            "Invalid synthetic rail policy"
        );
        self.payment_limit.require_positive()?;
        Ok(())
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RailPolicySnapshot {
    pub policy: Option<RailPolicyVersion>,
    pub paused: bool,
    pub revision: i64,
}
impl RailPolicySnapshot {
    pub fn enabled_policy(&self) -> Result<&RailPolicyVersion> {
        let p = self
            .policy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Rail policy unavailable"))?;
        ensure!(!self.paused && p.enabled, "New rail submissions are paused");
        Ok(p)
    }
}
impl Payments {
    pub async fn rail_policy(&self) -> Result<RailPolicySnapshot> {
        let mut tx = self.pool.begin().await?;
        let snapshot = Self::rail_policy_in(&mut tx, false).await?;
        tx.commit().await?;
        Ok(snapshot)
    }
    pub(crate) async fn rail_policy_in(
        tx: &mut Transaction<'_, Postgres>,
        locked: bool,
    ) -> Result<RailPolicySnapshot> {
        let query = if locked {
            "SELECT paused,revision FROM payments.rail_policy_head WHERE rail='SIMULATED' FOR SHARE"
        } else {
            "SELECT paused,revision FROM payments.rail_policy_head WHERE rail='SIMULATED'"
        };
        let head = sqlx::query(query).fetch_one(&mut **tx).await?;
        let value:Option<Value>=sqlx::query_scalar("SELECT document FROM payments.rail_policies WHERE rail='SIMULATED' AND (document->>'effectiveFrom')::date<=(now() AT TIME ZONE 'UTC')::date AND (document->>'effectiveUntil')::date>(now() AT TIME ZONE 'UTC')::date ORDER BY version DESC LIMIT 1").fetch_optional(&mut **tx).await?;
        Ok(RailPolicySnapshot {
            policy: value.map(serde_json::from_value).transpose()?,
            paused: head.get("paused"),
            revision: head.get("revision"),
        })
    }
    pub async fn publish_rail_policy(
        &self,
        actor: &ActorContext,
        proposal: Uuid,
        draft: &RailPolicyVersion,
    ) -> Result<()> {
        draft.validate()?;
        let proof = self
            .controls
            .approved_change(actor, proposal, ChangeKind::Rail, draft.id, draft)
            .await?;
        let (maker, checker) = proof.people();
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT rail FROM payments.rail_policy_head WHERE rail='SIMULATED' FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
        let created=sqlx::query("INSERT INTO payments.rail_policies(id,rail,version,document,approval_id,maker_person,checker_person) VALUES($1,'SIMULATED',$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING RETURNING id").bind(draft.id).bind(draft.version).bind(serde_json::to_value(draft)?).bind(proposal).bind(maker).bind(checker).fetch_optional(&mut *tx).await?;
        let saved =
            sqlx::query("SELECT document,approval_id FROM payments.rail_policies WHERE id=$1")
                .bind(draft.id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            saved.get::<Value, _>("document") == serde_json::to_value(draft)?
                && saved.get::<Uuid, _>("approval_id") == proposal,
            "Rail policy identity conflict"
        );
        if created.is_some() {
            // First effective approved policy enables the explicit local fixture. Emergency pause is sticky for subsequent policy publications.
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM payments.rail_policies")
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query("UPDATE payments.rail_policy_head SET paused=CASE WHEN $1 AND $2 THEN false ELSE paused END,revision=revision+1 WHERE rail='SIMULATED'").bind(count==1&&draft.enabled).bind(draft.effective_from<=Utc::now().date_naive()&&draft.effective_until>Utc::now().date_naive()).execute(&mut *tx).await?;
            Self::audit_in(&mut tx, actor, "rail.policy-published", draft.id).await?;
        }
        sqlx::query("UPDATE payments.rail_changes SET acted_at=now() WHERE proposal_id=$1 AND acted_at IS NULL").bind(proposal).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn pause_rail(&self, actor: &ActorContext) -> Result<()> {
        ensure!(
            actor.trust == TrustContext::Workforce
                && self
                    .controls
                    .authorize(actor, Action::ManageRails, None, SIMULATED_CLEARING)
                    .await?,
            "Rail pause denied"
        );
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE payments.rail_policy_head SET paused=true,revision=revision+1 WHERE rail='SIMULATED' AND NOT paused").execute(&mut *tx).await?;
        if changed.rows_affected() == 1 {
            Self::audit_in(
                &mut tx,
                actor,
                "rail.submissions-paused",
                SIMULATED_CLEARING,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn propose_rail_policy(
        &self,
        actor: &ActorContext,
        draft: &RailPolicyVersion,
    ) -> Result<Uuid> {
        draft.validate()?;
        self.stage_rail_change(actor, "POLICY", draft.id, serde_json::to_value(draft)?)
            .await
    }
    pub async fn propose_rail_resume(&self, actor: &ActorContext) -> Result<Uuid> {
        let current = self.rail_policy().await?;
        let policy = current
            .policy
            .ok_or_else(|| anyhow::anyhow!("No rail policy"))?;
        ensure!(
            policy.enabled && current.paused,
            "Only an enabled paused rail can resume"
        );
        self.stage_rail_change(actor,"RESUME",policy.id,json!({"action":"RESUME_SUBMISSIONS","rail":"SIMULATED","policyVersionId":policy.id,"revision":current.revision})).await
    }
    async fn stage_rail_change(
        &self,
        actor: &ActorContext,
        kind: &str,
        resource: Uuid,
        details: Value,
    ) -> Result<Uuid> {
        let proposal = self
            .controls
            .propose_change(
                actor,
                ChangeKind::Rail,
                resource,
                &details,
                Utc::now() + Duration::hours(1),
            )
            .await?;
        let person = self.controls.person(actor).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO payments.rail_changes(proposal_id,kind,resource_id,details,maker_person) VALUES($1,$2,$3,$4,$5)").bind(proposal).bind(kind).bind(resource).bind(details).bind(person.0).execute(&mut *tx).await?;
        Self::audit_in(&mut tx, actor, "rail.change-staged", resource).await?;
        tx.commit().await?;
        Ok(proposal)
    }
    pub async fn rail_changes(&self, actor: &ActorContext) -> Result<Value> {
        ensure!(
            actor.trust == TrustContext::Workforce
                && (self
                    .controls
                    .authorize(actor, Action::ManageRails, None, SIMULATED_CLEARING)
                    .await?
                    || self
                        .controls
                        .authorize(actor, Action::ApproveRails, None, SIMULATED_CLEARING)
                        .await?),
            "Rail change inquiry denied"
        );
        let rows = sqlx::query(
            "SELECT * FROM payments.rail_changes ORDER BY created_at DESC,proposal_id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(
            json!({"items":rows.into_iter().map(|r|json!({"proposalId":r.get::<Uuid,_>("proposal_id"),"kind":r.get::<String,_>("kind"),"resourceId":r.get::<Uuid,_>("resource_id"),"details":r.get::<Value,_>("details"),"makerPerson":r.get::<Uuid,_>("maker_person"),"actedAt":r.get::<Option<DateTime<Utc>>,_>("acted_at"),"createdAt":r.get::<DateTime<Utc>,_>("created_at")})).collect::<Vec<_>>()}),
        )
    }
    pub async fn approve_rail_change(&self, actor: &ActorContext, proposal: Uuid) -> Result<()> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM payments.rail_changes WHERE proposal_id=$1)",
        )
        .bind(proposal)
        .fetch_one(&self.pool)
        .await?;
        ensure!(exists, "Unknown rail change");
        self.controls.approve_change(actor, proposal).await
    }
    pub async fn apply_rail_change(&self, actor: &ActorContext, proposal: Uuid) -> Result<()> {
        let row = sqlx::query(
            "SELECT kind,resource_id,details FROM payments.rail_changes WHERE proposal_id=$1",
        )
        .bind(proposal)
        .fetch_one(&self.pool)
        .await?;
        let value: Value = row.get("details");
        if row.get::<String, _>("kind") == "POLICY" {
            let draft: RailPolicyVersion = serde_json::from_value(value)?;
            return self.publish_rail_policy(actor, proposal, &draft).await;
        }
        let resource: Uuid = row.get("resource_id");
        let proof = self
            .controls
            .approved_change(actor, proposal, ChangeKind::Rail, resource, &value)
            .await?;
        proof.assert_bound("RAIL_POLICY", resource, &value)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT rail FROM payments.rail_policy_head WHERE rail='SIMULATED' FOR UPDATE")
            .fetch_one(&mut *tx)
            .await?;
        let acted: bool = sqlx::query_scalar(
            "SELECT acted_at IS NOT NULL FROM payments.rail_changes WHERE proposal_id=$1",
        )
        .bind(proposal)
        .fetch_one(&mut *tx)
        .await?;
        if acted {
            tx.commit().await?;
            return Ok(());
        }
        let snapshot = Self::rail_policy_in(&mut tx, false).await?;
        ensure!(
            value
                == json!({"action":"RESUME_SUBMISSIONS","rail":"SIMULATED","policyVersionId":resource,"revision":snapshot.revision})
                && snapshot
                    .policy
                    .as_ref()
                    .is_some_and(|p| p.id == resource && p.enabled),
            "Resume approval scope changed"
        );
        sqlx::query("UPDATE payments.rail_policy_head SET paused=false,revision=revision+1 WHERE rail='SIMULATED'").execute(&mut *tx).await?;
        sqlx::query("UPDATE payments.rail_changes SET acted_at=now() WHERE proposal_id=$1 AND acted_at IS NULL").bind(proposal).execute(&mut *tx).await?;
        Self::audit_in(&mut tx, actor, "rail.submissions-resumed", resource).await?;
        tx.commit().await?;
        Ok(())
    }
}
