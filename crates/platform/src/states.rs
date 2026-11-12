use crate::domain::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentState {
    Draft,
    PendingApproval,
    Validating,
    ReviewRequired,
    Rejected,
    FundsReserved,
    Submitting,
    Submitted,
    Reconciling,
    Settled,
    Failed,
    Cancelled,
    ReturnPending,
    Returned,
}

/// Trusted application evidence, not request-body flags. Financial side effects
/// must be committed/query-confirmed before their evidence flag is populated.
#[derive(Default, Clone, Copy)]
pub struct PaymentEvidence {
    pub approval_current: bool,
    pub gates_current: bool,
    pub hold_committed: bool,
    pub submission_intent_durable: bool,
    pub provider_accepted: bool,
    pub finality_confirmed: bool,
    pub core_booked: bool,
    pub internal_transfer: bool,
    pub non_execution_proven: bool,
    pub hold_released: bool,
}

impl PaymentState {
    pub fn transition(self, next: Self, e: PaymentEvidence) -> Result<Self, DomainError> {
        use PaymentState::*;
        let allowed = match (self, next) {
            (Draft, PendingApproval | Validating) => true,
            (PendingApproval, Validating) => e.approval_current,
            (PendingApproval | Validating, Rejected) => true,
            (Validating, ReviewRequired) => true,
            (ReviewRequired, Validating) => e.gates_current,
            (Validating, FundsReserved) => e.gates_current && e.hold_committed,
            (FundsReserved, Submitting) => e.submission_intent_durable,
            (FundsReserved, Settled) => e.internal_transfer && e.core_booked && e.gates_current,
            (Submitting | Reconciling, Submitted) => e.provider_accepted,
            (Submitting | Submitted, Reconciling) => true,
            (Submitted | Reconciling, Settled) => e.finality_confirmed && e.core_booked,
            (Submitting | Submitted | Reconciling, Failed) => {
                e.non_execution_proven && e.hold_released
            }
            (Draft | PendingApproval | Validating | ReviewRequired, Cancelled) => {
                e.non_execution_proven
            }
            (FundsReserved | Submitted | Reconciling, Cancelled) => {
                e.non_execution_proven && e.hold_released
            }
            (Submitted, Submitted) => true, // cancellation requested; state stays pending
            (Settled, ReturnPending) => e.finality_confirmed,
            (ReturnPending, Returned) => e.core_booked,
            (ReturnPending, ReturnPending) => true,
            _ => false,
        };
        if allowed {
            Ok(next)
        } else {
            Err(DomainError::InvalidTransition)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApplicationState {
    Draft,
    Submitted,
    ChecksPending,
    InformationRequired,
    ReviewRequired,
    Approved,
    Declined,
    Withdrawn,
    Expired,
}
#[derive(Default)]
pub struct ApplicationEvidence {
    pub fields_complete: bool,
    pub consent_current: bool,
    pub all_checks_current: bool,
    pub party_linked: bool,
    pub authorized_decision: bool,
}
impl ApplicationState {
    pub fn transition(self, next: Self, e: ApplicationEvidence) -> Result<Self, DomainError> {
        use ApplicationState::*;
        let allowed = match (self, next) {
            (Draft, Draft) => true,
            (Draft, Submitted) => e.fields_complete && e.consent_current,
            (Submitted, ChecksPending) => true,
            (ChecksPending, ChecksPending | InformationRequired | ReviewRequired) => true,
            (InformationRequired, ChecksPending) => e.fields_complete,
            (ReviewRequired, InformationRequired | ChecksPending | Declined) => {
                e.authorized_decision
            }
            (ChecksPending, Approved) => e.all_checks_current && e.party_linked,
            (ChecksPending, Declined) => e.authorized_decision,
            (Draft | InformationRequired | ChecksPending | ReviewRequired, Withdrawn) => {
                e.authorized_decision
            }
            (Draft | InformationRequired, Expired) => e.authorized_decision,
            (Approved, ChecksPending) => true,
            _ => false,
        };
        if allowed {
            Ok(next)
        } else {
            Err(DomainError::InvalidTransition)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AccountState {
    PendingActivation,
    Active,
    Restricted,
    ClosurePending,
    Closed,
    OpeningFailed,
}
#[derive(Default)]
pub struct AccountEvidence {
    pub gates_current: bool,
    pub terms_current: bool,
    pub authorized: bool,
    pub zero_balance: bool,
    pub no_obligations: bool,
}
impl AccountState {
    pub fn transition(self, next: Self, e: AccountEvidence) -> Result<Self, DomainError> {
        use AccountState::*;
        let allowed = match (self, next) {
            (PendingActivation, Active) => e.authorized && e.gates_current && e.terms_current,
            (PendingActivation, PendingActivation) => e.authorized,
            (PendingActivation, OpeningFailed) => {
                e.authorized && e.zero_balance && e.no_obligations
            }
            (Active, Restricted | ClosurePending) | (Restricted, ClosurePending) => e.authorized,
            (Restricted, Active) => e.authorized && e.gates_current,
            (ClosurePending, Active) => e.authorized && e.gates_current,
            (ClosurePending, Restricted) => e.authorized,
            (ClosurePending, Closed) => {
                e.authorized && e.gates_current && e.zero_balance && e.no_obligations
            }
            _ => false,
        };
        if allowed {
            Ok(next)
        } else {
            Err(DomainError::InvalidTransition)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledgement_and_unknown_outcome_cannot_settle_or_release() {
        use PaymentState::*;
        assert!(
            Submitting
                .transition(
                    Submitted,
                    PaymentEvidence {
                        provider_accepted: true,
                        ..Default::default()
                    }
                )
                .is_ok()
        );
        for state in [Submitting, Submitted, Reconciling] {
            assert!(
                state
                    .transition(Settled, PaymentEvidence::default())
                    .is_err()
            );
            assert!(
                state
                    .transition(Failed, PaymentEvidence::default())
                    .is_err()
            );
            assert!(
                state
                    .transition(Cancelled, PaymentEvidence::default())
                    .is_err()
            );
        }
        assert!(
            Reconciling
                .transition(
                    Settled,
                    PaymentEvidence {
                        finality_confirmed: true,
                        core_booked: true,
                        ..Default::default()
                    }
                )
                .is_ok()
        );
        assert!(
            Reconciling
                .transition(
                    Failed,
                    PaymentEvidence {
                        non_execution_proven: true,
                        hold_released: true,
                        ..Default::default()
                    }
                )
                .is_ok()
        );
        assert!(
            Settled
                .transition(
                    Failed,
                    PaymentEvidence {
                        non_execution_proven: true,
                        hold_released: true,
                        ..Default::default()
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn execution_requires_current_gates_and_durable_financial_evidence() {
        use PaymentState::*;
        assert!(
            Validating
                .transition(
                    FundsReserved,
                    PaymentEvidence {
                        gates_current: true,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert!(
            Validating
                .transition(
                    FundsReserved,
                    PaymentEvidence {
                        gates_current: true,
                        hold_committed: true,
                        ..Default::default()
                    }
                )
                .is_ok()
        );
        assert!(
            FundsReserved
                .transition(Submitting, PaymentEvidence::default())
                .is_err()
        );
        assert!(
            FundsReserved
                .transition(
                    Settled,
                    PaymentEvidence {
                        core_booked: true,
                        internal_transfer: true,
                        gates_current: true,
                        ..Default::default()
                    }
                )
                .is_ok()
        );
        assert!(
            Returned
                .transition(Submitting, PaymentEvidence::default())
                .is_err()
        );
    }
    #[test]
    fn application_and_account_gates_block_incomplete_or_unfunded_closure() {
        assert!(
            ApplicationState::Draft
                .transition(ApplicationState::Submitted, ApplicationEvidence::default())
                .is_err()
        );
        assert!(
            ApplicationState::ChecksPending
                .transition(
                    ApplicationState::Approved,
                    ApplicationEvidence {
                        all_checks_current: true,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert!(
            AccountState::PendingActivation
                .transition(
                    AccountState::Active,
                    AccountEvidence {
                        authorized: true,
                        gates_current: true,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert!(
            AccountState::ClosurePending
                .transition(
                    AccountState::Closed,
                    AccountEvidence {
                        authorized: true,
                        gates_current: true,
                        zero_balance: true,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert!(
            AccountState::Closed
                .transition(
                    AccountState::Active,
                    AccountEvidence {
                        authorized: true,
                        gates_current: true,
                        terms_current: true,
                        ..Default::default()
                    }
                )
                .is_err()
        );
    }
}
