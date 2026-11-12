//! Frozen outgoing instruction / clearing file / Core / GL matching. No financial commands.
use crate::settlement::ClearingRows;
use crate::{
    Finance,
    reports::ReportItem,
    service_actor,
    settlement::{SettlementReply, SettlementRow},
};
use anyhow::{Result, ensure};
use bank_controls::cases::{CaseKind, Cases, NewCase};
use bank_core::{
    finance_snapshot::SnapshotLimitExceeded,
    payment_accounting::{PaymentAccountingFact, PaymentAccountingSnapshot},
    payments::InternalInstruction,
};
use bank_payments::finance::{PaymentLineage, PaymentLineageSnapshot};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{ActorContext, fingerprint},
};
use bank_rails::{RailEvidence, RailInstruction, RailStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
#[derive(Debug)]
pub struct ReconciliationKeyConflict;
impl std::fmt::Display for ReconciliationKeyConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reconciliation key binds another report or clearing file")
    }
}
impl std::error::Error for ReconciliationKeyConflict {}
#[derive(Debug)]
pub struct ReconciliationInputError;
impl std::fmt::Display for ReconciliationInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Invalid reconciliation scope or page")
    }
}
impl std::error::Error for ReconciliationInputError {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconciliationInput {
    pub report_id: Uuid,
    pub file_id: Uuid,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderTotal {
    pub currency_code: String,
    pub currency_scale: u8,
    pub principal: String,
    pub provider_fee: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconciliationSummary {
    pub id: Uuid,
    pub scope: String,
    pub format_version: i32,
    pub report_id: Uuid,
    pub report_digest: String,
    pub file_id: Uuid,
    pub file_digest: String,
    pub core_digest: String,
    pub payments_digest: String,
    pub items_digest: String,
    pub report_as_of: DateTime<Utc>,
    pub core_as_of: DateTime<Utc>,
    pub payments_as_of: DateTime<Utc>,
    pub file_generated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub record_count: i32,
    pub matched_count: i32,
    pub exception_count: i32,
    pub timing_count: i32,
    pub pending_count: i32,
    pub not_posted_count: i32,
    pub core_principal_total: String,
    pub core_bank_fee_total: String,
    pub matched_principal_total: String,
    pub provider_totals: Vec<ProviderTotal>,
    pub completeness: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconciliationItem {
    pub ordinal: i32,
    pub payment_id: Uuid,
    pub classification: String,
    pub issue_codes: Vec<String>,
    pub instruction: Option<InternalInstruction>,
    pub instruction_digest: Option<String>,
    pub payment_state: Option<String>,
    pub core_outcome: Option<String>,
    pub core_principal: Option<bank_platform::domain::Money>,
    pub core_bank_fee: Option<bank_platform::domain::Money>,
    pub core_batch_id: Option<Uuid>,
    pub journal_batch_ids: Vec<Uuid>,
    pub gl_export_ids: Vec<Uuid>,
    pub provider_rows: Vec<SettlementRow>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationIssue {
    pub id: Uuid,
    pub ordinal: i32,
    pub code: String,
    pub case_id: Option<Uuid>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationDetail {
    pub reconciliation: ReconciliationSummary,
    pub digest: String,
    pub issues: Vec<ReconciliationIssue>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationPage {
    pub items: Vec<ReconciliationItem>,
    pub total: i32,
    pub next_after: Option<i32>,
    pub reconciliation_digest: String,
}

fn evidence(packet: &Value, instruction: &RailInstruction) -> Result<RailEvidence> {
    let p: SignedDocument = serde_json::from_value(packet.clone())?;
    p.verify_integrity(bank_rails::signing_key().as_bytes())?;
    let value: RailEvidence = serde_json::from_value(p.body)?;
    value.validate_for(instruction)?;
    Ok(value)
}
pub(crate) struct Inspection<'a> {
    pub payment: Option<&'a PaymentLineage>,
    pub core: Option<&'a PaymentAccountingFact>,
    pub sources: Vec<&'a ReportItem>,
    pub file: ClearingRows<'a, SettlementRow>,
    pub core_as_of: DateTime<Utc>,
    pub payments_as_of: DateTime<Utc>,
    pub duplicate_lines: &'a BTreeSet<Uuid>,
}
pub(crate) fn inspect(id: Uuid, ordinal: i32, scope: Inspection<'_>) -> Result<ReconciliationItem> {
    let Inspection {
        payment,
        core,
        sources,
        file,
        core_as_of,
        payments_as_of,
        duplicate_lines,
    } = scope;
    let rows = file
        .rows
        .iter()
        .filter(|r| r.payment_id == id)
        .cloned()
        .collect::<Vec<_>>();
    let mut codes = BTreeSet::<String>::new();
    macro_rules! add {
        ($code:expr) => {{
            codes.insert(($code).into());
        }};
    }
    let quote =
        payment.and_then(|p| serde_json::from_value::<InternalInstruction>(p.quote.clone()).ok());
    let mut rail = None;
    if let Some(p) = payment {
        if fingerprint(&p.quote)? != p.quote_digest
            || quote.as_ref().is_none_or(|q| {
                q.operation_id != id
                    || q.details.quote_id != p.quote_id.to_string()
                    || q.details.beneficiary_id != p.beneficiary_id.to_string()
                    || q.details.payer_account_id == q.payee_account_id
                    || q.details.amount.require_positive().is_err()
                    || q.details.fee.minor_units() < 0
                    || q.details.amount.checked_add(q.details.fee).is_err()
                    || !matches!(q.details.rail.as_str(), "INTERNAL" | "SIMULATED")
            })
        {
            add!("INVALID_PAYMENT_INSTRUCTION");
        }
    } else {
        add!(if core.is_some_and(|c| c.recorded_at > payments_as_of) {
            "SOURCE_SNAPSHOT_TIMING"
        } else {
            "MISSING_PAYMENT_INSTRUCTION"
        });
    }
    if let (Some(p), Some(q)) = (payment, quote.as_ref()) {
        if q.details.rail == "SIMULATED" {
            if let Some(value) = &p.transport {
                match serde_json::from_value::<RailInstruction>(value.clone()) {
                    Ok(r)
                        if r.validate().is_ok()
                            && Some(fingerprint(&r)?) == p.transport_digest
                            && r.instruction_id == id
                            && r.input_digest == fingerprint(q)?
                            && r.amount == q.details.amount
                            && r.requested_date == q.details.requested_date
                            && r.beneficiary_reference == p.receiving_reference =>
                    {
                        rail = Some(r);
                    }
                    _ => add!("PROVIDER_INSTRUCTION_MISMATCH"),
                }
            } else if !rows.is_empty() || core.is_some_and(|c| c.outcome == "SETTLED") {
                add!("MISSING_PROVIDER_INSTRUCTION");
            }
        } else if p.transport.is_some() || !p.receipts.is_empty() || !rows.is_empty() {
            add!("UNEXPECTED_PROVIDER_RECORD");
        }
    }
    let mut settled_receipt = None;
    if let Some(p) = payment {
        if p.receipts.len() > 1 {
            add!("DUPLICATE_PROVIDER_FINALITY");
        }
        for packet in &p.receipts {
            match rail.as_ref().and_then(|r| evidence(packet, r).ok()) {
                Some(value) if value.status == RailStatus::Settled => {
                    settled_receipt = Some(value);
                }
                Some(value)
                    if matches!(value.status, RailStatus::Cancelled | RailStatus::Rejected) =>
                {
                    if core.is_some_and(|c| c.outcome == "SETTLED") || !rows.is_empty() {
                        add!("PROVIDER_FINALITY_CONFLICT");
                    }
                }
                _ => add!("PROVIDER_RECEIPT_INVALID"),
            }
        }
    }
    let mut core_evidence = None;
    if let Some(c) = core {
        if c.rail == "SIMULATED" && c.evidence_id.is_some() {
            match c.packet.as_ref().and_then(|p| {
                (|| -> Result<RailEvidence> {
                    let packet: SignedDocument = serde_json::from_value(p.clone())?;
                    packet.verify_integrity(bank_rails::signing_key().as_bytes())?;
                    let e: RailEvidence = serde_json::from_value(packet.body)?;
                    e.validate_binding(
                        c.payment_id,
                        &c.request_digest,
                        c.principal,
                        c.requested_date,
                    )?;
                    Ok(e)
                })()
                .ok()
            }) {
                Some(value)
                    if Some(value.evidence_id) == c.evidence_id
                        && Some(fingerprint(&value)?) == c.evidence_digest =>
                {
                    core_evidence = Some(value.clone());
                    let expected = match value.status {
                        RailStatus::Settled => "SETTLED",
                        RailStatus::Cancelled => "CANCELLED",
                        RailStatus::Rejected => "DECLINED",
                        _ => "INVALID",
                    };
                    if c.outcome != expected {
                        add!("CORE_PROVIDER_OUTCOME_MISMATCH");
                    }
                    if let Some(other) = &settled_receipt
                        && other.evidence_id != value.evidence_id
                    {
                        add!("CORE_PROVIDER_EVIDENCE_MISMATCH");
                    }
                }
                _ => add!("CORE_PROVIDER_RECEIPT_INVALID"),
            }
        } else if c.rail == "SIMULATED" && c.outcome == "SETTLED" {
            add!("MISSING_CORE_PROVIDER_EVIDENCE");
        }
        if let Some(q) = &quote {
            if c.request_digest != fingerprint(q)? {
                add!("CORE_REQUEST_DIGEST_MISMATCH");
            }
            if c.rail != q.details.rail || c.principal.currency() != q.details.amount.currency() {
                add!("CORE_CURRENCY_OR_RAIL_MISMATCH");
            }
            if c.payer_account_id != q.details.payer_account_id
                || (c.outcome == "SETTLED" && c.payee_account_id != Some(q.payee_account_id))
            {
                add!("CORE_ACCOUNT_MISMATCH");
            }
            if c.principal != q.details.amount {
                add!("CORE_PRINCIPAL_MISMATCH");
            }
            if c.fee != q.details.fee {
                add!("CORE_BANK_FEE_MISMATCH");
            }
            if c.requested_date != q.details.requested_date
                || c.product_version_id != q.product_version_id
            {
                add!("CORE_POLICY_OR_DATE_MISMATCH");
            }
        }
        if c.outcome == "SETTLED" {
            if c.batch_id.is_none() {
                add!("MISSING_CORE_JOURNAL");
            }
            if let Some(p) = payment
                && (p.batch_id != c.batch_id || p.state != "SETTLED")
            {
                add!(if p.batch_id.is_none() && p.state != "SETTLED" {
                    "PAYMENT_ACK_TIMING"
                } else {
                    "PAYMENT_RECEIPT_MISMATCH"
                });
            }
        } else if c.batch_id.is_some() || !sources.is_empty() {
            add!("UNEXPECTED_CORE_JOURNAL");
        }
    } else if !sources.is_empty()
        || !rows.is_empty()
        || payment.is_some_and(|p| p.state == "SETTLED")
    {
        add!(if payment.is_some_and(|p| p.updated_at > core_as_of) {
            "SOURCE_SNAPSHOT_TIMING"
        } else {
            "MISSING_CORE_PAYMENT"
        });
    }
    if sources.len() > 1 {
        add!("DUPLICATE_CORE_JOURNAL");
    }
    for source in &sources {
        if core.is_none_or(|c| Some(source.source.journal.id) != c.batch_id) {
            add!("CORE_JOURNAL_RECEIPT_MISMATCH");
        }
        if let Some(q) = &quote {
            let expected_kind = if q.details.rail == "SIMULATED" {
                "EXTERNAL_PAYMENT"
            } else {
                "INTERNAL_PAYMENT"
            };
            if source.source.journal.posting_kind != expected_kind {
                add!("CORE_POSTING_KIND_MISMATCH");
            }
            let mut expected = BTreeMap::new();
            if let Ok(total) = q
                .details
                .amount
                .checked_add(q.details.fee)
                .and_then(|n| n.checked_neg())
            {
                expected.insert(q.details.payer_account_id.0, total);
                expected.insert(q.payee_account_id.0, q.details.amount);
                if q.details.fee.minor_units() > 0 {
                    expected.insert(
                        Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000004")?,
                        q.details.fee,
                    );
                }
                let actual = source
                    .source
                    .journal
                    .entries
                    .iter()
                    .map(|e| (e.account_id.0, e.amount))
                    .collect::<BTreeMap<_, _>>();
                if expected != actual {
                    add!("CORE_JOURNAL_LEGS_MISMATCH");
                }
            } else {
                add!("CORE_JOURNAL_LEGS_MISMATCH");
            }
        }
        if let Some(c) = core
            && c.business_date != Some(source.source.journal.business_date)
        {
            add!("CORE_JOURNAL_DATE_MISMATCH");
        }
        if source.classification != "MATCHED" {
            if source.issue_codes.iter().any(|c| c == "GL_TIMING_WINDOW") {
                add!("GL_TIMING_WINDOW");
            } else {
                for code in &source.issue_codes {
                    add!(code);
                }
            }
            if source.issue_codes.is_empty() {
                add!("GL_NOT_MATCHED");
            }
        }
    }
    if let Some(c) = core.filter(|c| c.outcome == "SETTLED")
        && sources.is_empty()
    {
        add!(if c.batch_id.is_none() || c.business_date.is_none() {
            "MISSING_CORE_JOURNAL"
        } else if c
            .business_date
            .is_some_and(|d| d < file.range.from_date || d >= file.range.to_date)
        {
            "SETTLED_OUTSIDE_REPORT_PERIOD"
        } else {
            "CORE_AFTER_REPORT"
        });
    }
    let external = quote
        .as_ref()
        .map(|q| q.details.rail == "SIMULATED")
        .unwrap_or_else(|| core.is_some_and(|c| c.rail == "SIMULATED"));
    if external && (core.is_some_and(|c| c.outcome == "SETTLED") || settled_receipt.is_some()) {
        if settled_receipt.is_none() {
            add!(if core_evidence.is_some() {
                "PAYMENT_ACK_TIMING"
            } else {
                "MISSING_PROVIDER_FINALITY"
            });
        }
        if rows.is_empty() {
            add!(if settled_receipt
                .as_ref()
                .or(core_evidence.as_ref())
                .is_some_and(|e| e.occurred_at > file.generated_at)
            {
                "SETTLED_AFTER_FILE"
            } else {
                "MISSING_PROVIDER_SETTLEMENT"
            });
        }
    }
    if rows.len() > 1 || rows.iter().any(|r| duplicate_lines.contains(&r.line_id)) {
        add!("DUPLICATE_PROVIDER_LINE");
    }
    for row in &rows {
        if row.currency_code != "TST" || row.currency_scale != 2 {
            add!("PROVIDER_CURRENCY_MISMATCH");
        }
        if quote
            .as_ref()
            .is_none_or(|q| row.principal_minor != q.details.amount.minor_units().to_string())
        {
            add!("PROVIDER_PRINCIPAL_MISMATCH");
        }
        // This is the provider's explicit zero-fee fixture schedule, not the customer's bank fee.
        if row.provider_fee_minor != "0" {
            add!("PROVIDER_FEE_UNEXPECTED");
        }
        if row.business_date < file.range.from_date
            || row.business_date >= file.range.to_date
            || core.is_some_and(|c| c.business_date.is_some_and(|d| d != row.business_date))
        {
            add!("PROVIDER_DATE_MISMATCH");
        }
        match settled_receipt.as_ref().or(core_evidence.as_ref()) {
            Some(e)
                if e.evidence_id == row.evidence_id
                    && e.version == row.provider_version
                    && e.settlement_date == Some(row.business_date) => {}
            _ => add!("PROVIDER_SETTLEMENT_EVIDENCE_MISMATCH"),
        }
    }
    let posted = core.is_some_and(|c| c.outcome == "SETTLED");
    let terminal = payment.is_some_and(|p| {
        matches!(
            p.state.as_str(),
            "CANCELLED" | "DECLINED" | "BLOCKED" | "EXPIRED"
        )
    }) && core
        .is_none_or(|c| matches!(c.outcome.as_str(), "CANCELLED" | "DECLINED" | "RELEASED"));
    if !posted && !terminal && codes.is_empty() {
        add!("PENDING_SETTLEMENT");
    }
    let timing = |c: &str| {
        matches!(
            c,
            "CORE_AFTER_REPORT"
                | "SETTLED_OUTSIDE_REPORT_PERIOD"
                | "SETTLED_AFTER_FILE"
                | "SOURCE_SNAPSHOT_TIMING"
                | "PAYMENT_ACK_TIMING"
                | "GL_TIMING_WINDOW"
                | "PROVIDER_DATE_MISMATCH"
        )
    };
    let classification = if codes.is_empty() {
        if posted { "MATCHED" } else { "NOT_POSTED" }
    } else if codes.iter().all(|c| c == "PENDING_SETTLEMENT") {
        "PENDING"
    } else if codes.iter().all(|c| timing(c)) {
        "TIMING"
    } else {
        "EXCEPTION"
    };
    Ok(ReconciliationItem {
        ordinal,
        payment_id: id,
        classification: classification.into(),
        issue_codes: codes.into_iter().collect(),
        instruction: quote,
        instruction_digest: payment.map(|p| p.quote_digest.clone()),
        payment_state: payment.map(|p| p.state.clone()),
        core_outcome: core.map(|c| c.outcome.clone()),
        core_principal: core.map(|c| c.principal),
        core_bank_fee: core.map(|c| c.fee),
        core_batch_id: core.and_then(|c| c.batch_id),
        journal_batch_ids: sources.iter().map(|s| s.source.journal.id).collect(),
        gl_export_ids: sources.iter().filter_map(|s| s.export_id).collect(),
        provider_rows: rows,
    })
}

impl Finance {
    pub async fn create_reconciliation(
        &self,
        actor: &ActorContext,
        key: Uuid,
        input: ReconciliationInput,
    ) -> Result<ReconciliationDetail> {
        self.authorize_staff(actor, true).await?;
        ensure!(
            !key.is_nil() && !input.report_id.is_nil() && !input.file_id.is_nil(),
            ReconciliationInputError
        );
        let request_digest = fingerprint(&input)?;
        if let Some(row)=sqlx::query("SELECT id,request_digest FROM finance.reconciliation_runs WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&self.pool).await? {
            ensure!(row.get::<String,_>("request_digest")==request_digest,ReconciliationKeyConflict);
            return self.reconciliation_detail(actor,row.get("id")).await;
        }
        let report = self.report_detail(actor, input.report_id).await?;
        let mut sources = vec![];
        let mut after = 0;
        loop {
            let page = self
                .report_items(actor, input.report_id, after, 100)
                .await?;
            sources.extend(page.items);
            if let Some(next) = page.next_after {
                after = next;
            } else {
                break;
            }
        }
        let (file, packet) = self.client.settlement_file(input.file_id).await?;
        ensure!(file.range == report.report.range, ReconciliationInputError);
        let batch_ids = sources
            .iter()
            .map(|s| s.source.journal.id)
            .collect::<Vec<_>>();
        let file_operations = file.rows.iter().map(|r| r.payment_id).collect::<Vec<_>>();
        let accounting = self
            .core
            .payment_accounting(report.report.range, &batch_ids, &file_operations)
            .await?;
        let ids = accounting
            .facts
            .iter()
            .map(|c| c.payment_id)
            .chain(file.rows.iter().map(|r| r.payment_id))
            .chain(
                sources
                    .iter()
                    .filter(|s| {
                        matches!(
                            s.source.journal.posting_kind.as_str(),
                            "INTERNAL_PAYMENT" | "EXTERNAL_PAYMENT"
                        )
                    })
                    .map(|s| s.source.journal.operation_id),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let payments = self.payments.snapshot(report.report.range, &ids).await?;
        let confirmation_ids = ids
            .iter()
            .copied()
            .chain(payments.payments.iter().map(|p| p.payment_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let accounting = self
            .core
            .payment_accounting(report.report.range, &batch_ids, &confirmation_ids)
            .await?;
        let ids = ids
            .into_iter()
            .chain(accounting.facts.iter().map(|c| c.payment_id))
            .chain(payments.payments.iter().map(|p| p.payment_id))
            .collect::<BTreeSet<_>>();
        ensure!(ids.len() <= 1000, SnapshotLimitExceeded);
        let mut line_counts = BTreeMap::new();
        for row in &file.rows {
            *line_counts.entry(row.line_id).or_insert(0usize) += 1;
        }
        let duplicate_lines = line_counts
            .into_iter()
            .filter_map(|(id, n)| (n > 1).then_some(id))
            .collect::<BTreeSet<_>>();
        let by_core = accounting
            .facts
            .iter()
            .map(|c| (c.payment_id, c))
            .collect::<BTreeMap<_, _>>();
        let by_payment = payments
            .payments
            .iter()
            .map(|p| (p.payment_id, p))
            .collect::<BTreeMap<_, _>>();
        let mut items = vec![];
        for (n, id) in ids.into_iter().enumerate() {
            let relevant = sources
                .iter()
                .filter(|s| {
                    s.source.journal.operation_id == id
                        && matches!(
                            s.source.journal.posting_kind.as_str(),
                            "INTERNAL_PAYMENT" | "EXTERNAL_PAYMENT"
                        )
                })
                .collect::<Vec<_>>();
            items.push(inspect(
                id,
                n as i32 + 1,
                Inspection {
                    payment: by_payment.get(&id).copied(),
                    core: by_core.get(&id).copied(),
                    sources: relevant,
                    file: ClearingRows {
                        rows: &file.rows,
                        range: file.range,
                        generated_at: file.generated_at,
                    },
                    core_as_of: accounting.as_of,
                    payments_as_of: payments.as_of,
                    duplicate_lines: &duplicate_lines,
                },
            )?);
        }
        let count = |state: &str| items.iter().filter(|i| i.classification == state).count() as i32;
        let mut totals = BTreeMap::<(String, u8), (i128, i128)>::new();
        for row in &file.rows {
            let total = totals
                .entry((row.currency_code.clone(), row.currency_scale))
                .or_default();
            total.0 += row.principal_minor.parse::<i64>()? as i128;
            total.1 += row.provider_fee_minor.parse::<i64>()? as i128;
        }
        let core_total = |fee: bool| {
            accounting
                .facts
                .iter()
                .filter(|c| c.outcome == "SETTLED")
                .map(|c| i128::from(if fee { c.fee } else { c.principal }.minor_units()))
                .sum::<i128>()
                .to_string()
        };
        let matched_total = items
            .iter()
            .filter(|i| i.classification == "MATCHED")
            .filter_map(|i| i.core_principal)
            .map(|a| i128::from(a.minor_units()))
            .sum::<i128>()
            .to_string();
        let summary = ReconciliationSummary {
            id: Uuid::new_v4(),
            scope: "OUTGOING_PAYMENTS_V1".into(),
            format_version: 1,
            report_id: input.report_id,
            report_digest: report.digest,
            file_id: input.file_id,
            file_digest: fingerprint(&file)?,
            core_digest: fingerprint(&accounting)?,
            payments_digest: fingerprint(&payments)?,
            items_digest: fingerprint(&items)?,
            report_as_of: report.report.core_as_of,
            core_as_of: accounting.as_of,
            payments_as_of: payments.as_of,
            file_generated_at: file.generated_at,
            created_at: Utc::now(),
            record_count: items.len() as i32,
            matched_count: count("MATCHED"),
            exception_count: count("EXCEPTION"),
            timing_count: count("TIMING"),
            pending_count: count("PENDING"),
            not_posted_count: count("NOT_POSTED"),
            core_principal_total: core_total(false),
            core_bank_fee_total: core_total(true),
            matched_principal_total: matched_total,
            provider_totals: totals
                .into_iter()
                .map(|((currency_code, currency_scale), (p, f))| ProviderTotal {
                    currency_code,
                    currency_scale,
                    principal: p.to_string(),
                    provider_fee: f.to_string(),
                })
                .collect(),
            completeness: if count("EXCEPTION") > 0 {
                "EXCEPTIONS"
            } else if count("TIMING") + count("PENDING") > 0 {
                "PARTIAL"
            } else {
                "COMPLETE"
            }
            .into(),
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,173))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        if let Some(row)=sqlx::query("SELECT id,request_digest FROM finance.reconciliation_runs WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await? {
            ensure!(row.get::<String,_>("request_digest")==request_digest,ReconciliationKeyConflict);let id=row.get("id");tx.commit().await?;return self.reconciliation_detail(actor,id).await;
        }
        sqlx::query("INSERT INTO finance.reconciliation_runs(id,report_id,file_id,issuer,subject,client_id,request_key,request_digest,document,digest,provider_packet,core_snapshot,payments_snapshot) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)").bind(summary.id).bind(summary.report_id).bind(summary.file_id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(request_digest).bind(serde_json::to_value(&summary)?).bind(fingerprint(&summary)?).bind(serde_json::to_value(packet)?).bind(serde_json::to_value(&accounting)?).bind(serde_json::to_value(&payments)?).execute(&mut *tx).await?;
        for item in &items {
            sqlx::query("INSERT INTO finance.reconciliation_items(run_id,ordinal,payment_id,document,digest) VALUES($1,$2,$3,$4,$5)").bind(summary.id).bind(item.ordinal).bind(item.payment_id).bind(serde_json::to_value(item)?).bind(fingerprint(item)?).execute(&mut *tx).await?;
            for code in &item.issue_codes {
                let grace = code == "PENDING_SETTLEMENT"
                    && by_payment.get(&item.payment_id).is_some_and(|p| {
                        summary.created_at - p.created_at < chrono::Duration::minutes(5)
                    })
                    || code == "GL_TIMING_WINDOW"
                    || code == "PAYMENT_ACK_TIMING"
                        && by_core.get(&item.payment_id).is_some_and(|c| {
                            summary.created_at - c.recorded_at < chrono::Duration::seconds(30)
                        });
                if !grace {
                    sqlx::query("INSERT INTO finance.reconciliation_issues(id,run_id,ordinal,code) VALUES($1,$2,$3,$4)").bind(Uuid::new_v4()).bind(summary.id).bind(item.ordinal).bind(code).execute(&mut *tx).await?;
                }
            }
        }
        Self::audit_in(&mut tx, actor, "finance.reconciliation.saved", summary.id).await?;
        tx.commit().await?;
        self.recover_reconciliation_cases().await?;
        self.reconciliation_detail(actor, summary.id).await
    }
    async fn verified_reconciliation(
        &self,
        id: Uuid,
    ) -> Result<(ReconciliationSummary, String, Vec<ReconciliationItem>)> {
        let row = sqlx::query("SELECT * FROM finance.reconciliation_runs WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let summary: ReconciliationSummary = serde_json::from_value(row.get("document"))?;
        let digest: String = row.get("digest");
        ensure!(
            summary.id == id
                && summary.format_version == 1
                && summary.scope == "OUTGOING_PAYMENTS_V1"
                && fingerprint(&summary)? == digest
                && row.get::<Uuid, _>("report_id") == summary.report_id
                && row.get::<Uuid, _>("file_id") == summary.file_id,
            "Reconciliation artifact identity mismatch"
        );
        let core: PaymentAccountingSnapshot = serde_json::from_value(row.get("core_snapshot"))?;
        let payments: PaymentLineageSnapshot =
            serde_json::from_value(row.get("payments_snapshot"))?;
        ensure!(
            fingerprint(&core)? == summary.core_digest
                && fingerprint(&payments)? == summary.payments_digest,
            "Reconciliation private source integrity mismatch"
        );
        let packet: SignedDocument = serde_json::from_value(row.get("provider_packet"))?;
        packet.verify_integrity(crate::protocol::signing_key().as_bytes())?;
        let reply: SettlementReply = serde_json::from_value(packet.body)?;
        reply.validate()?;
        ensure!(
            reply.file.file_id == summary.file_id
                && fingerprint(&reply.file)? == summary.file_digest,
            "Reconciliation clearing lineage mismatch"
        );
        let parent = sqlx::query("SELECT document,digest FROM finance.reports WHERE id=$1")
            .bind(summary.report_id)
            .fetch_one(&self.pool)
            .await?;
        ensure!(
            parent.get::<String, _>("digest") == summary.report_digest
                && fingerprint(&parent.get::<Value, _>("document"))? == summary.report_digest,
            "Reconciliation parent report mismatch"
        );
        let rows=sqlx::query("SELECT document,digest FROM finance.reconciliation_items WHERE run_id=$1 ORDER BY ordinal").bind(id).fetch_all(&self.pool).await?;
        let mut items = vec![];
        for (n, row) in rows.into_iter().enumerate() {
            let item: ReconciliationItem = serde_json::from_value(row.get("document"))?;
            ensure!(
                fingerprint(&item)? == row.get::<String, _>("digest")
                    && item.ordinal == n as i32 + 1,
                "Reconciliation item integrity mismatch"
            );
            items.push(item);
        }
        ensure!(
            items.len() == summary.record_count as usize
                && fingerprint(&items)? == summary.items_digest,
            "Reconciliation membership mismatch"
        );
        Ok((summary, digest, items))
    }
    pub async fn reconciliation_detail(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<ReconciliationDetail> {
        self.authorize_staff(actor, false).await?;
        let (reconciliation, digest, _) = self.verified_reconciliation(id).await?;
        let issues = sqlx::query(
            "SELECT * FROM finance.reconciliation_issues WHERE run_id=$1 ORDER BY ordinal,code,id",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|r| ReconciliationIssue {
            id: r.get("id"),
            ordinal: r.get("ordinal"),
            code: r.get("code"),
            case_id: r.get("case_id"),
        })
        .collect();
        Ok(ReconciliationDetail {
            reconciliation,
            digest,
            issues,
        })
    }
    pub async fn reconciliations(
        &self,
        actor: &ActorContext,
        report: Option<Uuid>,
    ) -> Result<Vec<ReconciliationSummary>> {
        self.authorize_staff(actor, false).await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.reconciliation_runs WHERE ($1::uuid IS NULL OR report_id=$1) ORDER BY created_at DESC,id LIMIT 100").bind(report).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.verified_reconciliation(id).await?.0);
        }
        Ok(out)
    }
    pub async fn reconciliation_items(
        &self,
        actor: &ActorContext,
        id: Uuid,
        after: i32,
        limit: i64,
    ) -> Result<ReconciliationPage> {
        self.authorize_staff(actor, false).await?;
        let (summary, digest, items) = self.verified_reconciliation(id).await?;
        ensure!(
            after >= 0 && after <= summary.record_count && (1..=100).contains(&limit),
            ReconciliationInputError
        );
        let items = items
            .into_iter()
            .skip(after as usize)
            .take(limit as usize)
            .collect::<Vec<_>>();
        let last = items.last().map(|i| i.ordinal).unwrap_or(after);
        Ok(ReconciliationPage {
            items,
            total: summary.record_count,
            next_after: (last < summary.record_count).then_some(last),
            reconciliation_digest: digest,
        })
    }
    pub async fn recover_reconciliation_cases(&self) -> Result<()> {
        let after: Uuid = sqlx::query_scalar(
            "SELECT last_issue_id FROM finance.reconciliation_case_cursor WHERE id",
        )
        .fetch_one(&self.pool)
        .await?;
        let rows=sqlx::query("SELECT i.id,i.run_id,i.ordinal,r.payment_id FROM finance.reconciliation_issues i JOIN finance.reconciliation_items r ON r.run_id=i.run_id AND r.ordinal=i.ordinal WHERE i.case_id IS NULL ORDER BY (i.id<=$1),i.id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        let cursor = rows
            .last()
            .map(|r| r.get::<Uuid, _>("id"))
            .unwrap_or(Uuid::nil());
        sqlx::query("UPDATE finance.reconciliation_case_cursor SET last_issue_id=$1 WHERE id")
            .bind(cursor)
            .execute(&self.pool)
            .await?;
        for row in rows {
            let id: Uuid = row.get("id");
            let run: Uuid = row.get("run_id");
            let payment: Uuid = row.get("payment_id");
            let owned = Cases::new(self.controls.pool.clone())
                .ensure(
                    &service_actor(),
                    NewCase {
                        kind: CaseKind::Finance,
                        origin_id: id,
                        resource_id: payment,
                        evidence_refs: vec![run, payment, id],
                        customer: None,
                    },
                )
                .await;
            if let Ok(case) = owned {
                sqlx::query("UPDATE finance.reconciliation_issues SET case_id=$2 WHERE id=$1 AND case_id IS NULL").bind(id).bind(case).execute(&self.pool).await?;
            } else {
                tracing::warn!(code="RECONCILIATION_CASE_ASSIGNMENT_PENDING",issue_id=%id,"Finance case assignment remains pending; continuing the bounded cursor");
            }
        }
        Ok(())
    }
}
