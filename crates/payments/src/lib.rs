//! Payments owns intent, recipient, quote and orchestration state. Core alone owns money.
mod audit;
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
    pub(crate) async fn authorize(
        &self,
        actor: &ActorContext,
        account: AccountId,
        action: Action,
    ) -> Result<()> {
        ensure!(
            actor.trust == TrustContext::Customer,
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
