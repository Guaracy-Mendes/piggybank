//! Workforce read models and immutable configuration proposals. No financial write capability.
use crate::{
    audit::{AuditLog, AuditRecord},
    entitlements::{Action, Entitlements},
    governance::ChangeKind,
};
use anyhow::{Result, ensure};
use bank_platform::domain::{ActorContext, TrustContext, fingerprint};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaffAccess {
    pub person_id: Uuid,
    pub capabilities: Vec<String>,
}
impl Entitlements {
    pub async fn finance_approval_authorized(&self, actor: &ActorContext) -> Result<bool> {
        if actor.trust != TrustContext::Workforce
            || !self
                .authorize(actor, Action::ApproveCorrection, None, Uuid::nil())
                .await?
        {
            return Ok(false);
        }
        self.finance_checker_current(self.person(actor).await?.0)
            .await
    }
    pub async fn finance_checker_current(&self, person: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=$1 AND role='finance_approver' AND scope IS NULL AND revoked_at IS NULL AND valid_from<=now() AND valid_until>now()) AND EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=$1 AND queue='finance-operations' AND capability='APPROVE' AND revoked_at IS NULL)").bind(person).fetch_one(&self.pool).await?)
    }
    pub async fn finance_authorized(&self, actor: &ActorContext, write: bool) -> Result<bool> {
        if actor.trust != TrustContext::Workforce {
            return Ok(false);
        }
        let maker = self
            .authorize(actor, Action::ProposeCorrection, None, Uuid::nil())
            .await?;
        let role = if maker {
            true
        } else if !write {
            self.authorize(actor, Action::ApproveCorrection, None, Uuid::nil())
                .await?
        } else {
            false
        };
        if !role {
            return Ok(false);
        }
        let person = self.person(actor).await?;
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=$1 AND queue='finance-operations' AND capability=ANY($2) AND revoked_at IS NULL)")
            .bind(person.0).bind(if write {vec!["REVIEW"]} else {vec!["REVIEW","APPROVE"]}).fetch_one(&self.pool).await?)
    }
    pub async fn staff_access(&self, actor: &ActorContext) -> Result<StaffAccess> {
        ensure!(actor.trust == TrustContext::Workforce, "Workforce required");
        let person = self.person(actor).await?;
        let mut capabilities: Vec<String> = vec![];
        for (name, action) in [
            ("case.review", Action::ReviewCase),
            ("payment.inquire", Action::InquirePayment),
            ("rail.manage", Action::ManageRails),
            ("rail.approve", Action::ApproveRails),
            ("case.approve", Action::ApproveCase),
            ("product.propose", Action::ProposeProduct),
            ("product.approve", Action::ApproveProduct),
            ("roles.manage", Action::ManageRoles),
            ("roles.approve", Action::ApproveRoles),
        ] {
            if self.authorize(actor, action, None, person.0).await? {
                capabilities.push(name.into());
            }
        }
        let records_manage = self
            .authorize(actor, Action::ManageRecords, None, person.0)
            .await?;
        let records_approve = self
            .authorize(actor, Action::ApproveRecords, None, person.0)
            .await?;
        if records_manage
            || records_approve
            || self
                .authorize(actor, Action::ReadAccount, None, person.0)
                .await?
        {
            capabilities.push("records.read".into());
        }
        let steward = self
            .authorize(actor, Action::ProposeMaster, None, person.0)
            .await?;
        let replay = self
            .authorize(actor, Action::ReplayEvents, None, person.0)
            .await?;
        if steward {
            capabilities.push("data.steward".into());
        }
        if replay {
            capabilities.push("events.replay".into());
        }
        if steward
            || replay
            || capabilities
                .iter()
                .any(|c| ["case.review", "case.approve", "payment.inquire"].contains(&c.as_str()))
        {
            capabilities.push("operations.view".into());
        }
        if records_manage {
            capabilities.push("records.manage".into());
        }
        if records_approve {
            capabilities.push("records.approve".into());
        }
        if self.finance_authorized(actor, false).await? {
            capabilities.push("finance.inquire".into());
        }
        if self.finance_authorized(actor, true).await? {
            capabilities.push("finance.recover".into());
        }
        if self.finance_approval_authorized(actor).await? {
            capabilities.push("finance.approve".into());
        }
        Ok(StaffAccess {
            person_id: person.0,
            capabilities,
        })
    }
    pub async fn stage_configuration(
        &self,
        actor: &ActorContext,
        kind: ChangeKind,
        resource: Uuid,
        details: &Value,
    ) -> Result<Uuid> {
        let kind_code = match kind {
            ChangeKind::Product => "PRODUCT",
            ChangeKind::Terms => "TERMS",
            _ => anyhow::bail!("Unsupported configuration kind"),
        };
        ensure!(
            actor.trust == TrustContext::Workforce
                && self
                    .authorize(actor, Action::ProposeProduct, None, resource)
                    .await?,
            "Configuration proposal denied"
        );
        let person = self.person(actor).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,28))")
            .bind(resource.to_string())
            .execute(&mut *tx)
            .await?;
        if let Some(row)=sqlx::query("SELECT d.*,p.maker_person FROM controls.configuration_drafts d JOIN controls.change_proposals p ON p.id=d.proposal_id WHERE d.resource_id=$1").bind(resource).fetch_optional(&mut *tx).await? {
   ensure!(row.get::<Value,_>("details")==*details && row.get::<String,_>("kind")==kind_code && row.get::<Uuid,_>("maker_person")==person.0,"Configuration identity conflict");
   let id=row.get("proposal_id");tx.commit().await?;return Ok(id);
  }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO controls.change_proposals(id,kind,resource_id,digest,maker_person,expires_at) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(kind_code).bind(resource).bind(fingerprint(details)?).bind(person.0).bind(Utc::now()+Duration::hours(24)).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO controls.configuration_drafts(proposal_id,resource_id,kind,details) VALUES($1,$2,$3,$4)").bind(id).bind(resource).bind(kind_code).bind(details).execute(&mut *tx).await?;
        AuditLog::append_in(
            &mut tx,
            id,
            "governance",
            &AuditRecord::new(actor, "change.proposed", resource, id),
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn configurations(&self, actor: &ActorContext) -> Result<Value> {
        let access = self.staff_access(actor).await?;
        ensure!(
            access
                .capabilities
                .iter()
                .any(|a| a.starts_with("product.")),
            "Configuration inquiry denied"
        );
        let rows=sqlx::query("SELECT d.*,p.state,p.maker_person,p.checker_person,p.expires_at FROM controls.configuration_drafts d JOIN controls.change_proposals p ON p.id=d.proposal_id ORDER BY p.created_at DESC,p.id LIMIT 100").fetch_all(&self.pool).await?;
        Ok(
            json!({"items":rows.into_iter().map(|r|json!({"proposalId":r.get::<Uuid,_>("proposal_id"),"resourceId":r.get::<Uuid,_>("resource_id"),"kind":r.get::<String,_>("kind"),"details":r.get::<Value,_>("details"),"state":r.get::<String,_>("state"),"makerPerson":r.get::<Uuid,_>("maker_person"),"checkerPerson":r.get::<Option<Uuid>,_>("checker_person"),"expiresAt":r.get::<DateTime<Utc>,_>("expires_at"),"publishedAt":r.get::<Option<DateTime<Utc>>,_>("published_at")})).collect::<Vec<_>>()}),
        )
    }
    pub async fn role_register(&self, actor: &ActorContext) -> Result<Value> {
        let access = self.staff_access(actor).await?;
        ensure!(
            access.capabilities.iter().any(|a| a.starts_with("roles.")),
            "Role register denied"
        );
        let proposals=sqlx::query("SELECT id,target_person,role,scope,maker_person,checker_person,state,valid_until FROM controls.role_proposals ORDER BY created_at DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        let grants=sqlx::query("SELECT id,person_id,role,scope,valid_until,revoked_at FROM controls.role_grants ORDER BY valid_from DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        Ok(
            json!({"proposals":proposals.into_iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"targetPerson":r.get::<Uuid,_>("target_person"),"role":r.get::<String,_>("role"),"scope":r.get::<Option<Uuid>,_>("scope"),"makerPerson":r.get::<Uuid,_>("maker_person"),"checkerPerson":r.get::<Option<Uuid>,_>("checker_person"),"state":r.get::<String,_>("state"),"validUntil":r.get::<DateTime<Utc>,_>("valid_until")})).collect::<Vec<_>>(),"grants":grants.into_iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"personId":r.get::<Uuid,_>("person_id"),"role":r.get::<String,_>("role"),"scope":r.get::<Option<Uuid>,_>("scope"),"validUntil":r.get::<DateTime<Utc>,_>("valid_until"),"revokedAt":r.get::<Option<DateTime<Utc>>,_>("revoked_at")})).collect::<Vec<_>>()}),
        )
    }
}
