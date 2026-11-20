//! Narrow independently approved correction capability for synthetic funding only.
use crate::{
    Core,
    finance::{FinanceRecords, FinancialBatch},
};
use anyhow::{Result, ensure};
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_platform::domain::{ActorContext, Currency, fingerprint};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Debug)]
pub struct CorrectionConflict;
impl std::fmt::Display for CorrectionConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Financial correction scope, source or state conflict")
    }
}
impl std::error::Error for CorrectionConflict {}
#[derive(Debug)]
pub struct CorrectionInputError;
impl std::fmt::Display for CorrectionInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Valid exact synthetic funding correction required")
    }
}
impl std::error::Error for CorrectionInputError {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionSource {
    pub batch: FinancialBatch,
    pub journal_fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionLeg {
    pub account_id: Uuid,
    pub account_kind: String,
    pub original_minor: String,
    pub reversing_minor: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionDetails {
    pub format_version: u8,
    pub kind: String,
    pub label: String,
    pub execution_id: Uuid,
    pub original_batch_id: Uuid,
    pub source_operation_id: Uuid,
    pub source_business_date: NaiveDate,
    pub source_digest: String,
    pub source_fingerprint: String,
    pub currency: Currency,
    pub legs: Vec<CorrectionLeg>,
    pub reason: String,
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl CorrectionSource {
    pub fn details(&self, execution: Uuid, reason: &str) -> Result<CorrectionDetails> {
        let j = &self.batch.journal;
        ensure!(
            !execution.is_nil()
                && execution != j.id
                && (4..=240).contains(&reason.chars().count())
                && reason.trim() == reason
                && !reason.chars().any(char::is_control),
            CorrectionInputError
        );
        ensure!(
            ["SYNTHETIC_FUNDING", "PERIODIC_FEE", "PERIODIC_ACCRUAL"]
                .contains(&j.posting_kind.as_str())
                && j.original_batch_id.is_none()
                && j.entries.len() == 2
                && digest(&self.journal_fingerprint),
            CorrectionInputError
        );
        let mut legs = vec![];
        let mut total = 0_i128;
        let mut customer = 0;
        let mut settlement = 0;
        for entry in &j.entries {
            let value = entry.amount.minor_units();
            let kind = self
                .batch
                .account_kinds
                .get(&entry.account_id.0)
                .ok_or(CorrectionInputError)?;
            match kind.as_str() {
                "CUSTOMER"
                    if (j.posting_kind == "PERIODIC_FEE" && value < 0)
                        || (j.posting_kind != "PERIODIC_FEE" && value > 0) =>
                {
                    customer += 1
                }
                "SETTLEMENT" if j.posting_kind == "SYNTHETIC_FUNDING" && value < 0 => {
                    settlement += 1
                }
                "CONTROL"
                    if (j.posting_kind == "PERIODIC_FEE" && value > 0)
                        || (j.posting_kind == "PERIODIC_ACCRUAL" && value < 0) =>
                {
                    settlement += 1
                }
                _ => anyhow::bail!(CorrectionInputError),
            }
            total += i128::from(value);
            legs.push(CorrectionLeg {
                account_id: entry.account_id.0,
                account_kind: kind.clone(),
                original_minor: value.to_string(),
                reversing_minor: value.checked_neg().ok_or(CorrectionInputError)?.to_string(),
            });
        }
        ensure!(
            customer == 1
                && settlement == 1
                && total == 0
                && legs.windows(2).all(|w| w[0].account_id < w[1].account_id),
            CorrectionInputError
        );
        Ok(CorrectionDetails {
            format_version: 1,
            kind: if j.posting_kind == "SYNTHETIC_FUNDING" {
                "FULL_SYNTHETIC_FUNDING_REVERSAL_V1"
            } else {
                "FULL_PERIODIC_REVERSAL_V1"
            }
            .into(),
            label: "SYNTHETIC".into(),
            execution_id: execution,
            original_batch_id: j.id,
            source_operation_id: j.operation_id,
            source_business_date: j.business_date,
            source_digest: fingerprint(&self.batch)?,
            source_fingerprint: self.journal_fingerprint.clone(),
            currency: Currency::synthetic(),
            legs,
            reason: reason.into(),
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionReceipt {
    pub execution_id: Uuid,
    pub proposal_id: Uuid,
    pub original_batch_id: Uuid,
    pub batch_id: Uuid,
    pub details_digest: String,
    pub maker_person: Uuid,
    pub checker_person: Uuid,
    pub recorded_at: DateTime<Utc>,
    pub financial_batch: FinancialBatch,
}
#[derive(Clone)]
pub struct CorrectionCommands {
    pub(crate) pool: PgPool,
}
impl Core {
    pub fn finance_corrections(&self) -> CorrectionCommands {
        CorrectionCommands {
            pool: self.pool.clone(),
        }
    }
}
impl CorrectionCommands {
    pub async fn source(&self, id: Uuid) -> Result<CorrectionSource> {
        let batch = Core::new(self.pool.clone()).financial_batch(id).await?;
        if ["PERIODIC_FEE", "PERIODIC_ACCRUAL"].contains(&batch.journal.posting_kind.as_str()) {
            let id: Uuid = sqlx::query_scalar(
                "SELECT finalization_id FROM core.periodic_postings WHERE batch_id=$1",
            )
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
            let _: serde_json::Value =
                sqlx::query_scalar("SELECT core.periodic_period_receipt($1)")
                    .bind(id)
                    .fetch_one(&self.pool)
                    .await?;
        }
        let journal_fingerprint =
            sqlx::query_scalar("SELECT fingerprint FROM core.journal_batches WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let source = CorrectionSource {
            batch,
            journal_fingerprint,
        };
        source.details(Uuid::new_v4(), "Validate eligible synthetic funding source")?;
        Ok(source)
    }
    pub async fn corrected(&self, id: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core.journal_batches WHERE original_batch=$1 AND posting_kind='CORRECTION')").bind(id).fetch_one(&self.pool).await?)
    }
    pub async fn receipt(
        &self,
        execution: Uuid,
        details: &CorrectionDetails,
    ) -> Result<Option<CorrectionReceipt>> {
        let Some(r) =
            sqlx::query("SELECT * FROM core.finance_correction_receipts WHERE execution_id=$1")
                .bind(execution)
                .fetch_optional(&self.pool)
                .await?
        else {
            return Ok(None);
        };
        let stored: CorrectionDetails = serde_json::from_value(r.get("details"))?;
        let scope = fingerprint(details)?;
        ensure!(
            fingerprint(&stored)? == scope
                && scope == r.get::<String, _>("details_digest")
                && execution == details.execution_id
                && details.original_batch_id == r.get::<Uuid, _>("original_batch_id"),
            CorrectionConflict
        );
        let source = self.source(details.original_batch_id).await?;
        ensure!(
            fingerprint(&source.details(execution, &details.reason)?)? == scope,
            CorrectionConflict
        );
        let batch_id = r.get("batch_id");
        let journal = Core::new(self.pool.clone())
            .financial_batch(batch_id)
            .await?;
        ensure!(
            journal.journal.operation_id == execution
                && journal.journal.posting_kind == "CORRECTION"
                && journal.journal.original_batch_id == Some(details.original_batch_id)
                && journal.journal.entries.len() == details.legs.len(),
            CorrectionConflict
        );
        for (entry, leg) in journal.journal.entries.iter().zip(&details.legs) {
            ensure!(
                entry.account_id.0 == leg.account_id
                    && entry.amount.minor_units().to_string() == leg.reversing_minor
                    && entry.amount.currency() == details.currency,
                CorrectionConflict
            );
        }
        let maker = r.get("maker_person");
        let checker = r.get("checker_person");
        ensure!(maker != checker, CorrectionConflict);
        Ok(Some(CorrectionReceipt {
            execution_id: execution,
            proposal_id: r.get("approval_id"),
            original_batch_id: details.original_batch_id,
            batch_id,
            details_digest: scope,
            maker_person: maker,
            checker_person: checker,
            recorded_at: r.get("recorded_at"),
            financial_batch: journal,
        }))
    }
    pub async fn execute(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        proposal: Uuid,
        details: &CorrectionDetails,
    ) -> Result<CorrectionReceipt> {
        ensure!(
            std::env::var("BANK_PROFILE").unwrap_or_else(|_| "local".into()) == "local",
            "Synthetic financial corrections are local only"
        );
        ensure!(
            controls.finance_authorized(actor, true).await?,
            "Current Finance maker authority required"
        );
        let maker = controls.person(actor).await?.0;
        if let Some(saved) = self.receipt(details.execution_id, details).await? {
            ensure!(
                saved.proposal_id == proposal && saved.maker_person == maker,
                CorrectionConflict
            );
            return Ok(saved);
        }
        let source = self.source(details.original_batch_id).await?;
        ensure!(
            fingerprint(&source.details(details.execution_id, &details.reason)?)?
                == fingerprint(details)?,
            CorrectionConflict
        );
        let approval = controls
            .approved_change(
                actor,
                proposal,
                ChangeKind::FinanceCorrection,
                details.original_batch_id,
                details,
            )
            .await?;
        let (approved_maker, checker) = approval.people();
        ensure!(maker == approved_maker, CorrectionConflict);
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM core.execute_finance_correction($1,$2,$3,$4,$5)")
            .bind(serde_json::to_value(details)?)
            .bind(fingerprint(details)?)
            .bind(proposal)
            .bind(maker)
            .bind(checker)
            .fetch_one(&mut *tx)
            .await?;
        if row.get("created") {
            Core::new(self.pool.clone())
                .posted(&mut tx, row.get("batch_id"), details.execution_id)
                .await?;
        }
        tx.commit().await?;
        self.receipt(details.execution_id, details)
            .await?
            .ok_or_else(|| CorrectionConflict.into())
    }
}
