use bank_core::payments::InternalInstruction;
use bank_platform::domain::{AccountId, ActorContext, Money, PaymentDetails};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Beneficiary {
    pub id: Uuid,
    pub label: String,
    pub state: String,
    pub version: i64,
    pub receiving_reference: String,
    pub rail: String,
    pub verified_at: DateTime<Utc>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub id: Uuid,
    pub payment_id: Uuid,
    pub beneficiary_id: Uuid,
    pub beneficiary_label: String,
    pub payer_account_id: AccountId,
    pub amount: Money,
    pub fee: Money,
    pub total: Money,
    pub requested_date: NaiveDate,
    pub rail: String,
    pub product_version_id: Uuid,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone)]
pub(crate) struct QuoteRecord {
    pub id: Uuid,
    pub beneficiary_id: Uuid,
    pub beneficiary_version: i64,
    pub instruction: InternalInstruction,
    pub expires_at: DateTime<Utc>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentStatus {
    pub id: Uuid,
    pub quote_id: Uuid,
    pub details: PaymentDetails,
    pub beneficiary_label: String,
    pub total: Money,
    pub state: String,
    pub reason_code: String,
    pub returns: bank_core::credit::ReturnSummary,
    pub cancellation: Option<crate::RecoveryRequest>,
    pub allowed_actions: Vec<String>,
    pub version: i64,
    pub operation_id: Option<Uuid>,
    pub batch_id: Option<Uuid>,
    pub challenge_id: Option<Uuid>,
    pub approval_url: Option<String>,
    pub approval_ready: bool,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
pub(crate) struct Intent {
    pub id: Uuid,
    /// Effective actor for execution: the authorizing customer when one is recorded, else the initiator.
    pub actor: ActorContext,
    /// Who drafted the intent (a partner client for partner-initiated payments). Immutable evidence.
    pub initiator: ActorContext,
    pub quote_id: Uuid,
    pub challenge: Option<Uuid>,
    pub state: String,
}
#[derive(Serialize)]
#[serde(tag = "progress", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Progress {
    Done { state: String },
    Pending,
    Review { code: String },
}
