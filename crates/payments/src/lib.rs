//! Payments owns intent, recipient, quote and orchestration state. Core alone owns money.
pub mod approvals;
pub mod atm;
mod audit;
pub mod batch;
mod credits;
pub mod finance;
pub use credits::CreditStatus;
mod execution;
mod finality;
mod intents;
pub mod policy;
mod quotes;
mod recipients;
mod recovery;
pub use recovery::{RecoveryAction, RecoveryRequest};
mod reconciliation;
pub mod routing;
pub use reconciliation::ReconciliationBreak;
mod submission;
mod types;
use anyhow::{Result, ensure};
use bank_controls::{
    checks::Checks,
    entitlements::{Action, Entitlements},
};
use bank_core::Core;
use bank_identity::IdentityService;
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, TrustContext},
    operations::Operations,
};
use sqlx::PgPool;
use std::sync::Arc;
pub use types::{Beneficiary, PaymentStatus, Progress, Quote};
use uuid::Uuid;
#[derive(Debug)]
pub struct ResourceUnavailable;
impl std::fmt::Display for ResourceUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Payment resource unavailable")
    }
}
impl std::error::Error for ResourceUnavailable {}
#[derive(Clone)]
pub struct Payments {
    pub pool: PgPool,
    pub core: Core,
    pub controls: Entitlements,
    pub checks: Checks,
    pub operations: Operations,
    pub identity: IdentityService,
    pub rail_client: bank_rails::RailClient,
}
impl Payments {
    /// WP-39: the client for one of the interchangeable simulated providers.
    pub(crate) fn rail(&self, provider: &str) -> Result<bank_rails::RailClient> {
        self.rail_client.with_provider(provider)
    }
    pub(crate) async fn authorize(
        &self,
        actor: &ActorContext,
        account: AccountId,
        action: Action,
    ) -> Result<()> {
        // WP-33: a consented partner acts for the owner; Controls re-checks the live grant at every decision.
        ensure!(
            matches!(actor.trust, TrustContext::Customer | TrustContext::Partner),
            "Customer payment context required"
        );
        let resource = self.core.authority(account, action).await?;
        ensure!(
            self.controls
                .authorize(actor, action, Some(&resource), account.0)
                .await?,
            ResourceUnavailable
        );
        Ok(())
    }
    /// WP-33 (FR-ACC-08): a delegate's payment amount must not exceed the delegation's per-payment limit.
    /// Owners and consented partners (who act for the owner) are bounded by the ordinary product and rail rules.
    pub(crate) async fn authorize_amount(
        &self,
        actor: &ActorContext,
        account: AccountId,
        amount: &bank_platform::domain::Money,
    ) -> Result<()> {
        ensure!(
            matches!(actor.trust, TrustContext::Customer | TrustContext::Partner),
            "Customer payment context required"
        );
        // One authority read serves both the Controls decision and the delegate limit (WP-41, CF-04).
        let resource = self.core.authority(account, Action::CreatePayment).await?;
        ensure!(
            self.controls
                .authorize(actor, Action::CreatePayment, Some(&resource), account.0)
                .await?,
            ResourceUnavailable
        );
        if actor.trust != TrustContext::Customer {
            return Ok(());
        }
        let person = self.controls.person(actor).await?;
        if resource.owners.contains(&person) {
            return Ok(());
        }
        let delegate = resource.delegate(person).ok_or(ResourceUnavailable)?;
        ensure!(
            delegate.can_pay
                && delegate
                    .payment_limit_minor
                    .is_some_and(|limit| amount.minor_units() <= limit),
            "Delegated payment limit exceeded"
        );
        Ok(())
    }
    pub async fn staff_list(&self, actor: &ActorContext) -> Result<Vec<PaymentStatus>> {
        ensure!(
            self.controls.payment_inquiry_authorized(actor).await?,
            "Current payment inquiry authority and queue required"
        );
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM payments.intents ORDER BY created_at DESC,id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut result = vec![];
        for id in ids {
            result.push(self.status_record(id).await?);
        }
        Ok(result)
    }
    pub async fn staff_detail(&self, actor: &ActorContext, id: Uuid) -> Result<PaymentStatus> {
        ensure!(
            self.controls.payment_inquiry_authorized(actor).await?,
            "Current payment inquiry authority and queue required"
        );
        self.status_record(id).await
    }
}
pub async fn local() -> Result<Arc<Payments>> {
    let controls = Module::Controls.connect().await?;
    Ok(Arc::new(Payments {
        pool: Module::Payments.connect().await?,
        core: Core::new(Module::Core.connect().await?),
        controls: Entitlements::new(controls.clone()),
        checks: Checks::new(controls)?,
        operations: Operations::new(Module::Platform.connect().await?),
        identity: IdentityService::new().await?,
        rail_client: bank_rails::RailClient::local()?,
    }))
}
