//! Finance observes Core sources, exports GL and coordinates exact approved synthetic funding corrections.
mod audit;
pub mod basis;
pub mod chart;
pub mod client;
pub mod corrections;
pub mod credit_reconciliation;
pub mod credit_settlement;
mod exports;
pub mod group_reconciliation;
pub mod liquidity;
pub mod periodic;
pub mod protocol;
pub mod reconciliation;
mod recovery;
pub mod reports;
pub mod revisions;
pub mod settlement;
pub mod settlement_set;
use bank_controls::entitlements::Entitlements;
use bank_core::{Core, finance::FinanceRecords};
use bank_platform::{
    database::Module,
    domain::{ActorContext, TrustContext},
    operations::Operations,
};
pub use exports::{ExportStatus, Progress};
pub use recovery::RetryCommand;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct Finance {
    pub pool: PgPool,
    pub core: Arc<dyn FinanceRecords>,
    pub periodic: bank_core::accruals::PolicyRegistry,
    pub bases: bank_core::accruals::basis::BasisRecords,
    pub corrections: bank_core::corrections::CorrectionCommands,
    pub payments: bank_payments::finance::FinanceReader,
    pub controls: Entitlements,
    pub operations: Operations,
    pub client: client::GlClient,
}
pub(crate) fn service_actor() -> ActorContext {
    ActorContext {
        issuer: "urn:piggybank:finance".into(),
        subject: "gl-export".into(),
        client_id: "bank-worker".into(),
        trust: TrustContext::Service,
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
pub async fn local() -> anyhow::Result<Arc<Finance>> {
    Ok(Arc::new(Finance {
        pool: Module::Finance.connect().await?,
        core: Arc::new(Core::new(Module::Core.connect().await?)),
        periodic: Core::new(Module::Core.connect().await?).periodic_policies(),
        bases: Core::new(Module::Core.connect().await?).periodic_bases(),
        corrections: Core::new(Module::Core.connect().await?).finance_corrections(),
        payments: bank_payments::finance::FinanceReader::new(Module::Payments.connect().await?),
        controls: Entitlements::new(Module::Controls.connect().await?),
        operations: Operations::new(Module::Platform.connect().await?),
        client: client::GlClient::local()?,
    }))
}
