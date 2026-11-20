//! Details-bound independent approvals shared by configuration and financial corrections.
use crate::{
    audit::{AuditLog, AuditRecord},
    entitlements::{Action, Entitlements},
};
use anyhow::{Result, ensure};
use bank_platform::domain::{ActorContext, TrustContext, fingerprint};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Copy)]
pub enum ChangeKind {
    Product,
    Correction,
    Terms,
    Master,
    Rail,
    FinanceChart,
    FinanceReport,
    PeriodicPolicy,
    PeriodicOffer,
    PeriodicActivation,
    FinanceCorrection,
}
impl ChangeKind {
    fn code(self) -> &'static str {
        match self {
            Self::Product => "PRODUCT",
            Self::Terms => "TERMS",
            Self::Master => "MASTER_REFERENCE",
            Self::Correction => "CORRECTION",
            Self::Rail => "RAIL_POLICY",
            Self::FinanceChart => "FINANCE_CHART",
            Self::FinanceReport => "FINANCE_REPORT",
            Self::PeriodicPolicy => "PERIODIC_POLICY",
            Self::PeriodicOffer => "PERIODIC_OFFER",
            Self::PeriodicActivation => "PERIODIC_ACTIVATION",
            Self::FinanceCorrection => "FINANCE_CORRECTION",
        }
    }
    fn propose(self) -> Action {
        match self {
            Self::Product | Self::Terms => Action::ProposeProduct,
            Self::Correction
            | Self::FinanceChart
            | Self::FinanceReport
            | Self::PeriodicPolicy
            | Self::PeriodicOffer
            | Self::PeriodicActivation
            | Self::FinanceCorrection => Action::ProposeCorrection,
            Self::Master => Action::ProposeMaster,
            Self::Rail => Action::ManageRails,
        }
    }
    fn approve(self) -> Action {
        match self {
            Self::Product | Self::Terms => Action::ApproveProduct,
            Self::Correction
            | Self::FinanceChart
            | Self::FinanceReport
            | Self::PeriodicPolicy
            | Self::PeriodicOffer
            | Self::PeriodicActivation
            | Self::FinanceCorrection => Action::ApproveCorrection,
            Self::Master => Action::ApproveMaster,
            Self::Rail => Action::ApproveRails,
        }
    }
}
/// Cannot be deserialized or constructed by a channel. Its scope binds material details.
pub struct ApprovedChange {
    id: Uuid,
    kind: &'static str,
    resource: Uuid,
    digest: String,
    maker: Uuid,
    checker: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRecord {
    pub id: Uuid,
    pub resource_id: Uuid,
    pub digest: String,
    pub maker_person: Uuid,
    pub checker_person: Option<Uuid>,
    pub state: String,
    pub expires_at: DateTime<Utc>,
}
impl ApprovedChange {
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn people(&self) -> (Uuid, Uuid) {
        (self.maker, self.checker)
    }
    pub fn assert_bound<T: Serialize>(
        &self,
        kind: &str,
        resource: Uuid,
        details: &T,
    ) -> Result<()> {
        ensure!(
            self.kind == kind && self.resource == resource && self.digest == fingerprint(details)?,
            "Approval scope changed"
        );
        Ok(())
    }
}
impl Entitlements {
    pub async fn propose_change<T: Serialize>(
        &self,
        actor: &ActorContext,
        kind: ChangeKind,
        resource: Uuid,
        details: &T,
        until: DateTime<Utc>,
    ) -> Result<Uuid> {
        self.propose_change_id(actor, Uuid::new_v4(), kind, resource, details, until)
            .await
    }
    /// Owning modules persist this identity before crossing the Controls boundary.
    pub async fn propose_change_id<T: Serialize>(
        &self,
        actor: &ActorContext,
        id: Uuid,
        kind: ChangeKind,
        resource: Uuid,
        details: &T,
        until: DateTime<Utc>,
    ) -> Result<Uuid> {
        let until = DateTime::<Utc>::from_timestamp_micros(until.timestamp_micros())
            .ok_or_else(|| anyhow::anyhow!("Invalid approval timestamp"))?;
        ensure!(
            actor.trust == TrustContext::Workforce
                && self
                    .authorize(actor, kind.propose(), None, resource)
                    .await?,
            "Change proposal denied"
        );
        ensure!(
            until > Utc::now() && until <= Utc::now() + chrono::Duration::days(7),
            "Invalid approval lifetime"
        );
        ensure!(!id.is_nil(), "Stable proposal identity required");
        let maker = self.person(actor).await?;
        let mut tx = self.pool.begin().await?;
        let inserted=sqlx::query("INSERT INTO controls.change_proposals(id,kind,resource_id,digest,maker_person,expires_at) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING")
            .bind(id).bind(kind.code()).bind(resource).bind(fingerprint(details)?).bind(maker.0).bind(until).execute(&mut *tx).await?.rows_affected()==1;
        let row=sqlx::query("SELECT kind,resource_id,digest,maker_person,expires_at FROM controls.change_proposals WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
        ensure!(
            row.get::<String, _>("kind") == kind.code()
                && row.get::<Uuid, _>("resource_id") == resource
                && row.get::<String, _>("digest") == fingerprint(details)?
                && row.get::<Uuid, _>("maker_person") == maker.0
                && row.get::<DateTime<Utc>, _>("expires_at") == until,
            "Proposal identity conflict"
        );
        if inserted {
            AuditLog::append_in(
                &mut tx,
                id,
                "governance",
                &AuditRecord::new(actor, "change.proposed", resource, id),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(id)
    }
    pub async fn approve_change(&self, actor: &ActorContext, id: Uuid) -> Result<()> {
        let kind: String =
            sqlx::query_scalar("SELECT kind FROM controls.change_proposals WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let kind = match kind.as_str() {
            "PRODUCT" => ChangeKind::Product,
            "TERMS" => ChangeKind::Terms,
            "MASTER_REFERENCE" => ChangeKind::Master,
            "CORRECTION" => ChangeKind::Correction,
            "RAIL_POLICY" => ChangeKind::Rail,
            "FINANCE_CHART" => ChangeKind::FinanceChart,
            "FINANCE_REPORT" => ChangeKind::FinanceReport,
            "PERIODIC_POLICY" => ChangeKind::PeriodicPolicy,
            "PERIODIC_OFFER" => ChangeKind::PeriodicOffer,
            "PERIODIC_ACTIVATION" => ChangeKind::PeriodicActivation,
            "FINANCE_CORRECTION" => ChangeKind::FinanceCorrection,
            _ => anyhow::bail!("Unsupported change"),
        };
        ensure!(
            actor.trust == TrustContext::Workforce
                && self.authorize(actor, kind.approve(), None, id).await?,
            "Change approval denied"
        );
        let person = self.person(actor).await?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM controls.change_proposals WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        if matches!(
            kind,
            ChangeKind::FinanceReport
                | ChangeKind::PeriodicPolicy
                | ChangeKind::PeriodicOffer
                | ChangeKind::PeriodicActivation
                | ChangeKind::FinanceCorrection
        ) {
            let current:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.role_grants g WHERE g.person_id=$1 AND g.role='finance_approver' AND g.scope IS NULL AND g.revoked_at IS NULL AND g.valid_from<=now() AND g.valid_until>now()) AND EXISTS(SELECT 1 FROM controls.case_queue_members q WHERE q.person_id=$1 AND q.queue='finance-operations' AND q.capability='APPROVE' AND q.revoked_at IS NULL)").bind(person.0).fetch_one(&mut *tx).await?;
            ensure!(current, "Current Finance approval role and queue required");
            ensure!(
                row.get::<String, _>("state") == "PENDING"
                    || row.get::<Option<Uuid>, _>("checker_person") == Some(person.0),
                "Another checker already approved this controlled change"
            );
        }
        ensure!(
            row.get::<Uuid, _>("maker_person") != person.0
                && row.get::<DateTime<Utc>, _>("expires_at") > Utc::now(),
            "Independent unexpired checker required"
        );
        if row.get::<String, _>("state") == "PENDING" {
            sqlx::query("UPDATE controls.change_proposals SET state='APPROVED',checker_person=$2 WHERE id=$1").bind(id).bind(person.0).execute(&mut *tx).await?;
            AuditLog::append_in(
                &mut tx,
                Uuid::new_v4(),
                "governance",
                &AuditRecord::new(actor, "change.approved", id, id),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn approved_change<T: Serialize>(
        &self,
        actor: &ActorContext,
        id: Uuid,
        kind: ChangeKind,
        resource: Uuid,
        details: &T,
    ) -> Result<ApprovedChange> {
        ensure!(
            actor.trust == TrustContext::Workforce
                && self.authorize(actor, kind.propose(), None, id).await?,
            "Change execution denied"
        );
        let row=sqlx::query("SELECT * FROM controls.change_proposals WHERE id=$1 AND state='APPROVED' AND expires_at>now()").bind(id).fetch_one(&self.pool).await?;
        let maker = row.get("maker_person");
        let checker = row.get("checker_person");
        // Grant revocation invalidates pending approval use, even across login aliases.
        let role = match kind {
            ChangeKind::Product | ChangeKind::Terms => "product_approver",
            ChangeKind::Correction
            | ChangeKind::FinanceChart
            | ChangeKind::FinanceReport
            | ChangeKind::PeriodicPolicy
            | ChangeKind::PeriodicOffer
            | ChangeKind::PeriodicActivation
            | ChangeKind::FinanceCorrection => "finance_approver",
            ChangeKind::Master => "data_approver",
            ChangeKind::Rail => "payment_approver",
        };
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.role_grants WHERE person_id=$1 AND role=$2 AND scope IS NULL AND revoked_at IS NULL AND valid_from<=now() AND valid_until>now())").bind(checker).bind(role).fetch_one(&self.pool).await?;
        ensure!(
            valid && maker != checker && row.get::<String, _>("kind") == kind.code(),
            "Approval authority revoked"
        );
        let proof = ApprovedChange {
            id,
            kind: kind.code(),
            resource: row.get("resource_id"),
            digest: row.get("digest"),
            maker,
            checker,
        };
        proof.assert_bound(kind.code(), resource, details)?;
        Ok(proof)
    }
    /// Internal read port; owning domains enforce current resource inquiry authority.
    pub async fn change_record(&self, id: Uuid, kind: ChangeKind) -> Result<Option<ChangeRecord>> {
        let row = sqlx::query("SELECT * FROM controls.change_proposals WHERE id=$1 AND kind=$2")
            .bind(id)
            .bind(kind.code())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| ChangeRecord {
            id: r.get("id"),
            resource_id: r.get("resource_id"),
            digest: r.get("digest"),
            maker_person: r.get("maker_person"),
            checker_person: r.get("checker_person"),
            state: r.get("state"),
            expires_at: r.get("expires_at"),
        }))
    }
}
