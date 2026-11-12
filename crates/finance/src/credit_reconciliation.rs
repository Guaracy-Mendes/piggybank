//! Immutable observation of incoming and returned principal, suspense, allocation and GL.
use crate::settlement::ClearingRows;
use crate::{
    Finance,
    credit_settlement::{CreditSettlementReply, CreditSettlementRow},
    reconciliation::{
        ProviderTotal, ReconciliationInput, ReconciliationInputError, ReconciliationIssue,
        ReconciliationKeyConflict,
    },
    reports::ReportItem,
    service_actor,
};
use anyhow::{Result, ensure};
use bank_controls::cases::{CaseKind, Cases, NewCase};
use bank_core::{
    credit_accounting::{CreditAccountingFact, CreditAccountingSnapshot},
    finance_snapshot::SnapshotLimitExceeded,
    payments::InternalInstruction,
};
use bank_payments::finance::{CreditLineage, CreditLineageSnapshot};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{AccountId, ActorContext, Money, fingerprint},
};
use bank_rails::{CreditEvidence, CreditKind, RailEvidence, RailStatus};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditReconciliationSummary {
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
    pub suspense_count: i32,
    pub core_principal_total: String,
    pub core_allocated_principal_total: String,
    pub core_suspense_principal_total: String,
    pub matched_principal_total: String,
    pub provider_totals: Vec<ProviderTotal>,
    pub completeness: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditReconciliationItem {
    pub ordinal: i32,
    pub movement_id: Uuid,
    pub classification: String,
    pub issue_codes: Vec<String>,
    pub kind: Option<CreditKind>,
    pub original_payment_id: Option<Uuid>,
    pub target_account_id: Option<AccountId>,
    pub receiving_reference: Option<String>,
    pub principal: Option<Money>,
    pub evidence_id: Option<Uuid>,
    pub evidence_digest: Option<String>,
    pub payment_state: Option<String>,
    pub suspense_batch_id: Option<Uuid>,
    pub allocation_batch_id: Option<Uuid>,
    pub original_batch_id: Option<Uuid>,
    pub suspense_business_date: Option<NaiveDate>,
    pub allocation_business_date: Option<NaiveDate>,
    pub returned_principal_total: Option<String>,
    pub journal_batch_ids: Vec<Uuid>,
    pub gl_export_ids: Vec<Uuid>,
    pub provider_rows: Vec<CreditSettlementRow>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditReconciliationDetail {
    pub reconciliation: CreditReconciliationSummary,
    pub digest: String,
    pub issues: Vec<ReconciliationIssue>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditReconciliationPage {
    pub items: Vec<CreditReconciliationItem>,
    pub total: i32,
    pub next_after: Option<i32>,
    pub reconciliation_digest: String,
}
fn evidence(packet: &Value) -> Result<CreditEvidence> {
    let p: SignedDocument = serde_json::from_value(packet.clone())?;
    p.verify_integrity(bank_rails::signing_key().as_bytes())?;
    let e: CreditEvidence = serde_json::from_value(p.body)?;
    e.validate()?;
    ensure!(
        !e.movement_id.is_nil() && !e.evidence_id.is_nil(),
        "Credit evidence identity required"
    );
    Ok(e)
}
fn credit_kind(k: &CreditKind) -> &'static str {
    match k {
        CreditKind::InboundSettled => "INBOUND_SETTLED",
        CreditKind::ReturnSettled => "RETURN_SETTLED",
    }
}
pub(crate) struct Inspection<'a> {
    pub core: Option<&'a CreditAccountingFact>,
    pub payment: Option<&'a CreditLineage>,
    pub sources: Vec<&'a ReportItem>,
    pub file: ClearingRows<'a, CreditSettlementRow>,
    pub payments_as_of: DateTime<Utc>,
    pub payments_requested: bool,
    pub duplicate_lines: &'a BTreeSet<Uuid>,
    pub duplicate_evidence: &'a BTreeSet<Uuid>,
}
pub(crate) fn inspect(
    id: Uuid,
    ordinal: i32,
    scope: Inspection<'_>,
) -> Result<CreditReconciliationItem> {
    let Inspection {
        core,
        payment,
        sources,
        file,
        payments_as_of,
        payments_requested,
        duplicate_lines,
        duplicate_evidence,
    } = scope;
    let rows = file
        .rows
        .iter()
        .filter(|r| r.movement_id == id)
        .cloned()
        .collect::<Vec<_>>();
    let mut codes = BTreeSet::<String>::new();
    macro_rules! add {
        ($c:expr) => {{
            codes.insert(($c).into());
        }};
    }
    let pe = payment.and_then(|p| evidence(&p.packet).ok());
    if let Some(p) = payment {
        if pe.as_ref().is_none_or(|e| {
            e.movement_id != id
                || e.evidence_id != p.evidence_id
                || fingerprint(e).ok().as_ref() != Some(&p.request_digest)
        }) {
            add!("INVALID_CREDIT_INSTRUCTION");
        }
    } else {
        add!(
            if core.is_some_and(|c| c.recorded_at > payments_as_of || !payments_requested) {
                "SOURCE_SNAPSHOT_TIMING"
            } else {
                "MISSING_CREDIT_INSTRUCTION"
            }
        );
    }
    let ce = core.and_then(|c| evidence(&c.packet).ok());
    if let Some(c) = core {
        if ce.as_ref().is_none_or(|e| {
            e.movement_id != id
                || e.evidence_id != c.evidence_id
                || fingerprint(e).ok().as_ref() != Some(&c.request_digest)
                || credit_kind(&e.event_type) != c.kind
                || e.amount != c.principal
                || e.business_date != c.business_date
                || e.original_payment_id != c.original_payment_id
                || e.receiving_reference != c.receiving_reference
        }) {
            add!("CORE_CREDIT_RECEIPT_MISMATCH");
        }
        if let Some(p) = payment {
            if p.request_digest != c.request_digest || p.target_account_id != c.target_account_id {
                add!("CREDIT_WORKFLOW_SCOPE_MISMATCH");
            }
            if c.allocation_batch_id.is_some() && p.state != "CREDITED" {
                add!("CREDIT_ACK_TIMING");
            }
            if c.allocation_batch_id.is_none() && p.state == "CREDITED" {
                add!("MISSING_CORE_ALLOCATION");
            }
        }
        if c.allocation_batch_id.is_some()
            && (c.allocation_digest.as_ref() != Some(&c.request_digest)
                || c.target_account_id.is_none())
        {
            add!("CORE_ALLOCATION_SCOPE_MISMATCH");
        }
        if let Some(e) = &ce {
            if e.event_type == CreditKind::ReturnSettled {
                let valid = (|| -> Result<bool> {
                    let Some(o) = &c.original else {
                        return Ok(false);
                    };
                    let q: InternalInstruction = serde_json::from_value(o.instruction.clone())?;
                    let packet: SignedDocument = serde_json::from_value(o.finality.clone())?;
                    packet.verify_integrity(bank_rails::signing_key().as_bytes())?;
                    let f: RailEvidence = serde_json::from_value(packet.body)?;
                    f.validate_binding(
                        o.payment_id,
                        &o.request_digest,
                        o.principal,
                        q.details.requested_date,
                    )?;
                    let total = o.returned_principal_total.parse::<i128>()?;
                    Ok(Some(o.payment_id) == e.original_payment_id
                        && o.payment_id == q.operation_id
                        && fingerprint(&q)? == o.request_digest
                        && q.details.rail == "SIMULATED"
                        && q.details.amount == o.principal
                        && q.details.payer_account_id == o.payer_account_id
                        && c.target_account_id == Some(o.payer_account_id)
                        && f.status == RailStatus::Settled
                        && f.settlement_date == Some(o.business_date)
                        && e.business_date >= o.business_date
                        && total.to_string() == o.returned_principal_total
                        && total >= i128::from(e.amount.minor_units())
                        && total <= i128::from(o.principal.minor_units()))
                })()
                .unwrap_or(false);
                if !valid {
                    add!("RETURN_ORIGINAL_OR_CAP_MISMATCH");
                }
            } else if c.original.is_some() || c.original_payment_id.is_some() {
                add!("UNEXPECTED_RETURN_ORIGINAL");
            }
        }
    } else if pe.is_some()
        && sources.is_empty()
        && payment.is_some_and(|p| !matches!(p.state.as_str(), "CREDITED" | "QUARANTINED"))
    {
        add!("CREDIT_PROCESSING_PENDING");
    } else {
        add!("MISSING_CORE_CREDIT");
    }
    // A valid native receipt can survive an orchestration ACK loss. Never replace native money with workflow state.
    let expected = ce.as_ref().or(pe.as_ref());
    if rows.len() > 1 {
        add!("DUPLICATE_PROVIDER_MOVEMENT");
    }
    for r in &rows {
        if duplicate_lines.contains(&r.line_id) {
            add!("DUPLICATE_PROVIDER_LINE");
        }
        if duplicate_evidence.contains(&r.evidence_id) {
            add!("DUPLICATE_PROVIDER_EVIDENCE");
        }
        if r.currency_code != "TST" || r.currency_scale != 2 {
            add!("PROVIDER_CURRENCY_MISMATCH");
        }
        if r.provider_fee_minor != "0" {
            add!("PROVIDER_FEE_UNEXPECTED");
        }
        if let Some(e) = expected {
            if r.principal_minor != e.amount.minor_units().to_string() {
                add!("PROVIDER_PRINCIPAL_MISMATCH");
            }
            if r.event_type != e.event_type
                || r.original_payment_id != e.original_payment_id
                || r.receiving_reference != e.receiving_reference
            {
                add!("PROVIDER_CREDIT_SCOPE_MISMATCH");
            }
            if r.evidence_id != e.evidence_id || r.provider_version != e.version {
                add!("PROVIDER_SETTLEMENT_EVIDENCE_MISMATCH");
            }
            if r.business_date != e.business_date {
                add!("PROVIDER_DATE_MISMATCH");
            }
        }
        if r.business_date < file.range.from_date || r.business_date >= file.range.to_date {
            add!("PROVIDER_DATE_OUTSIDE_FILE");
        }
    }
    if !rows.is_empty() && expected.is_some_and(|e| e.occurred_at > file.generated_at) {
        add!("PROVIDER_EVENT_AFTER_FILE_CUTOFF");
    }
    if rows.is_empty() {
        if expected.is_some_and(|e| {
            e.business_date < file.range.from_date || e.business_date >= file.range.to_date
        }) {
            add!("CREDIT_VALUE_OUTSIDE_FILE_PERIOD");
        } else if expected.is_some_and(|e| e.occurred_at > file.generated_at) {
            add!("SETTLED_AFTER_FILE");
        } else {
            add!("MISSING_PROVIDER_SETTLEMENT");
        }
    }
    if let Some(c) = core {
        let clearing = Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?;
        let suspense = Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000006")?;
        let principal = c.principal.minor_units();
        for (batch, date, allocation) in
            std::iter::once((c.suspense_batch_id, c.suspense_business_date, false)).chain(
                c.allocation_batch_id
                    .zip(c.allocation_business_date)
                    .map(|(b, d)| (b, d, true)),
            )
        {
            let found = sources
                .iter()
                .filter(|s| s.source.journal.id == batch)
                .copied()
                .collect::<Vec<_>>();
            if found.is_empty() {
                add!(
                    if date < file.range.from_date || date >= file.range.to_date {
                        "CREDIT_OUTSIDE_REPORT_PERIOD"
                    } else {
                        "CORE_AFTER_REPORT"
                    }
                );
                continue;
            }
            let s = found[0];
            let j = &s.source.journal;
            let kind = if allocation {
                "CREDIT_ALLOCATION"
            } else if c.kind == "RETURN_SETTLED" {
                "RETURN_SUSPENSE"
            } else {
                "INBOUND_SUSPENSE"
            };
            let original = if allocation {
                Some(c.suspense_batch_id)
            } else {
                c.original.as_ref().map(|o| o.batch_id)
            };
            let legs = j
                .entries
                .iter()
                .map(|e| (e.account_id.0, e.amount.minor_units()))
                .collect::<BTreeMap<_, _>>();
            let expected_legs = if allocation {
                BTreeMap::from([
                    (suspense, -principal),
                    (
                        c.target_account_id.map(|a| a.0).unwrap_or(Uuid::nil()),
                        principal,
                    ),
                ])
            } else {
                BTreeMap::from([(clearing, -principal), (suspense, principal)])
            };
            if j.operation_id != id
                || j.posting_kind != kind
                || j.original_batch_id != original
                || j.business_date != date
                || legs != expected_legs
                || j.entries.len() != 2
            {
                add!("CORE_CREDIT_JOURNAL_MISMATCH");
            }
            if !allocation && date != c.business_date {
                add!("CREDIT_BOOKING_DATE_DIFFERENCE");
            }
            for code in &s.issue_codes {
                add!(code.clone());
            }
            if s.classification != "MATCHED" && s.issue_codes.is_empty() {
                add!("MISSING_GL_BOOKING");
            }
        }
        let expected_count = 1 + usize::from(c.allocation_batch_id.is_some());
        if sources.iter().any(|s| {
            s.source.journal.id != c.suspense_batch_id
                && Some(s.source.journal.id) != c.allocation_batch_id
        }) || sources.len() > expected_count
        {
            add!("UNEXPECTED_CREDIT_JOURNAL");
        }
        if c.allocation_batch_id.is_none() {
            add!("FUNDS_IN_SUSPENSE");
        }
    } else if !sources.is_empty() {
        add!("UNLINKED_CREDIT_JOURNAL");
    }
    let timing = |c: &str| {
        matches!(
            c,
            "CREDIT_VALUE_OUTSIDE_FILE_PERIOD"
                | "SOURCE_SNAPSHOT_TIMING"
                | "CREDIT_ACK_TIMING"
                | "SETTLED_AFTER_FILE"
                | "CREDIT_OUTSIDE_REPORT_PERIOD"
                | "CORE_AFTER_REPORT"
                | "GL_TIMING_WINDOW"
                | "CREDIT_BOOKING_DATE_DIFFERENCE"
        )
    };
    let classification = if codes.is_empty() {
        "MATCHED"
    } else if codes.contains("CREDIT_PROCESSING_PENDING")
        && codes
            .iter()
            .all(|c| c == "CREDIT_PROCESSING_PENDING" || timing(c))
    {
        "PENDING"
    } else if codes.iter().all(|c| c == "FUNDS_IN_SUSPENSE") {
        "SUSPENSE"
    } else if codes.iter().all(|c| timing(c) || c == "FUNDS_IN_SUSPENSE") {
        "TIMING"
    } else {
        "EXCEPTION"
    };
    Ok(CreditReconciliationItem {
        ordinal,
        movement_id: id,
        classification: classification.into(),
        issue_codes: codes.into_iter().collect(),
        kind: expected.map(|e| e.event_type.clone()),
        original_payment_id: expected.and_then(|e| e.original_payment_id),
        target_account_id: core.and_then(|c| c.target_account_id),
        receiving_reference: expected.and_then(|e| e.receiving_reference.clone()),
        principal: core.map(|c| c.principal),
        evidence_id: expected.map(|e| e.evidence_id),
        evidence_digest: core
            .map(|c| c.request_digest.clone())
            .or_else(|| payment.map(|p| p.request_digest.clone())),
        payment_state: payment.map(|p| p.state.clone()),
        suspense_batch_id: core.map(|c| c.suspense_batch_id),
        allocation_batch_id: core.and_then(|c| c.allocation_batch_id),
        original_batch_id: core.and_then(|c| c.original.as_ref().map(|o| o.batch_id)),
        suspense_business_date: core.map(|c| c.suspense_business_date),
        allocation_business_date: core.and_then(|c| c.allocation_business_date),
        returned_principal_total: core.and_then(|c| {
            c.original
                .as_ref()
                .map(|o| o.returned_principal_total.clone())
        }),
        journal_batch_ids: sources.iter().map(|s| s.source.journal.id).collect(),
        gl_export_ids: sources.iter().filter_map(|s| s.export_id).collect(),
        provider_rows: rows,
    })
}
impl Finance {
    pub async fn create_credit_reconciliation(
        &self,
        actor: &ActorContext,
        key: Uuid,
        input: ReconciliationInput,
    ) -> Result<CreditReconciliationDetail> {
        self.authorize_staff(actor, true).await?;
        ensure!(
            !key.is_nil() && !input.report_id.is_nil() && !input.file_id.is_nil(),
            ReconciliationInputError
        );
        let request_digest = fingerprint(&input)?;
        if let Some(r)=sqlx::query("SELECT id,request_digest FROM finance.credit_reconciliation_runs WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&self.pool).await? {
            ensure!(r.get::<String,_>("request_digest")==request_digest,ReconciliationKeyConflict);
            return self.credit_reconciliation_detail(actor,r.get("id")).await;
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
                after = next
            } else {
                break;
            }
        }
        let (file, packet) = self.client.credit_settlement_file(input.file_id).await?;
        ensure!(file.range == report.report.range, ReconciliationInputError);
        let batch_ids = sources
            .iter()
            .map(|s| s.source.journal.id)
            .collect::<Vec<_>>();
        let file_ids = file.rows.iter().map(|r| r.movement_id).collect::<Vec<_>>();
        let accounting = self
            .core
            .credit_accounting(report.report.range, &batch_ids, &file_ids)
            .await?;
        let relevant = |s: &&ReportItem| {
            matches!(
                s.source.journal.posting_kind.as_str(),
                "INBOUND_SUSPENSE" | "RETURN_SUSPENSE" | "CREDIT_ALLOCATION"
            )
        };
        let ids = accounting
            .facts
            .iter()
            .map(|c| c.movement_id)
            .chain(file_ids)
            .chain(
                sources
                    .iter()
                    .filter(relevant)
                    .map(|s| s.source.journal.operation_id),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let requested_ids = ids.iter().copied().collect::<BTreeSet<_>>();
        let payments = self
            .payments
            .credit_snapshot(report.report.range, &ids)
            .await?;
        let confirmation_ids = ids
            .iter()
            .copied()
            .chain(payments.credits.iter().map(|p| p.movement_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let accounting = self
            .core
            .credit_accounting(report.report.range, &batch_ids, &confirmation_ids)
            .await?;
        let ids = confirmation_ids
            .into_iter()
            .chain(accounting.facts.iter().map(|c| c.movement_id))
            .collect::<BTreeSet<_>>();
        ensure!(ids.len() <= 1000, SnapshotLimitExceeded);
        let duplicates = |evidence: bool| {
            let mut counts = BTreeMap::new();
            for r in &file.rows {
                *counts
                    .entry(if evidence { r.evidence_id } else { r.line_id })
                    .or_insert(0usize) += 1;
            }
            counts
                .into_iter()
                .filter_map(|(id, n)| (n > 1).then_some(id))
                .collect::<BTreeSet<_>>()
        };
        let duplicate_lines = duplicates(false);
        let duplicate_evidence = duplicates(true);
        let by_core = accounting
            .facts
            .iter()
            .map(|c| (c.movement_id, c))
            .collect::<BTreeMap<_, _>>();
        let by_payment = payments
            .credits
            .iter()
            .map(|p| (p.movement_id, p))
            .collect::<BTreeMap<_, _>>();
        let mut items = vec![];
        for (n, id) in ids.into_iter().enumerate() {
            items.push(inspect(
                id,
                n as i32 + 1,
                Inspection {
                    core: by_core.get(&id).copied(),
                    payment: by_payment.get(&id).copied(),
                    sources: sources
                        .iter()
                        .filter(relevant)
                        .filter(|s| s.source.journal.operation_id == id)
                        .collect(),
                    file: ClearingRows {
                        rows: &file.rows,
                        range: file.range,
                        generated_at: file.generated_at,
                    },
                    payments_as_of: payments.as_of,
                    payments_requested: requested_ids.contains(&id),
                    duplicate_lines: &duplicate_lines,
                    duplicate_evidence: &duplicate_evidence,
                },
            )?);
        }
        let count = |state: &str| items.iter().filter(|i| i.classification == state).count() as i32;
        let mut totals = BTreeMap::<(String, u8), (i128, i128)>::new();
        for row in &file.rows {
            let t = totals
                .entry((row.currency_code.clone(), row.currency_scale))
                .or_default();
            t.0 += i128::from(row.principal_minor.parse::<i64>()?);
            t.1 += i128::from(row.provider_fee_minor.parse::<i64>()?);
        }
        let total = |allocated: Option<bool>| {
            accounting
                .facts
                .iter()
                .filter(|c| allocated.is_none_or(|a| c.allocation_batch_id.is_some() == a))
                .map(|c| i128::from(c.principal.minor_units()))
                .sum::<i128>()
                .to_string()
        };
        let summary = CreditReconciliationSummary {
            id: Uuid::new_v4(),
            scope: "INCOMING_RETURNS_V1".into(),
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
            suspense_count: count("SUSPENSE"),
            core_principal_total: total(None),
            core_allocated_principal_total: total(Some(true)),
            core_suspense_principal_total: total(Some(false)),
            matched_principal_total: items
                .iter()
                .filter(|i| i.classification == "MATCHED")
                .filter_map(|i| i.principal)
                .map(|m| i128::from(m.minor_units()))
                .sum::<i128>()
                .to_string(),
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
            } else if count("TIMING") + count("PENDING") + count("SUSPENSE") > 0 {
                "PARTIAL"
            } else {
                "COMPLETE"
            }
            .into(),
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,174))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        if let Some(row)=sqlx::query("SELECT id,request_digest FROM finance.credit_reconciliation_runs WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await? {
            ensure!(row.get::<String,_>("request_digest")==request_digest,ReconciliationKeyConflict);let id=row.get("id");tx.commit().await?;return self.credit_reconciliation_detail(actor,id).await;
        }
        sqlx::query("INSERT INTO finance.credit_reconciliation_runs(id,report_id,file_id,issuer,subject,client_id,request_key,request_digest,document,digest,provider_packet,core_snapshot,payments_snapshot) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)").bind(summary.id).bind(summary.report_id).bind(summary.file_id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(request_digest).bind(serde_json::to_value(&summary)?).bind(fingerprint(&summary)?).bind(serde_json::to_value(packet)?).bind(serde_json::to_value(&accounting)?).bind(serde_json::to_value(&payments)?).execute(&mut *tx).await?;
        for item in &items {
            sqlx::query("INSERT INTO finance.credit_reconciliation_items(run_id,ordinal,movement_id,document,digest) VALUES($1,$2,$3,$4,$5)").bind(summary.id).bind(item.ordinal).bind(item.movement_id).bind(serde_json::to_value(item)?).bind(fingerprint(item)?).execute(&mut *tx).await?;
            for code in &item.issue_codes {
                let grace = code == "CREDIT_PROCESSING_PENDING"
                    && by_payment.get(&item.movement_id).is_some_and(|p| {
                        summary.created_at - p.created_at < chrono::Duration::minutes(5)
                    })
                    || code == "GL_TIMING_WINDOW"
                    || code == "CREDIT_ACK_TIMING"
                        && by_core.get(&item.movement_id).is_some_and(|c| {
                            summary.created_at - c.allocated_at.unwrap_or(c.recorded_at)
                                < chrono::Duration::seconds(30)
                        });
                if !grace {
                    sqlx::query("INSERT INTO finance.credit_reconciliation_issues(id,run_id,ordinal,code) VALUES($1,$2,$3,$4)").bind(Uuid::new_v4()).bind(summary.id).bind(item.ordinal).bind(code).execute(&mut *tx).await?;
                }
            }
        }
        Self::audit_in(
            &mut tx,
            actor,
            "finance.credit_reconciliation.saved",
            summary.id,
        )
        .await?;
        tx.commit().await?;
        self.recover_credit_reconciliation_cases().await?;
        self.credit_reconciliation_detail(actor, summary.id).await
    }
    async fn verified_credit_reconciliation(
        &self,
        id: Uuid,
    ) -> Result<(
        CreditReconciliationSummary,
        String,
        Vec<CreditReconciliationItem>,
    )> {
        let row = sqlx::query("SELECT * FROM finance.credit_reconciliation_runs WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let summary: CreditReconciliationSummary = serde_json::from_value(row.get("document"))?;
        let digest: String = row.get("digest");
        ensure!(
            summary.id == id
                && summary.format_version == 1
                && summary.scope == "INCOMING_RETURNS_V1"
                && fingerprint(&summary)? == digest
                && row.get::<Uuid, _>("report_id") == summary.report_id
                && row.get::<Uuid, _>("file_id") == summary.file_id,
            "Reconciliation artifact identity mismatch"
        );
        let core: CreditAccountingSnapshot = serde_json::from_value(row.get("core_snapshot"))?;
        let payments: CreditLineageSnapshot = serde_json::from_value(row.get("payments_snapshot"))?;
        ensure!(
            fingerprint(&core)? == summary.core_digest
                && core.as_of == summary.core_as_of
                && fingerprint(&payments)? == summary.payments_digest
                && payments.as_of == summary.payments_as_of,
            "Reconciliation private source integrity mismatch"
        );
        let packet: SignedDocument = serde_json::from_value(row.get("provider_packet"))?;
        packet.verify_integrity(crate::protocol::signing_key().as_bytes())?;
        let reply: CreditSettlementReply = serde_json::from_value(packet.body)?;
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
        let rows=sqlx::query("SELECT ordinal,movement_id,document,digest FROM finance.credit_reconciliation_items WHERE run_id=$1 ORDER BY ordinal").bind(id).fetch_all(&self.pool).await?;
        let mut items = vec![];
        for (n, row) in rows.into_iter().enumerate() {
            let item: CreditReconciliationItem = serde_json::from_value(row.get("document"))?;
            ensure!(
                fingerprint(&item)? == row.get::<String, _>("digest")
                    && item.ordinal == n as i32 + 1
                    && item.ordinal == row.get::<i32, _>("ordinal")
                    && item.movement_id == row.get::<Uuid, _>("movement_id"),
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
    pub async fn credit_reconciliation_detail(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<CreditReconciliationDetail> {
        self.authorize_staff(actor, false).await?;
        let (reconciliation, digest, _) = self.verified_credit_reconciliation(id).await?;
        let issues = sqlx::query(
            "SELECT * FROM finance.credit_reconciliation_issues WHERE run_id=$1 ORDER BY ordinal,code,id",
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
        Ok(CreditReconciliationDetail {
            reconciliation,
            digest,
            issues,
        })
    }
    pub async fn credit_reconciliations(
        &self,
        actor: &ActorContext,
        report: Option<Uuid>,
    ) -> Result<Vec<CreditReconciliationSummary>> {
        self.authorize_staff(actor, false).await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.credit_reconciliation_runs WHERE ($1::uuid IS NULL OR report_id=$1) ORDER BY created_at DESC,id LIMIT 100").bind(report).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.verified_credit_reconciliation(id).await?.0);
        }
        Ok(out)
    }
    pub async fn credit_reconciliation_items(
        &self,
        actor: &ActorContext,
        id: Uuid,
        after: i32,
        limit: i64,
    ) -> Result<CreditReconciliationPage> {
        self.authorize_staff(actor, false).await?;
        let (summary, digest, items) = self.verified_credit_reconciliation(id).await?;
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
        Ok(CreditReconciliationPage {
            items,
            total: summary.record_count,
            next_after: (last < summary.record_count).then_some(last),
            reconciliation_digest: digest,
        })
    }
    pub async fn recover_credit_reconciliation_cases(&self) -> Result<()> {
        let after: Uuid = sqlx::query_scalar(
            "SELECT last_issue_id FROM finance.credit_reconciliation_case_cursor WHERE id",
        )
        .fetch_one(&self.pool)
        .await?;
        let rows=sqlx::query("SELECT i.id,i.run_id,i.ordinal,r.movement_id FROM finance.credit_reconciliation_issues i JOIN finance.credit_reconciliation_items r ON r.run_id=i.run_id AND r.ordinal=i.ordinal WHERE i.case_id IS NULL ORDER BY (i.id<=$1),i.id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        let cursor = rows
            .last()
            .map(|r| r.get::<Uuid, _>("id"))
            .unwrap_or(Uuid::nil());
        sqlx::query(
            "UPDATE finance.credit_reconciliation_case_cursor SET last_issue_id=$1 WHERE id",
        )
        .bind(cursor)
        .execute(&self.pool)
        .await?;
        for row in rows {
            let id: Uuid = row.get("id");
            let run: Uuid = row.get("run_id");
            let payment: Uuid = row.get("movement_id");
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
                sqlx::query("UPDATE finance.credit_reconciliation_issues SET case_id=$2 WHERE id=$1 AND case_id IS NULL").bind(id).bind(case).execute(&self.pool).await?;
            } else {
                tracing::warn!(code="RECONCILIATION_CASE_ASSIGNMENT_PENDING",issue_id=%id,"Finance case assignment remains pending; continuing the bounded cursor");
            }
        }
        Ok(())
    }
}
