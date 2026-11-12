//! Closed multi-file matching. Native money and historical V1 artifacts remain unchanged.
use crate::{
    Finance,
    credit_reconciliation::{self, CreditReconciliationItem},
    credit_settlement::{CreditSettlementReply, CreditSettlementRow},
    reconciliation::{
        self, ProviderTotal, ReconciliationInputError, ReconciliationIssue, ReconciliationItem,
        ReconciliationKeyConflict,
    },
    reports::ReportItem,
    service_actor,
    settlement::{ClearingRows, SettlementReply, SettlementRow, SettlementUnavailable},
    settlement_set::{CapturedFile, SetKind, SetMember, SetReply, SettlementSet},
};
use anyhow::{Result, ensure};
use bank_controls::cases::{CaseKind, Cases, NewCase};
use bank_core::{
    credit_accounting::CreditAccountingSnapshot,
    finance_snapshot::{ReportRange, SnapshotLimitExceeded},
    payment_accounting::PaymentAccountingSnapshot,
};
use bank_payments::finance::{CreditLineageSnapshot, PaymentLineageSnapshot};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{ActorContext, fingerprint},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupInput {
    pub report_id: Uuid,
    pub set_id: Uuid,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupSummary {
    pub id: Uuid,
    pub format_version: i32,
    pub kind: SetKind,
    pub scope: String,
    pub report_id: Uuid,
    pub report_digest: String,
    pub set_id: Uuid,
    pub set_digest: String,
    pub members: Vec<SetMember>,
    pub range: ReportRange,
    pub provider_row_count: i32,
    pub core_digest: String,
    pub payments_digest: String,
    pub items_digest: String,
    pub report_as_of: DateTime<Utc>,
    pub core_as_of: DateTime<Utc>,
    pub payments_as_of: DateTime<Utc>,
    pub provider_cutoff: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub record_count: i32,
    pub matched_count: i32,
    pub exception_count: i32,
    pub timing_count: i32,
    pub pending_count: i32,
    pub suspense_count: i32,
    pub not_posted_count: i32,
    pub core_principal_total: String,
    pub core_bank_fee_total: Option<String>,
    pub core_allocated_principal_total: Option<String>,
    pub core_suspense_principal_total: Option<String>,
    pub matched_principal_total: String,
    pub provider_totals: Vec<ProviderTotal>,
    pub completeness: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRow {
    pub file_id: Uuid,
    pub row_ordinal: i32,
    pub line_id: Uuid,
    pub row_digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupItem {
    pub ordinal: i32,
    pub operation_id: Uuid,
    pub classification: String,
    pub issue_codes: Vec<String>,
    pub source_rows: Vec<SourceRow>,
    pub outgoing: Option<ReconciliationItem>,
    pub credit: Option<CreditReconciliationItem>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDetail {
    pub reconciliation: GroupSummary,
    pub digest: String,
    pub issues: Vec<ReconciliationIssue>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPage {
    pub items: Vec<GroupItem>,
    pub total: i32,
    pub next_after: Option<i32>,
    pub reconciliation_digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "snapshot",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
enum NativeSnapshot {
    Outgoing(PaymentAccountingSnapshot),
    Credits(CreditAccountingSnapshot),
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "snapshot",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
enum WorkflowSnapshot {
    Outgoing(PaymentLineageSnapshot),
    Credits(CreditLineageSnapshot),
}
struct Capture {
    core: NativeSnapshot,
    payments: WorkflowSnapshot,
    items: Vec<GroupItem>,
}
impl Capture {
    fn clocks(&self) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
        match (&self.core, &self.payments) {
            (NativeSnapshot::Outgoing(c), WorkflowSnapshot::Outgoing(p)) => Ok((c.as_of, p.as_of)),
            (NativeSnapshot::Credits(c), WorkflowSnapshot::Credits(p)) => Ok((c.as_of, p.as_of)),
            _ => anyhow::bail!("Grouped source namespace mismatch"),
        }
    }
    fn grace(&self, id: Uuid, code: &str, now: DateTime<Utc>) -> bool {
        if code == "GL_TIMING_WINDOW" {
            return true;
        }
        match (&self.core, &self.payments) {
            (NativeSnapshot::Outgoing(c), WorkflowSnapshot::Outgoing(p)) => {
                code == "PENDING_SETTLEMENT"
                    && p.payments.iter().any(|p| {
                        p.payment_id == id && now - p.created_at < chrono::Duration::minutes(5)
                    })
                    || code == "PAYMENT_ACK_TIMING"
                        && c.facts.iter().any(|c| {
                            c.payment_id == id
                                && now - c.recorded_at < chrono::Duration::seconds(30)
                        })
            }
            (NativeSnapshot::Credits(c), WorkflowSnapshot::Credits(p)) => {
                code == "CREDIT_PROCESSING_PENDING"
                    && p.credits.iter().any(|p| {
                        p.movement_id == id && now - p.created_at < chrono::Duration::minutes(5)
                    })
                    || code == "CREDIT_ACK_TIMING"
                        && c.facts.iter().any(|c| {
                            c.movement_id == id
                                && now - c.allocated_at.unwrap_or(c.recorded_at)
                                    < chrono::Duration::seconds(30)
                        })
            }
            _ => false,
        }
    }
}
fn duplicates(ids: impl Iterator<Item = Uuid>) -> BTreeSet<Uuid> {
    let mut counts = BTreeMap::new();
    for id in ids {
        *counts.entry(id).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(id, n)| (n > 1).then_some(id))
        .collect()
}
fn lineage(files: &[CapturedFile], id: Uuid) -> Result<Vec<SourceRow>> {
    let mut refs = vec![];
    for file in files {
        match file {
            CapturedFile::Outgoing(f) => {
                for (n, r) in f
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.payment_id == id)
                {
                    refs.push(SourceRow {
                        file_id: f.file_id,
                        row_ordinal: n as i32 + 1,
                        line_id: r.line_id,
                        row_digest: fingerprint(r)?,
                    });
                }
            }
            CapturedFile::Credits(f) => {
                for (n, r) in f
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.movement_id == id)
                {
                    refs.push(SourceRow {
                        file_id: f.file_id,
                        row_ordinal: n as i32 + 1,
                        line_id: r.line_id,
                        row_digest: fingerprint(r)?,
                    });
                }
            }
        }
    }
    Ok(refs)
}
fn native_outgoing_time(
    fact: Option<&bank_core::payment_accounting::PaymentAccountingFact>,
) -> Option<DateTime<Utc>> {
    let fact = fact?;
    (|| -> Result<DateTime<Utc>> {
        let packet: SignedDocument = serde_json::from_value(
            fact.packet
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No native external proof"))?,
        )?;
        packet.verify_integrity(bank_rails::signing_key().as_bytes())?;
        let evidence: bank_rails::RailEvidence = serde_json::from_value(packet.body)?;
        evidence.validate_binding(
            fact.payment_id,
            &fact.request_digest,
            fact.principal,
            fact.requested_date,
        )?;
        ensure!(
            evidence.status == bank_rails::RailStatus::Settled,
            "Settled outgoing proof required"
        );
        Ok(evidence.occurred_at)
    })()
    .ok()
}
fn source_claims(files: &[CapturedFile], id: Uuid) -> Result<Value> {
    let mut rows = vec![];
    for file in files {
        match file {
            CapturedFile::Outgoing(f) => {
                for row in f.rows.iter().filter(|r| r.payment_id == id) {
                    rows.push(serde_json::to_value(row)?);
                }
            }
            CapturedFile::Credits(f) => {
                for row in f.rows.iter().filter(|r| r.movement_id == id) {
                    rows.push(serde_json::to_value(row)?);
                }
            }
        }
    }
    Ok(Value::Array(rows))
}
fn parse_archives(
    manifest: &SettlementSet,
    packets: &[SignedDocument],
) -> Result<Vec<CapturedFile>> {
    manifest.validate()?;
    ensure!(
        packets.len() == manifest.members.len(),
        "Complete closed settlement membership required"
    );
    let mut files = vec![];
    for (n, packet) in packets.iter().enumerate() {
        packet.verify_integrity(crate::protocol::signing_key().as_bytes())?;
        let file = match manifest.kind {
            SetKind::Outgoing => {
                let r: SettlementReply = serde_json::from_value(packet.body.clone())?;
                r.validate()?;
                CapturedFile::Outgoing(r.file)
            }
            SetKind::Credits => {
                let r: CreditSettlementReply = serde_json::from_value(packet.body.clone())?;
                r.validate()?;
                CapturedFile::Credits(r.file)
            }
        };
        manifest.verify_member(n, &file)?;
        files.push(file);
    }
    Ok(files)
}
impl Finance {
    async fn capture_group(
        &self,
        manifest: &SettlementSet,
        files: &[CapturedFile],
        sources: &[ReportItem],
    ) -> Result<Capture> {
        let range = manifest.range;
        let batch_ids = sources
            .iter()
            .map(|s| s.source.journal.id)
            .collect::<Vec<_>>();
        match manifest.kind {
            SetKind::Outgoing => {
                let rows = files
                    .iter()
                    .flat_map(|f| match f {
                        CapturedFile::Outgoing(f) => f.rows.iter(),
                        _ => unreachable!("verified namespace"),
                    })
                    .cloned()
                    .collect::<Vec<SettlementRow>>();
                let file_ids = rows.iter().map(|r| r.payment_id).collect::<Vec<_>>();
                let first = self
                    .core
                    .payment_accounting(range, &batch_ids, &file_ids)
                    .await?;
                let relevant = |s: &&ReportItem| {
                    matches!(
                        s.source.journal.posting_kind.as_str(),
                        "INTERNAL_PAYMENT" | "EXTERNAL_PAYMENT"
                    )
                };
                let ids = first
                    .facts
                    .iter()
                    .map(|c| c.payment_id)
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
                let payments = self.payments.snapshot(range, &ids).await?;
                let confirm = ids
                    .iter()
                    .copied()
                    .chain(payments.payments.iter().map(|p| p.payment_id))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let core = self
                    .core
                    .payment_accounting(range, &batch_ids, &confirm)
                    .await?;
                let ids = confirm
                    .into_iter()
                    .chain(core.facts.iter().map(|c| c.payment_id))
                    .collect::<BTreeSet<_>>();
                ensure!(ids.len() <= 1000, SnapshotLimitExceeded);
                let lines = duplicates(rows.iter().map(|r| r.line_id));
                let evidence = duplicates(rows.iter().map(|r| r.evidence_id));
                let by_core = core
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
                    let mut item = reconciliation::inspect(
                        id,
                        n as i32 + 1,
                        reconciliation::Inspection {
                            payment: by_payment.get(&id).copied(),
                            core: by_core.get(&id).copied(),
                            sources: sources
                                .iter()
                                .filter(relevant)
                                .filter(|s| s.source.journal.operation_id == id)
                                .collect(),
                            file: ClearingRows {
                                rows: &rows,
                                range,
                                generated_at: manifest.generated_at,
                            },
                            core_as_of: core.as_of,
                            payments_as_of: payments.as_of,
                            duplicate_lines: &lines,
                        },
                    )?;
                    if rows.iter().any(|r| r.payment_id == id)
                        && native_outgoing_time(by_core.get(&id).copied())
                            .is_some_and(|at| at > manifest.generated_at)
                    {
                        item.issue_codes
                            .push("PROVIDER_EVENT_AFTER_FILE_CUTOFF".into());
                        item.issue_codes.sort();
                        item.issue_codes.dedup();
                        item.classification = "EXCEPTION".into();
                    }
                    if rows
                        .iter()
                        .any(|r| r.payment_id == id && evidence.contains(&r.evidence_id))
                    {
                        item.issue_codes.push("DUPLICATE_PROVIDER_EVIDENCE".into());
                        item.issue_codes.sort();
                        item.issue_codes.dedup();
                        item.classification = "EXCEPTION".into();
                    }
                    items.push(GroupItem {
                        ordinal: item.ordinal,
                        operation_id: id,
                        classification: item.classification.clone(),
                        issue_codes: item.issue_codes.clone(),
                        source_rows: lineage(files, id)?,
                        outgoing: Some(item),
                        credit: None,
                    });
                }
                Ok(Capture {
                    core: NativeSnapshot::Outgoing(core),
                    payments: WorkflowSnapshot::Outgoing(payments),
                    items,
                })
            }
            SetKind::Credits => {
                let rows = files
                    .iter()
                    .flat_map(|f| match f {
                        CapturedFile::Credits(f) => f.rows.iter(),
                        _ => unreachable!("verified namespace"),
                    })
                    .cloned()
                    .collect::<Vec<CreditSettlementRow>>();
                let file_ids = rows.iter().map(|r| r.movement_id).collect::<Vec<_>>();
                let first = self
                    .core
                    .credit_accounting(range, &batch_ids, &file_ids)
                    .await?;
                let relevant = |s: &&ReportItem| {
                    matches!(
                        s.source.journal.posting_kind.as_str(),
                        "INBOUND_SUSPENSE" | "RETURN_SUSPENSE" | "CREDIT_ALLOCATION"
                    )
                };
                let ids = first
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
                let requested = ids.iter().copied().collect::<BTreeSet<_>>();
                let payments = self.payments.credit_snapshot(range, &ids).await?;
                let confirm = ids
                    .iter()
                    .copied()
                    .chain(payments.credits.iter().map(|p| p.movement_id))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let core = self
                    .core
                    .credit_accounting(range, &batch_ids, &confirm)
                    .await?;
                let ids = confirm
                    .into_iter()
                    .chain(core.facts.iter().map(|c| c.movement_id))
                    .collect::<BTreeSet<_>>();
                ensure!(ids.len() <= 1000, SnapshotLimitExceeded);
                let lines = duplicates(rows.iter().map(|r| r.line_id));
                let evidence = duplicates(rows.iter().map(|r| r.evidence_id));
                let by_core = core
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
                    let item = credit_reconciliation::inspect(
                        id,
                        n as i32 + 1,
                        credit_reconciliation::Inspection {
                            core: by_core.get(&id).copied(),
                            payment: by_payment.get(&id).copied(),
                            sources: sources
                                .iter()
                                .filter(relevant)
                                .filter(|s| s.source.journal.operation_id == id)
                                .collect(),
                            file: ClearingRows {
                                rows: &rows,
                                range,
                                generated_at: manifest.generated_at,
                            },
                            payments_as_of: payments.as_of,
                            payments_requested: requested.contains(&id),
                            duplicate_lines: &lines,
                            duplicate_evidence: &evidence,
                        },
                    )?;
                    items.push(GroupItem {
                        ordinal: item.ordinal,
                        operation_id: id,
                        classification: item.classification.clone(),
                        issue_codes: item.issue_codes.clone(),
                        source_rows: lineage(files, id)?,
                        outgoing: None,
                        credit: Some(item),
                    });
                }
                Ok(Capture {
                    core: NativeSnapshot::Credits(core),
                    payments: WorkflowSnapshot::Credits(payments),
                    items,
                })
            }
        }
    }
}

impl Finance {
    pub async fn create_group_reconciliation(
        &self,
        actor: &ActorContext,
        key: Uuid,
        input: GroupInput,
    ) -> Result<GroupDetail> {
        self.authorize_staff(actor, true).await?;
        ensure!(
            !key.is_nil() && !input.report_id.is_nil() && !input.set_id.is_nil(),
            ReconciliationInputError
        );
        let request_digest = fingerprint(&input)?;
        if let Some(row)=sqlx::query("SELECT id,request_digest FROM finance.group_reconciliation_runs WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&self.pool).await? {
            ensure!(row.get::<String,_>("request_digest")==request_digest,ReconciliationKeyConflict);return self.group_reconciliation_detail(actor,row.get("id")).await;
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
        let (manifest, manifest_packet) = self.client.settlement_set(input.set_id).await?;
        ensure!(
            manifest.range == report.report.range,
            ReconciliationInputError
        );
        let mut packets = vec![];
        for member in &manifest.members {
            let packet = match manifest.kind {
                SetKind::Outgoing => self.client.settlement_file(member.file_id).await?.1,
                SetKind::Credits => self.client.credit_settlement_file(member.file_id).await?.1,
            };
            packets.push(packet);
        }
        let files = parse_archives(&manifest, &packets).map_err(|_| SettlementUnavailable)?;
        let capture = self.capture_group(&manifest, &files, &sources).await?;
        let (core_as_of, payments_as_of) = capture.clocks()?;
        let count = |s: &str| {
            capture
                .items
                .iter()
                .filter(|i| i.classification == s)
                .count() as i32
        };
        let (principal, bank_fee, allocated, suspense) = match &capture.core {
            NativeSnapshot::Outgoing(c) => {
                let booked = c
                    .facts
                    .iter()
                    .filter(|c| c.outcome == "SETTLED")
                    .collect::<Vec<_>>();
                (
                    booked
                        .iter()
                        .map(|c| i128::from(c.principal.minor_units()))
                        .sum::<i128>()
                        .to_string(),
                    Some(
                        booked
                            .iter()
                            .map(|c| i128::from(c.fee.minor_units()))
                            .sum::<i128>()
                            .to_string(),
                    ),
                    None,
                    None,
                )
            }
            NativeSnapshot::Credits(c) => {
                let total = |allocated: Option<bool>| {
                    c.facts
                        .iter()
                        .filter(|c| allocated.is_none_or(|a| c.allocation_batch_id.is_some() == a))
                        .map(|c| i128::from(c.principal.minor_units()))
                        .sum::<i128>()
                        .to_string()
                };
                (
                    total(None),
                    None,
                    Some(total(Some(true))),
                    Some(total(Some(false))),
                )
            }
        };
        let matched = capture
            .items
            .iter()
            .filter(|i| i.classification == "MATCHED")
            .filter_map(|i| {
                i.outgoing
                    .as_ref()
                    .and_then(|o| o.core_principal)
                    .or_else(|| i.credit.as_ref().and_then(|c| c.principal))
            })
            .map(|m| i128::from(m.minor_units()))
            .sum::<i128>()
            .to_string();
        let mut totals = BTreeMap::<(String, u8), (i128, i128)>::new();
        for f in &files {
            let values = match f {
                CapturedFile::Outgoing(f) => f
                    .rows
                    .iter()
                    .map(|r| {
                        (
                            &r.currency_code,
                            r.currency_scale,
                            &r.principal_minor,
                            &r.provider_fee_minor,
                        )
                    })
                    .collect::<Vec<_>>(),
                CapturedFile::Credits(f) => f
                    .rows
                    .iter()
                    .map(|r| {
                        (
                            &r.currency_code,
                            r.currency_scale,
                            &r.principal_minor,
                            &r.provider_fee_minor,
                        )
                    })
                    .collect::<Vec<_>>(),
            };
            for (code, scale, p, fee) in values {
                let total = totals.entry((code.clone(), scale)).or_default();
                total.0 += i128::from(p.parse::<i64>()?);
                total.1 += i128::from(fee.parse::<i64>()?);
            }
        }
        let summary = GroupSummary {
            id: Uuid::new_v4(),
            format_version: 1,
            kind: manifest.kind,
            scope: match manifest.kind {
                SetKind::Outgoing => "OUTGOING_PAYMENT_GROUP_V1",
                SetKind::Credits => "INCOMING_RETURN_GROUP_V1",
            }
            .into(),
            report_id: input.report_id,
            report_digest: report.digest,
            set_id: input.set_id,
            set_digest: fingerprint(&manifest)?,
            members: manifest.members.clone(),
            range: manifest.range,
            provider_row_count: manifest.row_count,
            core_digest: fingerprint(&capture.core)?,
            payments_digest: fingerprint(&capture.payments)?,
            items_digest: fingerprint(&capture.items)?,
            report_as_of: report.report.core_as_of,
            core_as_of,
            payments_as_of,
            provider_cutoff: manifest.generated_at,
            created_at: Utc::now(),
            record_count: capture.items.len() as i32,
            matched_count: count("MATCHED"),
            exception_count: count("EXCEPTION"),
            timing_count: count("TIMING"),
            pending_count: count("PENDING"),
            suspense_count: count("SUSPENSE"),
            not_posted_count: count("NOT_POSTED"),
            core_principal_total: principal,
            core_bank_fee_total: bank_fee,
            core_allocated_principal_total: allocated,
            core_suspense_principal_total: suspense,
            matched_principal_total: matched,
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
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,175))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        if let Some(row)=sqlx::query("SELECT id,request_digest FROM finance.group_reconciliation_runs WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await? {
            ensure!(row.get::<String,_>("request_digest")==request_digest,ReconciliationKeyConflict);let id=row.get("id");tx.commit().await?;return self.group_reconciliation_detail(actor,id).await;
        }
        sqlx::query("INSERT INTO finance.group_reconciliation_runs(id,report_id,set_id,issuer,subject,client_id,request_key,request_digest,document,digest,manifest_packet,file_packets,core_snapshot,payments_snapshot) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)").bind(summary.id).bind(summary.report_id).bind(summary.set_id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(request_digest).bind(serde_json::to_value(&summary)?).bind(fingerprint(&summary)?).bind(serde_json::to_value(manifest_packet)?).bind(serde_json::to_value(packets)?).bind(serde_json::to_value(&capture.core)?).bind(serde_json::to_value(&capture.payments)?).execute(&mut *tx).await?;
        for item in &capture.items {
            sqlx::query("INSERT INTO finance.group_reconciliation_items(run_id,ordinal,operation_id,document,digest) VALUES($1,$2,$3,$4,$5)").bind(summary.id).bind(item.ordinal).bind(item.operation_id).bind(serde_json::to_value(item)?).bind(fingerprint(item)?).execute(&mut *tx).await?;
            for code in &item.issue_codes {
                if !capture.grace(item.operation_id, code, summary.created_at) {
                    sqlx::query("INSERT INTO finance.group_reconciliation_issues(id,run_id,ordinal,code) VALUES($1,$2,$3,$4)").bind(Uuid::new_v4()).bind(summary.id).bind(item.ordinal).bind(code).execute(&mut *tx).await?;
                }
            }
        }
        Self::audit_in(
            &mut tx,
            actor,
            "finance.group_reconciliation.saved",
            summary.id,
        )
        .await?;
        tx.commit().await?;
        self.recover_group_reconciliation_cases().await?;
        self.group_reconciliation_detail(actor, summary.id).await
    }
    async fn verified_group(&self, id: Uuid) -> Result<(GroupSummary, String, Vec<GroupItem>)> {
        let row = sqlx::query("SELECT * FROM finance.group_reconciliation_runs WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let summary: GroupSummary = serde_json::from_value(row.get("document"))?;
        let digest: String = row.get("digest");
        ensure!(
            summary.id == id
                && summary.format_version == 1
                && fingerprint(&summary)? == digest
                && summary.report_id == row.get::<Uuid, _>("report_id")
                && summary.set_id == row.get::<Uuid, _>("set_id"),
            "Grouped reconciliation artifact identity mismatch"
        );
        let manifest_packet: SignedDocument = serde_json::from_value(row.get("manifest_packet"))?;
        manifest_packet.verify_integrity(crate::protocol::signing_key().as_bytes())?;
        let reply: SetReply = serde_json::from_value(manifest_packet.body)?;
        reply.validate()?;
        let manifest = reply.manifest;
        ensure!(
            manifest.set_id == summary.set_id
                && manifest.kind == summary.kind
                && manifest.range == summary.range
                && manifest.generated_at == summary.provider_cutoff
                && fingerprint(&manifest)? == summary.set_digest
                && fingerprint(&manifest.members)? == fingerprint(&summary.members)?
                && manifest.row_count == summary.provider_row_count,
            "Grouped manifest integrity mismatch"
        );
        let expected_scope = match manifest.kind {
            SetKind::Outgoing => "OUTGOING_PAYMENT_GROUP_V1",
            SetKind::Credits => "INCOMING_RETURN_GROUP_V1",
        };
        ensure!(summary.scope == expected_scope, "Grouped scope mismatch");
        let packets: Vec<SignedDocument> = serde_json::from_value(row.get("file_packets"))?;
        let files = parse_archives(&manifest, &packets)?;
        let core: NativeSnapshot = serde_json::from_value(row.get("core_snapshot"))?;
        let payments: WorkflowSnapshot = serde_json::from_value(row.get("payments_snapshot"))?;
        let source_kind = match (&core, &payments) {
            (NativeSnapshot::Outgoing(_), WorkflowSnapshot::Outgoing(_)) => SetKind::Outgoing,
            (NativeSnapshot::Credits(_), WorkflowSnapshot::Credits(_)) => SetKind::Credits,
            _ => anyhow::bail!("Grouped private source namespace mismatch"),
        };
        ensure!(
            source_kind == manifest.kind
                && fingerprint(&core)? == summary.core_digest
                && fingerprint(&payments)? == summary.payments_digest,
            "Grouped private source integrity mismatch"
        );
        let clocks = Capture {
            core,
            payments,
            items: vec![],
        }
        .clocks()?;
        ensure!(
            clocks == (summary.core_as_of, summary.payments_as_of),
            "Grouped source clock mismatch"
        );
        let parent = sqlx::query("SELECT document,digest FROM finance.reports WHERE id=$1")
            .bind(summary.report_id)
            .fetch_one(&self.pool)
            .await?;
        let parent_doc: Value = parent.get("document");
        let parent_range: ReportRange = serde_json::from_value(parent_doc["range"].clone())?;
        ensure!(
            parent.get::<String, _>("digest") == summary.report_digest
                && fingerprint(&parent_doc)? == summary.report_digest
                && parent_range == manifest.range,
            "Grouped parent report integrity mismatch"
        );
        let rows=sqlx::query("SELECT ordinal,operation_id,document,digest FROM finance.group_reconciliation_items WHERE run_id=$1 ORDER BY ordinal").bind(id).fetch_all(&self.pool).await?;
        let mut items = vec![];
        for (n, row) in rows.into_iter().enumerate() {
            let item: GroupItem = serde_json::from_value(row.get("document"))?;
            ensure!(
                item.ordinal == n as i32 + 1
                    && item.ordinal == row.get::<i32, _>("ordinal")
                    && item.operation_id == row.get::<Uuid, _>("operation_id")
                    && fingerprint(&item)? == row.get::<String, _>("digest"),
                "Grouped item identity or integrity mismatch"
            );
            let valid = match (&item.outgoing, &item.credit, manifest.kind) {
                (Some(o), None, SetKind::Outgoing) => {
                    o.payment_id == item.operation_id
                        && o.ordinal == item.ordinal
                        && o.classification == item.classification
                        && o.issue_codes == item.issue_codes
                }
                (None, Some(c), SetKind::Credits) => {
                    c.movement_id == item.operation_id
                        && c.ordinal == item.ordinal
                        && c.classification == item.classification
                        && c.issue_codes == item.issue_codes
                }
                _ => false,
            };
            let claims = source_claims(&files, item.operation_id)?;
            let claims_match = match (&item.outgoing, &item.credit) {
                (Some(o), None) => fingerprint(&o.provider_rows)? == fingerprint(&claims)?,
                (None, Some(c)) => fingerprint(&c.provider_rows)? == fingerprint(&claims)?,
                _ => false,
            };
            ensure!(
                valid
                    && claims_match
                    && fingerprint(&item.source_rows)?
                        == fingerprint(&lineage(&files, item.operation_id)?)?,
                "Grouped item source lineage mismatch"
            );
            items.push(item);
        }
        ensure!(
            items.len() == summary.record_count as usize
                && fingerprint(&items)? == summary.items_digest,
            "Grouped item membership mismatch"
        );
        Ok((summary, digest, items))
    }
    pub async fn group_reconciliation_detail(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<GroupDetail> {
        self.authorize_staff(actor, false).await?;
        let (reconciliation, digest, _) = self.verified_group(id).await?;
        let issues=sqlx::query("SELECT * FROM finance.group_reconciliation_issues WHERE run_id=$1 ORDER BY ordinal,code,id").bind(id).fetch_all(&self.pool).await?.into_iter().map(|r|ReconciliationIssue{id:r.get("id"),ordinal:r.get("ordinal"),code:r.get("code"),case_id:r.get("case_id")}).collect();
        Ok(GroupDetail {
            reconciliation,
            digest,
            issues,
        })
    }
    pub async fn group_reconciliations(
        &self,
        actor: &ActorContext,
        report: Option<Uuid>,
    ) -> Result<Vec<GroupSummary>> {
        self.authorize_staff(actor, false).await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.group_reconciliation_runs WHERE ($1::uuid IS NULL OR report_id=$1) ORDER BY created_at DESC,id LIMIT 100").bind(report).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.verified_group(id).await?.0);
        }
        Ok(out)
    }
    pub async fn group_reconciliation_items(
        &self,
        actor: &ActorContext,
        id: Uuid,
        after: i32,
        limit: i64,
    ) -> Result<GroupPage> {
        self.authorize_staff(actor, false).await?;
        let (summary, digest, items) = self.verified_group(id).await?;
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
        Ok(GroupPage {
            items,
            total: summary.record_count,
            next_after: (last < summary.record_count).then_some(last),
            reconciliation_digest: digest,
        })
    }
    pub async fn recover_group_reconciliation_cases(&self) -> Result<()> {
        let after: Uuid = sqlx::query_scalar(
            "SELECT last_issue_id FROM finance.group_reconciliation_case_cursor WHERE id",
        )
        .fetch_one(&self.pool)
        .await?;
        let rows=sqlx::query("SELECT i.id,i.run_id,r.operation_id FROM finance.group_reconciliation_issues i JOIN finance.group_reconciliation_items r ON r.run_id=i.run_id AND r.ordinal=i.ordinal WHERE i.case_id IS NULL ORDER BY (i.id<=$1),i.id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        let cursor = rows
            .last()
            .map(|r| r.get::<Uuid, _>("id"))
            .unwrap_or(Uuid::nil());
        sqlx::query(
            "UPDATE finance.group_reconciliation_case_cursor SET last_issue_id=$1 WHERE id",
        )
        .bind(cursor)
        .execute(&self.pool)
        .await?;
        for row in rows {
            let id: Uuid = row.get("id");
            let run: Uuid = row.get("run_id");
            let operation: Uuid = row.get("operation_id");
            let owned = Cases::new(self.controls.pool.clone())
                .ensure(
                    &service_actor(),
                    NewCase {
                        kind: CaseKind::Finance,
                        origin_id: id,
                        resource_id: operation,
                        evidence_refs: vec![run, operation, id],
                        customer: None,
                    },
                )
                .await;
            if let Ok(case) = owned {
                sqlx::query("UPDATE finance.group_reconciliation_issues SET case_id=$2 WHERE id=$1 AND case_id IS NULL").bind(id).bind(case).execute(&self.pool).await?;
            } else {
                tracing::warn!(code="GROUP_RECONCILIATION_CASE_ASSIGNMENT_PENDING",issue_id=%id,"Grouped Finance ownership remains pending; continuing cursor");
            }
        }
        Ok(())
    }
}
