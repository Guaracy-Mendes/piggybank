use crate::audit::{AuditLog, AuditRecord, shard_stream};
use anyhow::{Result, ensure};
use bank_platform::domain::{
    ActorContext, Clock, PartyId, PaymentDetails, SystemClock, TrustContext, fingerprint,
};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

/// Built by the owning domain from current authoritative records, never deserialized from requests.
pub struct ResourceAuthority {
    pub resource_id: Uuid,
    pub owners: Vec<PartyId>,
    pub permitted_staff: Vec<PartyId>,
    pub action_restricted: bool,
    /// WP-33 (FR-ACC-08): current, unexpired, unrevoked delegations on the account.
    pub delegates: Vec<Delegate>,
}
/// Bounded delegated authority. A delegate may read; with `can_pay` may also start payments, each no larger than
/// `payment_limit_minor` (enforced by Payments against the exact amount). Delegates never service the account.
#[derive(Clone, Debug)]
pub struct Delegate {
    pub person: PartyId,
    pub can_pay: bool,
    pub payment_limit_minor: Option<i64>,
}
impl ResourceAuthority {
    pub fn delegate(&self, person: PartyId) -> Option<&Delegate> {
        self.delegates.iter().find(|d| d.person == person)
    }
}
#[derive(Clone, Copy)]
pub struct ApprovalRef {
    pub challenge_id: Uuid,
    pub operation_id: Uuid,
}
#[derive(Clone, Copy)]
pub enum Action {
    ReadAccount,
    CreatePayment,
    InquirePayment,
    ServiceAccount,
    ReviewCase,
    ApproveCase,
    ManageRoles,
    ApproveRoles,
    WorkflowWork,
    ReplayEvents,
    ProposeProduct,
    ApproveProduct,
    ProposeCorrection,
    ApproveCorrection,
    ProposeMaster,
    ApproveMaster,
    ManageRails,
    ApproveRails,
    ManageRecords,
    ApproveRecords,
}
impl Action {
    fn code(self) -> &'static str {
        match self {
            Self::ReadAccount => "account.read",
            Self::CreatePayment => "payment.create",
            Self::InquirePayment => "payment.inquire",
            Self::ServiceAccount => "account.service",
            Self::ReviewCase => "case.review",
            Self::ApproveCase => "case.approve",
            Self::ManageRoles => "roles.manage",
            Self::ApproveRoles => "roles.approve",
            Self::WorkflowWork => "workflow.execute",
            Self::ReplayEvents => "events.replay",
            Self::ProposeProduct => "product.propose",
            Self::ApproveProduct => "product.approve",
            Self::ProposeCorrection => "correction.propose",
            Self::ApproveCorrection => "correction.approve",
            Self::ProposeMaster => "master.propose",
            Self::ApproveMaster => "master.approve",
            Self::ManageRails => "rail.manage",
            Self::ApproveRails => "rail.approve",
            Self::ManageRecords => "records.manage",
            Self::ApproveRecords => "records.approve",
        }
    }
    fn role(self) -> &'static str {
        match self {
            Self::ReadAccount | Self::ServiceAccount => "account_support",
            Self::CreatePayment => "payment_operator",
            Self::InquirePayment => "payment_inquiry",
            Self::ReviewCase => "case_reviewer",
            Self::ApproveCase => "case_approver",
            Self::ManageRoles => "security_admin",
            Self::ApproveRoles => "security_approver",
            Self::WorkflowWork => "workflow_service",
            Self::ReplayEvents => "operations_replay",
            Self::ProposeProduct => "product_admin",
            Self::ApproveProduct => "product_approver",
            Self::ProposeCorrection => "finance_adjuster",
            Self::ApproveCorrection => "finance_approver",
            Self::ProposeMaster => "data_steward",
            Self::ApproveMaster => "data_approver",
            Self::ManageRails => "payment_operator",
            Self::ApproveRails => "payment_approver",
            Self::ManageRecords => "records_officer",
            Self::ApproveRecords => "records_approver",
        }
    }
}
#[derive(Clone)]
pub struct Entitlements {
    pub pool: PgPool,
    clock: Arc<dyn Clock>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub intent_digest: String,
    pub nonce: String,
    pub expires_at: DateTime<Utc>,
}
impl Entitlements {
    pub async fn revoke_role(
        &self,
        actor: &ActorContext,
        grant: Uuid,
        correlation: Uuid,
    ) -> Result<()> {
        ensure!(
            actor.trust == TrustContext::Workforce
                && self
                    .authorize(actor, Action::ManageRoles, None, correlation)
                    .await?,
            "Role revocation denied"
        );
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE controls.role_grants SET revoked_at=now() WHERE id=$1 AND revoked_at IS NULL",
        )
        .bind(grant)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() == 1 {
            AuditLog::append_in(
                &mut tx,
                Uuid::new_v4(),
                "roles",
                &AuditRecord::new(actor, "role.revoked", grant, correlation),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            clock: Arc::new(SystemClock),
        }
    }
    /// Core integration after approved onboarding; existing links cannot be replaced.
    pub async fn link_person(&self, actor: &ActorContext, person: PartyId) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO controls.person_links(issuer,subject,person_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(&actor.issuer).bind(&actor.subject).bind(person.0).execute(&mut *tx).await?;
        let saved: Uuid = sqlx::query_scalar(
            "SELECT person_id FROM controls.person_links WHERE issuer=$1 AND subject=$2",
        )
        .bind(&actor.issuer)
        .bind(&actor.subject)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(
            saved == person.0,
            "Existing canonical identity link conflicts"
        );
        AuditLog::append_in(
            &mut tx,
            Uuid::new_v4(),
            "identity-links",
            &AuditRecord::new(actor, "identity.linked", person.0, person.0),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn person(&self, actor: &ActorContext) -> Result<PartyId> {
        let id: Uuid = sqlx::query_scalar(
            "SELECT person_id FROM controls.person_links WHERE issuer=$1 AND subject=$2",
        )
        .bind(&actor.issuer)
        .bind(&actor.subject)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No authoritative person link"))?;
        Ok(PartyId(id))
    }
    async fn has_role(&self, person: PartyId, role: &str, scope: Option<Uuid>) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=$1 AND role=$2 AND (scope IS NULL OR scope=$3) AND revoked_at IS NULL AND valid_from<=$4 AND valid_until>$4)")
            .bind(person.0).bind(role).bind(scope).bind(self.clock.now()).fetch_one(&self.pool).await?)
    }
    pub async fn authorize(
        &self,
        actor: &ActorContext,
        action: Action,
        resource: Option<&ResourceAuthority>,
        correlation: Uuid,
    ) -> Result<bool> {
        let person = match self.person(actor).await {
            Ok(person) => person,
            Err(error) if error.downcast_ref::<sqlx::Error>().is_none() => {
                let mut record = AuditRecord::new(
                    actor,
                    action.code(),
                    resource.map(|r| r.resource_id).unwrap_or(Uuid::nil()),
                    correlation,
                );
                record.outcome = "DENIED".into();
                record.reason_code = "NO_PERSON_LINK".into();
                let id = Uuid::new_v4();
                AuditLog::new(self.pool.clone())
                    .append(id, &shard_stream("authorization", id), &record)
                    .await?;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let allowed = match actor.trust {
            TrustContext::Customer => resource.is_some_and(|r| {
                let owner = r.owners.contains(&person);
                let delegate = r.delegate(person);
                !r.action_restricted
                    && match action {
                        Action::ReadAccount => owner || delegate.is_some(),
                        Action::CreatePayment => owner || delegate.is_some_and(|d| d.can_pay),
                        Action::ServiceAccount => owner,
                        _ => false,
                    }
            }),
            // WP-34: inside an assisted session the staff member acts for the represented customer, limited to the
            // operations confirmed for the channel and to the customer's own, unrestricted accounts.
            TrustContext::Workforce
                if actor.roles.iter().any(|role| role.starts_with("assisted:")) =>
            {
                let session = actor
                    .roles
                    .iter()
                    .find_map(|role| role.strip_prefix("assisted:"))
                    .and_then(|id| Uuid::parse_str(id).ok());
                let operation = match action {
                    Action::ReadAccount => Some("READ_ACCOUNT"),
                    Action::ServiceAccount => Some("PROTECTIVE_FREEZE"),
                    Action::CreatePayment => Some("DRAFT_PAYMENT"),
                    _ => None,
                };
                match (session, operation, actor.represented_party, resource) {
                    (Some(session), Some(operation), Some(customer), Some(r))
                        if r.owners.contains(&customer) && !r.action_restricted =>
                    {
                        sqlx::query_scalar::<_, bool>(
                            "SELECT controls.assisted_permits($1,$2,$3,$4,$5)",
                        )
                        .bind(session)
                        .bind(person.0)
                        .bind(customer.0)
                        .bind(r.resource_id)
                        .bind(operation)
                        .fetch_one(&self.pool)
                        .await?
                    }
                    _ => false,
                }
            }
            TrustContext::Workforce => {
                let related = resource
                    .is_none_or(|r| r.permitted_staff.contains(&person) && !r.action_restricted);
                related
                    && self
                        .has_role(person, action.role(), resource.map(|r| r.resource_id))
                        .await?
            }
            TrustContext::Service => {
                matches!(action, Action::WorkflowWork)
                    && self.has_role(person, action.role(), None).await?
            }
            // WP-33: a partner acts only for the consenting owner, on a consented account, within a live grant.
            // The API limits each endpoint to its scope; here either consented scope may read the account (a
            // payment initiation reads its payer account), and only `payments:initiate` may create a payment.
            TrustContext::Partner => match resource {
                Some(r) if r.owners.contains(&person) && !r.action_restricted => {
                    let consent = actor
                        .roles
                        .iter()
                        .find_map(|role| role.strip_prefix("consent:"))
                        .and_then(|id| Uuid::parse_str(id).ok());
                    let scopes: &[&str] = match action {
                        Action::ReadAccount => &["accounts:read", "payments:initiate"],
                        Action::CreatePayment => &["payments:initiate"],
                        _ => &[],
                    };
                    let mut permitted = false;
                    if let Some(consent) = consent {
                        for scope in scopes {
                            if sqlx::query_scalar::<_, bool>(
                                "SELECT identity.consent_permits($1,$2,$3,$4,$5)",
                            )
                            .bind(consent)
                            .bind(&actor.client_id)
                            .bind(person.0)
                            .bind(r.resource_id)
                            .bind(scope)
                            .fetch_one(&self.pool)
                            .await?
                            {
                                permitted = true;
                                break;
                            }
                        }
                    }
                    permitted
                }
                _ => false,
            },
        };
        let mut resolved = actor.clone();
        resolved.person_id = Some(person);
        let mut record = AuditRecord::new(
            &resolved,
            action.code(),
            resource.map(|r| r.resource_id).unwrap_or(Uuid::nil()),
            correlation,
        );
        record.outcome = if allowed { "ALLOWED" } else { "DENIED" }.into();
        record.reason_code = "ENTITLEMENT_POLICY".into();
        // Keyed by the record id, not the resource: one busy account must not concentrate on one shard.
        let id = Uuid::new_v4();
        AuditLog::new(self.pool.clone())
            .append(id, &shard_stream("authorization", id), &record)
            .await?;
        Ok(allowed)
    }
    pub async fn propose_role(
        &self,
        maker: &ActorContext,
        target: PartyId,
        role: &str,
        scope: Option<Uuid>,
        until: DateTime<Utc>,
        correlation: Uuid,
    ) -> Result<Uuid> {
        ensure!(
            maker.trust == TrustContext::Workforce
                && self
                    .authorize(maker, Action::ManageRoles, None, correlation)
                    .await?,
            "Role management denied"
        );
        ensure!(
            [
                "account_support",
                "payment_operator",
                "payment_approver",
                "payment_inquiry",
                "case_reviewer",
                "case_approver",
                "security_admin",
                "security_approver",
                "workflow_service",
                "operations_replay",
                "product_admin",
                "product_approver",
                "finance_adjuster",
                "finance_approver",
                "data_steward",
                "data_approver",
                "records_officer",
                "records_approver"
            ]
            .contains(&role),
            "Unknown role"
        );
        ensure!(
            until > self.clock.now() && until <= self.clock.now() + Duration::days(90),
            "Invalid synthetic grant duration"
        );
        let known: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM controls.person_links WHERE person_id=$1)",
        )
        .bind(target.0)
        .fetch_one(&self.pool)
        .await?;
        ensure!(known, "Explicit canonical target identity required");
        let person = self.person(maker).await?;
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO controls.role_proposals(id,target_person,role,scope,maker_person,valid_until,state) VALUES($1,$2,$3,$4,$5,$6,'PENDING')")
            .bind(id).bind(target.0).bind(role).bind(scope).bind(person.0).bind(until).execute(&mut *tx).await?;
        AuditLog::append_in(
            &mut tx,
            id,
            "roles",
            &AuditRecord::new(maker, "role.proposed", id, correlation),
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn approve_role(
        &self,
        checker: &ActorContext,
        id: Uuid,
        correlation: Uuid,
    ) -> Result<()> {
        ensure!(
            checker.trust == TrustContext::Workforce
                && self
                    .authorize(checker, Action::ApproveRoles, None, correlation)
                    .await?,
            "Role approval denied"
        );
        let person = self.person(checker).await?;
        let mut tx = self.pool.begin().await?;
        let proposal = sqlx::query("SELECT * FROM controls.role_proposals WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            proposal.get::<Uuid, _>("maker_person") != person.0,
            "Maker cannot approve their own request across sessions or issuers"
        );
        ensure!(
            proposal.get::<DateTime<Utc>, _>("valid_until") > self.clock.now(),
            "Expired grant proposal"
        );
        if proposal.get::<String, _>("state") == "APPROVED" {
            tx.commit().await?;
            return Ok(());
        }
        ensure!(
            proposal.get::<String, _>("state") == "PENDING",
            "Proposal unavailable"
        );
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,scope,valid_until,proposal_id) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(Uuid::new_v4()).bind(proposal.get::<Uuid,_>("target_person")).bind(proposal.get::<String,_>("role")).bind(proposal.get::<Option<Uuid>,_>("scope")).bind(proposal.get::<DateTime<Utc>,_>("valid_until")).bind(id).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE controls.role_proposals SET state='APPROVED',checker_person=$2 WHERE id=$1",
        )
        .bind(id)
        .bind(person.0)
        .execute(&mut *tx)
        .await?;
        AuditLog::append_in(
            &mut tx,
            Uuid::new_v4(),
            "roles",
            &AuditRecord::new(checker, "role.approved", id, correlation),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn challenge(
        &self,
        actor: &ActorContext,
        operation: Uuid,
        details: &PaymentDetails,
        authority: &ResourceAuthority,
        correlation: Uuid,
    ) -> Result<Challenge> {
        ensure!(
            details.payer_account_id.0 == authority.resource_id
                && self
                    .authorize(actor, Action::CreatePayment, Some(authority), correlation)
                    .await?,
            "Payment authority denied"
        );
        let person = self.person(actor).await?;
        let digest = fingerprint(details)?;
        let id = Uuid::new_v4();
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("Entropy unavailable"))?;
        let nonce = fingerprint(&(id, &actor.issuer, &actor.subject, &digest, bytes))?;
        sqlx::query("INSERT INTO controls.approval_challenges(id,operation_id,issuer,subject,person_id,intent_digest,nonce,state,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,'PENDING',$8) ON CONFLICT(operation_id) DO NOTHING")
            .bind(id).bind(operation).bind(&actor.issuer).bind(&actor.subject).bind(person.0).bind(&digest).bind(&nonce).bind(self.clock.now()+Duration::minutes(3)).execute(&self.pool).await?;
        let row=sqlx::query("SELECT id FROM controls.approval_challenges WHERE operation_id=$1 AND issuer=$2 AND subject=$3 AND intent_digest=$4").bind(operation).bind(&actor.issuer).bind(&actor.subject).bind(&digest).fetch_optional(&self.pool).await?.ok_or_else(||anyhow::anyhow!("Approval operation fingerprint conflict"))?;
        self.pending_challenge(actor, row.get("id")).await
    }
    pub async fn pending_challenge(&self, actor: &ActorContext, id: Uuid) -> Result<Challenge> {
        let row=sqlx::query("SELECT * FROM controls.approval_challenges WHERE id=$1 AND issuer=$2 AND subject=$3 AND state='PENDING' AND expires_at>$4")
            .bind(id).bind(&actor.issuer).bind(&actor.subject).bind(self.clock.now()).fetch_one(&self.pool).await?;
        ensure!(
            row.get::<Uuid, _>("person_id") == self.person(actor).await?.0,
            "Identity link changed"
        );
        Ok(Challenge {
            id,
            operation_id: row.get("operation_id"),
            issuer: row.get("issuer"),
            subject: row.get("subject"),
            intent_digest: row.get("intent_digest"),
            nonce: row.get("nonce"),
            expires_at: row.get("expires_at"),
        })
    }
    pub async fn approve_payment(
        &self,
        identity: &bank_identity::IdentityService,
        actor: &ActorContext,
        approval: ApprovalRef,
        details: &PaymentDetails,
        authority: &ResourceAuthority,
        correlation: Uuid,
    ) -> Result<()> {
        let id = approval.challenge_id;
        let operation = approval.operation_id;
        ensure!(
            self.authorize(actor, Action::CreatePayment, Some(authority), correlation)
                .await?,
            "Current payment authority denied"
        );
        let digest = fingerprint(details)?;
        let row=sqlx::query("SELECT * FROM controls.approval_challenges WHERE id=$1 AND operation_id=$2 AND issuer=$3 AND subject=$4 AND intent_digest=$5 AND expires_at>$6")
            .bind(id).bind(operation).bind(&actor.issuer).bind(&actor.subject).bind(&digest).bind(self.clock.now()).fetch_one(&self.pool).await?;
        ensure!(
            row.get::<Uuid, _>("person_id") == self.person(actor).await?.0,
            "Identity link changed"
        );
        identity
            .consume_step_up(actor, id, operation, &digest)
            .await?; // Idempotent receipt survives a cross-module acknowledgement loss.
        let mut tx = self.pool.begin().await?;
        let updated=sqlx::query("UPDATE controls.approval_challenges SET state='APPROVED' WHERE id=$1 AND state='PENDING'").bind(id).execute(&mut *tx).await?;
        if updated.rows_affected() == 1 {
            AuditLog::append_in(
                &mut tx,
                id,
                "approvals",
                &AuditRecord::new(actor, "payment.approved", operation, correlation),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

impl bank_platform::events::ReplayAuthorizer for Entitlements {
    fn authorize_replay<'a>(
        &'a self,
        actor: &'a ActorContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(self.authorize(actor, Action::ReplayEvents, None, Uuid::new_v4()))
    }
}
