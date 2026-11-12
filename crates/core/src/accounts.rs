use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::entitlements::{Action, Entitlements, ResourceAuthority};
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, EventEnvelope, EventId, PartyId, fingerprint},
    events::EventStore,
};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpeningRequest {
    pub application_id: Uuid,
    pub party_id: PartyId,
    pub product_version_id: Uuid,
    pub acceptance_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationEvidence {
    pub application_id: Uuid,
    pub party_id: PartyId,
    pub product_version_id: Uuid,
    pub acceptance_id: Uuid,
    pub controls_revision: String,
    pub expires_at: DateTime<Utc>,
}
/// Implemented by the owning opening coordinator using current Controls and Documents ports.
/// Request JSON cannot implement this policy or assert a clearance.
pub trait OpeningPolicy: Send + Sync {
    fn validate<'a>(
        &'a self,
        request: &'a OpeningRequest,
    ) -> BoxFuture<'a, Result<ActivationEvidence>>;
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub id: AccountId,
    pub application_id: Uuid,
    pub product_version_id: Uuid,
    pub state: String,
    pub debit_blocked: bool,
    pub credit_blocked: bool,
    pub revision: i64,
}
impl Core {
    /// Called only after approved onboarding. The legal digest cannot be replaced silently.
    pub async fn ensure_party(
        &self,
        actor: &ActorContext,
        application: Uuid,
        identity_digest: &str,
    ) -> Result<PartyId> {
        self.ensure_party_linked(actor, application, identity_digest, None)
            .await
    }
    pub async fn ensure_party_linked(
        &self,
        actor: &ActorContext,
        application: Uuid,
        identity_digest: &str,
        person: Option<PartyId>,
    ) -> Result<PartyId> {
        ensure!(
            !actor.issuer.is_empty() && !actor.subject.is_empty() && identity_digest.len() == 64,
            "Invalid canonical identity reference"
        );
        let mut tx = self.pool.begin().await?;
        let person = PartyId(
            sqlx::query_scalar("SELECT core.ensure_party_linked($1,$2,$3,$4,$5)")
                .bind(&actor.issuer)
                .bind(&actor.subject)
                .bind(application)
                .bind(identity_digest)
                .bind(person.map(|id| id.0))
                .fetch_one(&mut *tx)
                .await?,
        );
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core.outbox WHERE aggregate_id=$1 AND aggregate_version=1)").bind(person.0).fetch_one(&mut *tx).await?;
        if !exists {
            EventStore::new(self.pool.clone(), Module::Core)
                .stage(
                    &mut tx,
                    &EventEnvelope {
                        event_id: EventId::new(),
                        event_type: "customer.created".into(),
                        schema_version: 1,
                        producer: "core".into(),
                        aggregate_id: person.0,
                        aggregate_version: 1,
                        occurred_at: Utc::now(),
                        correlation_id: application,
                        causation_id: application.to_string(),
                        data: serde_json::json!({"partyId":person}),
                    },
                )
                .await?;
        }
        tx.commit().await?;
        Ok(person)
    }
    pub async fn stage_account(&self, request: &OpeningRequest) -> Result<AccountId> {
        let mut tx = self.pool.begin().await?;
        let id = AccountId(
            sqlx::query_scalar("SELECT core.stage_account($1,$2,$3,$4,$5)")
                .bind(request.application_id)
                .bind(request.party_id.0)
                .bind(request.product_version_id)
                .bind(request.acceptance_id)
                .bind(fingerprint(request)?)
                .fetch_one(&mut *tx)
                .await?,
        );
        self.account_fact(&mut tx, id, request.application_id)
            .await?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn prepare_opening<P: OpeningPolicy>(
        &self,
        request: &OpeningRequest,
        policy: &P,
    ) -> Result<AccountId> {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT acceptance_id FROM core.account_registry WHERE application_id=$1",
        )
        .bind(request.application_id)
        .fetch_optional(&self.pool)
        .await?;
        if existing.is_none_or(|id| id == request.acceptance_id) {
            return self.stage_account(request).await;
        }
        let evidence = policy.validate(request).await?;
        let mut tx = self.pool.begin().await?;
        let account = AccountId(
            sqlx::query_scalar("SELECT core.rebind_staged_terms($1,$2,$3,$4,$5,$6)")
                .bind(request.application_id)
                .bind(request.party_id.0)
                .bind(request.product_version_id)
                .bind(request.acceptance_id)
                .bind(fingerprint(request)?)
                .bind(serde_json::to_value(evidence)?)
                .fetch_one(&mut *tx)
                .await?,
        );
        self.account_fact(&mut tx, account, request.application_id)
            .await?;
        tx.commit().await?;
        Ok(account)
    }
    pub async fn activate<P: OpeningPolicy>(
        &self,
        account: AccountId,
        request: &OpeningRequest,
        policy: &P,
    ) -> Result<()> {
        let evidence = policy.validate(request).await?;
        ensure!(
            evidence.application_id == request.application_id
                && evidence.party_id == request.party_id
                && evidence.product_version_id == request.product_version_id
                && evidence.acceptance_id == request.acceptance_id
                && evidence.expires_at > Utc::now()
                && !evidence.controls_revision.is_empty(),
            "Current independent activation gates required"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.activate_account($1,$2)")
            .bind(account.0)
            .bind(serde_json::to_value(evidence)?)
            .execute(&mut *tx)
            .await?;
        self.account_fact(&mut tx, account, request.application_id)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn authority(&self, account: AccountId, action: Action) -> Result<ResourceAuthority> {
        let row = sqlx::query("SELECT state,debit_blocked FROM core.ledger_accounts WHERE id=$1")
            .bind(account.0)
            .fetch_one(&self.pool)
            .await?;
        let related=sqlx::query("SELECT person_id,kind FROM core.account_relationships WHERE account_id=$1 AND revoked_at IS NULL").bind(account.0).fetch_all(&self.pool).await?;
        let mut owners = vec![];
        let mut staff = vec![];
        for row in related {
            let person = PartyId(row.get("person_id"));
            if row.get::<String, _>("kind") == "OWNER" {
                owners.push(person);
            } else {
                staff.push(person);
            }
        }
        let restricted = matches!(action, Action::CreatePayment)
            && (row.get::<String, _>("state") != "ACTIVE" || row.get::<bool, _>("debit_blocked"));
        Ok(ResourceAuthority {
            resource_id: account.0,
            owners,
            permitted_staff: staff,
            action_restricted: restricted,
        })
    }
    pub async fn accounts_for(&self, person: PartyId) -> Result<Vec<AccountSummary>> {
        let rows=sqlx::query("SELECT r.*,a.state,a.debit_blocked,a.credit_blocked FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id JOIN core.account_relationships x ON x.account_id=r.id WHERE x.person_id=$1 AND x.kind='OWNER' AND x.revoked_at IS NULL ORDER BY r.created_at,r.id LIMIT 100").bind(person.0).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| AccountSummary {
                id: AccountId(r.get("id")),
                application_id: r.get("application_id"),
                product_version_id: r.get("product_version_id"),
                state: r.get("state"),
                debit_blocked: r.get("debit_blocked"),
                credit_blocked: r.get("credit_blocked"),
                revision: r.get("revision"),
            })
            .collect())
    }
    pub async fn restrict(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        account: AccountId,
        debit: bool,
        credit: bool,
    ) -> Result<()> {
        let authority = self.authority(account, Action::ServiceAccount).await?;
        ensure!(
            controls
                .authorize(actor, Action::ServiceAccount, Some(&authority), account.0)
                .await?,
            "Account service denied"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.restrict_account($1,$2,$3,$4)")
            .bind(account.0)
            .bind(debit)
            .bind(credit)
            .bind(actor.trust == bank_platform::domain::TrustContext::Customer)
            .execute(&mut *tx)
            .await?;
        self.account_fact(&mut tx, account, Uuid::new_v4()).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn close_account(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        account: AccountId,
    ) -> Result<()> {
        let authority = self.authority(account, Action::ServiceAccount).await?;
        ensure!(
            controls
                .authorize(actor, Action::ServiceAccount, Some(&authority), account.0)
                .await?,
            "Account closure denied"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT core.close_account($1)")
            .bind(account.0)
            .execute(&mut *tx)
            .await?;
        self.account_fact(&mut tx, account, Uuid::new_v4()).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn obligation(
        &self,
        operation: Uuid,
        account: AccountId,
        kind: &str,
        resolved: bool,
    ) -> Result<()> {
        ensure!(
            !kind.is_empty() && kind.len() <= 64,
            "Invalid obligation kind"
        );
        sqlx::query("SELECT core.set_obligation($1,$2,$3,$4)")
            .bind(operation)
            .bind(account.0)
            .bind(kind)
            .bind(resolved)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    async fn account_fact(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        account: AccountId,
        correlation: Uuid,
    ) -> Result<()> {
        let row=sqlx::query("SELECT r.revision,a.state,a.debit_blocked,a.credit_blocked FROM core.account_registry r JOIN core.ledger_accounts a ON a.id=r.id WHERE r.id=$1 AND NOT EXISTS(SELECT 1 FROM core.outbox o WHERE o.aggregate_id=r.id AND o.aggregate_version=r.revision)").bind(account.0).fetch_optional(&mut **tx).await?;
        if let Some(row) = row {
            EventStore::new(self.pool.clone(),Module::Core).stage(tx,&EventEnvelope{event_id:EventId::new(),event_type:"account.changed".into(),schema_version:1,producer:"core".into(),aggregate_id:account.0,aggregate_version:row.get("revision"),occurred_at:Utc::now(),correlation_id:correlation,causation_id:correlation.to_string(),data:serde_json::json!({"accountId":account,"state":row.get::<String,_>("state"),"debitBlocked":row.get::<bool,_>("debit_blocked"),"creditBlocked":row.get::<bool,_>("credit_blocked")})}).await?;
        }
        Ok(())
    }
}
