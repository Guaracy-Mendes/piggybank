//! Immutable observation artifacts. A report never books money or upgrades missing evidence.
use crate::{
    Finance,
    chart::ChartVersion,
    liquidity::LiquiditySnapshot,
    protocol::{GlBatch, GlEvidence, GlStatus},
    service_actor,
};
use anyhow::{Result, ensure};
use bank_controls::cases::{CaseKind, Cases, NewCase};
use bank_core::{
    finance::FinancialBatch,
    finance_snapshot::{FinancialSnapshot, PositionSnapshot, ReportRange},
};
use bank_platform::{
    authenticity::SignedDocument,
    domain::{ActorContext, Currency, fingerprint},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, postgres::PgRow};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
#[derive(Debug)]
pub struct ReportKeyConflict;
impl std::fmt::Display for ReportKeyConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Report key already binds a different reporting period")
    }
}
impl std::error::Error for ReportKeyConflict {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountTotal {
    pub account_id: Uuid,
    pub kind: String,
    pub debit: String,
    pub credit: String,
    pub net: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportItem {
    pub ordinal: i32,
    pub source: FinancialBatch,
    pub source_digest: String,
    pub export_id: Option<Uuid>,
    pub gl_state: Option<String>,
    pub batch: Option<GlBatch>,
    pub receipt: Option<GlEvidence>,
    pub classification: String,
    pub issue_codes: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportSummary {
    pub format_version: i32,
    pub id: Uuid,
    pub range: ReportRange,
    pub currency: Currency,
    pub core_as_of: DateTime<Utc>,
    pub finance_as_of: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub core_digest: String,
    pub items_digest: String,
    pub source_count: i32,
    pub matched_count: i32,
    pub missing_count: i32,
    pub pending_count: i32,
    pub conflicting_count: i32,
    pub core_debit_total: String,
    pub core_credit_total: String,
    pub confirmed_gl_debit_total: String,
    pub confirmed_gl_credit_total: String,
    pub accounts: Vec<AccountTotal>,
    pub positions: PositionSnapshot,
    pub liquidity: LiquiditySnapshot,
    pub completeness: String,
    pub gl_completeness: String,
    pub reason_codes: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportIssue {
    pub id: Uuid,
    pub ordinal: Option<i32>,
    pub resource_id: Uuid,
    pub code: String,
    pub case_id: Option<Uuid>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDetail {
    pub report: ReportSummary,
    pub digest: String,
    pub issues: Vec<ReportIssue>,
    pub current_liquidity_status: String,
    pub revisions: Vec<crate::revisions::PublishedRevision>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPage {
    pub items: Vec<ReportItem>,
    pub total: i32,
    pub next_after: Option<i32>,
    pub report_digest: String,
}

fn inspect(
    source: &FinancialBatch,
    row: Option<&PgRow>,
    ordinal: i32,
) -> Result<(ReportItem, Option<Value>)> {
    let mut item = ReportItem {
        ordinal,
        source: source.clone(),
        source_digest: fingerprint(source)?,
        export_id: None,
        gl_state: None,
        batch: None,
        receipt: None,
        classification: "GL_MISSING".into(),
        issue_codes: vec![],
    };
    let Some(row) = row else {
        item.issue_codes.push("MISSING_GL_EXPORT".into());
        return Ok((item, None));
    };
    item.export_id = Some(row.get("id"));
    item.gl_state = Some(row.get("state"));
    let packet: Option<Value> = row.get("packet");
    let checked = (|| -> Result<()> {
        let batch: GlBatch = serde_json::from_value(row.get("document"))?;
        batch.validate()?;
        item.batch = Some(batch.clone());
        ensure!(
            batch.export_id == item.export_id.unwrap()
                && batch.source_batch_id == source.journal.id
                && batch.core_operation_id == source.journal.operation_id
                && batch.posting_kind == source.journal.posting_kind
                && batch.original_source_batch_id == source.journal.original_batch_id
                && batch.source_digest == item.source_digest,
            "GL_IDENTITY_MISMATCH"
        );
        if batch.business_date != source.journal.business_date {
            item.issue_codes.push("GL_DATE_MISMATCH".into());
        }
        let actual: BTreeMap<_, _> = batch
            .entries
            .iter()
            .map(|e| (e.core_account_id, e.delta))
            .collect();
        let expected: BTreeMap<_, _> = source
            .journal
            .entries
            .iter()
            .map(|e| (e.account_id.0, e.amount))
            .collect();
        if actual != expected {
            item.issue_codes.push("GL_VALUE_MISMATCH".into());
        }
        let chart: ChartVersion = serde_json::from_value(row.get("chart_document"))?;
        chart.validate()?;
        ensure!(
            chart.id == batch.chart_id
                && chart.version == batch.chart_version
                && chart.code == source.chart_code
                && batch.chart_digest == fingerprint(&chart)?
                && source.journal.business_date >= chart.effective_from
                && source.journal.business_date < chart.effective_until,
            "GL_MAPPING_MISMATCH"
        );
        for e in &batch.entries {
            let kind = source
                .account_kinds
                .get(&e.core_account_id)
                .ok_or_else(|| anyhow::anyhow!("GL_MAPPING_MISMATCH"))?;
            let code = chart.by_account.get(&e.core_account_id).or_else(|| {
                if kind == "CONTROL" {
                    None
                } else {
                    chart.by_kind.get(kind)
                }
            });
            ensure!(code == Some(&e.gl_account), "GL_MAPPING_MISMATCH");
            let class = chart.accounts.get(&e.gl_account).map(String::as_str);
            ensure!(
                match kind.as_str() {
                    "CUSTOMER" => class == Some("LIABILITY"),
                    "SETTLEMENT" => class == Some("ASSET"),
                    "CONTROL" => class.is_some(),
                    _ => false,
                },
                "GL_MAPPING_MISMATCH"
            );
        }
        if item.gl_state.as_deref() == Some("BOOKED") {
            let p: SignedDocument = serde_json::from_value(
                packet
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("GL_RECEIPT_MISSING"))?,
            )?;
            p.verify_integrity(crate::protocol::signing_key().as_bytes())?;
            let evidence: GlEvidence = serde_json::from_value(p.body)?;
            evidence.validate(&batch)?;
            ensure!(evidence.status == GlStatus::Booked, "GL_RECEIPT_CONFLICT");
            item.receipt = Some(evidence);
        } else {
            item.issue_codes.push("GL_EXPORT_PENDING".into());
        }
        Ok(())
    })();
    if let Err(e) = checked {
        let code = match e.to_string().as_str() {
            "GL_IDENTITY_MISMATCH" => "GL_IDENTITY_MISMATCH",
            "GL_MAPPING_MISMATCH" => "GL_MAPPING_MISMATCH",
            "GL_RECEIPT_MISSING" => "GL_RECEIPT_MISSING",
            _ => "GL_PAYLOAD_OR_RECEIPT_INVALID",
        };
        item.issue_codes.push(code.into());
    }
    item.classification = if item.issue_codes.is_empty() {
        "MATCHED"
    } else if item.issue_codes.iter().all(|c| c == "GL_EXPORT_PENDING") {
        "GL_PENDING"
    } else {
        "GL_CONFLICT"
    }
    .into();
    Ok((item, packet))
}
impl Finance {
    pub async fn create_report(
        &self,
        actor: &ActorContext,
        key: Uuid,
        range: ReportRange,
    ) -> Result<ReportDetail> {
        self.authorize_staff(actor, true).await?;
        range.validate()?;
        ensure!(!key.is_nil(), "Report request identity required");
        let request_digest = fingerprint(&range)?;
        if let Some(row)=sqlx::query("SELECT id,request_digest FROM finance.reports WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&self.pool).await? {
            ensure!(row.get::<String,_>("request_digest")==request_digest,ReportKeyConflict);return self.report_detail(actor,row.get("id")).await;
        }
        let core = self.core.snapshot(range).await?;
        let ids = core
            .batches
            .iter()
            .map(|s| s.journal.id)
            .collect::<Vec<_>>();
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let finance_as_of: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let rows=sqlx::query("SELECT e.id,e.source_batch_id,e.state,e.document,c.document AS chart_document,r.document AS packet FROM finance.gl_exports e JOIN finance.chart_versions c ON c.id=e.chart_id LEFT JOIN finance.gl_receipts r ON r.export_id=e.id WHERE e.source_batch_id=ANY($1)").bind(&ids).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        let mut by_source = BTreeMap::new();
        for row in &rows {
            by_source.insert(row.get::<Uuid, _>("source_batch_id"), row);
        }
        // Provider transport starts only after both database snapshots have closed.
        let observation = self.observe_liquidity(&core.positions).await?;
        let mut items = vec![];
        let mut packets = vec![];
        let mut accounts: BTreeMap<Uuid, (String, i128, i128)> = BTreeMap::new();
        let mut reasons = BTreeSet::new();
        let (mut debit, mut credit, mut matched_debit, mut matched_credit) =
            (0i128, 0i128, 0i128, 0i128);
        let (mut matched, mut missing, mut pending, mut conflicting) = (0, 0, 0, 0);
        for (n, source) in core.batches.iter().enumerate() {
            let (mut item, packet) = inspect(
                source,
                by_source.get(&source.journal.id).copied(),
                n as i32 + 1,
            )?;
            let (mut bd, mut bc) = (0i128, 0i128);
            for leg in &source.journal.entries {
                let total = accounts.entry(leg.account_id.0).or_insert((
                    source
                        .account_kinds
                        .get(&leg.account_id.0)
                        .cloned()
                        .unwrap_or_default(),
                    0,
                    0,
                ));
                let delta = i128::from(leg.amount.minor_units());
                if delta < 0 {
                    total.1 -= delta;
                    bd -= delta;
                } else {
                    total.2 += delta;
                    bc += delta;
                }
            }
            debit += bd;
            credit += bc;
            match item.classification.as_str() {
                "MATCHED" => {
                    matched += 1;
                    matched_debit += bd;
                    matched_credit += bc;
                }
                "GL_MISSING" => missing += 1,
                "GL_PENDING" => pending += 1,
                _ => conflicting += 1,
            }
            if ["GL_MISSING", "GL_PENDING"].contains(&item.classification.as_str())
                && (finance_as_of - source.journal.created_at).num_seconds() < 30
            {
                item.issue_codes.push("GL_TIMING_WINDOW".into());
            }
            reasons.extend(item.issue_codes.clone());
            items.push(item);
            packets.push(packet);
        }
        ensure!(debit == credit, "Report Core control totals must balance");
        if core.positions.ledger_control_total != "0" {
            reasons.insert("CORE_LEDGER_CONTROL_MISMATCH".into());
        }
        let gl_complete = missing == 0
            && pending == 0
            && conflicting == 0
            && core.positions.ledger_control_total == "0";
        if observation.view.status != "FRESH" {
            reasons.insert(format!("LIQUIDITY_{}", observation.view.status));
        }
        let complete = gl_complete && observation.view.status == "FRESH";
        let summary = ReportSummary {
            format_version: 1,
            id: Uuid::new_v4(),
            range,
            currency: Currency::synthetic(),
            core_as_of: core.positions.as_of,
            finance_as_of,
            created_at: Utc::now(),
            core_digest: fingerprint(&core)?,
            items_digest: fingerprint(&items)?,
            source_count: items.len() as i32,
            matched_count: matched,
            missing_count: missing,
            pending_count: pending,
            conflicting_count: conflicting,
            core_debit_total: debit.to_string(),
            core_credit_total: credit.to_string(),
            confirmed_gl_debit_total: matched_debit.to_string(),
            confirmed_gl_credit_total: matched_credit.to_string(),
            accounts: accounts
                .into_iter()
                .map(|(account_id, (kind, d, c))| AccountTotal {
                    account_id,
                    kind,
                    debit: d.to_string(),
                    credit: c.to_string(),
                    net: (c - d).to_string(),
                })
                .collect(),
            positions: core.positions,
            liquidity: observation.view,
            completeness: if complete { "COMPLETE" } else { "PARTIAL" }.into(),
            gl_completeness: if gl_complete { "COMPLETE" } else { "PARTIAL" }.into(),
            reason_codes: reasons.into_iter().collect(),
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,169))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        if let Some(row)=sqlx::query("SELECT id,request_digest FROM finance.reports WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await? {
            ensure!(row.get::<String,_>("request_digest")==request_digest,ReportKeyConflict);let id=row.get("id");tx.commit().await?;return self.report_detail(actor,id).await;
        }
        sqlx::query("INSERT INTO finance.reports(id,issuer,subject,client_id,request_key,request_digest,document,digest) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(summary.id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(request_digest).bind(serde_json::to_value(&summary)?).bind(fingerprint(&summary)?).execute(&mut *tx).await?;
        for (item, packet) in items.iter().zip(packets) {
            sqlx::query("INSERT INTO finance.report_items(report_id,ordinal,source_batch_id,document,digest,gl_packet) VALUES($1,$2,$3,$4,$5,$6)").bind(summary.id).bind(item.ordinal).bind(item.source.journal.id).bind(serde_json::to_value(item)?).bind(fingerprint(item)?).bind(packet).execute(&mut *tx).await?;
            if !item.issue_codes.iter().any(|c| c == "GL_TIMING_WINDOW") {
                for code in &item.issue_codes {
                    sqlx::query("INSERT INTO finance.report_issues(id,report_id,ordinal,resource_id,code) VALUES($1,$2,$3,$4,$5)").bind(Uuid::new_v4()).bind(summary.id).bind(item.ordinal).bind(item.source.journal.id).bind(code).execute(&mut *tx).await?;
                }
            }
        }
        if summary.positions.ledger_control_total != "0" {
            sqlx::query("INSERT INTO finance.report_issues(id,report_id,resource_id,code) VALUES($1,$2,$2,'CORE_LEDGER_CONTROL_MISMATCH')").bind(Uuid::new_v4()).bind(summary.id).execute(&mut *tx).await?;
        }
        if summary.liquidity.status != "FRESH" {
            sqlx::query("INSERT INTO finance.report_issues(id,report_id,resource_id,code) VALUES($1,$2,$2,$3)").bind(Uuid::new_v4()).bind(summary.id).bind(format!("LIQUIDITY_{}",summary.liquidity.status)).execute(&mut *tx).await?;
        }
        sqlx::query(
            "INSERT INTO finance.report_liquidity(report_id,document,observation) VALUES($1,$2,$3)",
        )
        .bind(summary.id)
        .bind(observation.packet)
        .bind(serde_json::to_value(&summary.liquidity)?)
        .execute(&mut *tx)
        .await?;
        Self::audit_in(&mut tx, actor, "finance.report.saved", summary.id).await?;
        tx.commit().await?;
        self.recover_report_cases().await?;
        self.report_detail(actor, summary.id).await
    }
    async fn report_summary(&self, id: Uuid) -> Result<(ReportSummary, String)> {
        let row = sqlx::query("SELECT document,digest FROM finance.reports WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let report: ReportSummary = serde_json::from_value(row.get("document"))?;
        let digest: String = row.get("digest");
        ensure!(
            report.id == id && report.format_version == 1 && fingerprint(&report)? == digest,
            "Report artifact integrity mismatch"
        );
        Ok((report, digest))
    }
    pub async fn staff_reports(&self, actor: &ActorContext) -> Result<Vec<ReportSummary>> {
        self.authorize_staff(actor, false).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM finance.reports ORDER BY created_at DESC,id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut items = vec![];
        for id in ids {
            items.push(self.report_summary(id).await?.0);
        }
        Ok(items)
    }
    pub async fn report_detail(&self, actor: &ActorContext, id: Uuid) -> Result<ReportDetail> {
        self.authorize_staff(actor, false).await?;
        let (report, digest) = self.report_summary(id).await?;
        let cash = sqlx::query(
            "SELECT document,observation FROM finance.report_liquidity WHERE report_id=$1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        ensure!(
            fingerprint(&cash.get::<Value, _>("observation"))? == fingerprint(&report.liquidity)?,
            "Report liquidity observation mismatch"
        );
        if let Some(expected) = &report.liquidity.evidence_digest {
            let packet: SignedDocument = serde_json::from_value(
                cash.get::<Option<Value>, _>("document")
                    .ok_or_else(|| anyhow::anyhow!("Report liquidity evidence missing"))?,
            )?;
            packet.verify_integrity(crate::liquidity::liquidity_key().as_bytes())?;
            let reply: crate::liquidity::LiquidityReply = serde_json::from_value(packet.body)?;
            reply.request.validate()?;
            reply.position.validate()?;
            ensure!(
                fingerprint(&reply.position)? == *expected
                    && Some(reply.position.evidence_id) == report.liquidity.evidence_id
                    && Some(reply.position.version) == report.liquidity.provider_version,
                "Report liquidity evidence lineage mismatch"
            );
        }
        let rows = sqlx::query(
            "SELECT document,digest FROM finance.report_items WHERE report_id=$1 ORDER BY ordinal",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let mut items = vec![];
        for row in rows {
            let item: ReportItem = serde_json::from_value(row.get("document"))?;
            ensure!(
                fingerprint(&item)? == row.get::<String, _>("digest")
                    && fingerprint(&item.source)? == item.source_digest,
                "Report lineage integrity mismatch"
            );
            items.push(item);
        }
        ensure!(
            items.len() == report.source_count as usize
                && fingerprint(&items)? == report.items_digest
                && fingerprint(&FinancialSnapshot {
                    range: report.range,
                    positions: report.positions.clone(),
                    batches: items.into_iter().map(|i| i.source).collect()
                })? == report.core_digest,
            "Report source membership mismatch"
        );
        let issues=sqlx::query("SELECT * FROM finance.report_issues WHERE report_id=$1 ORDER BY ordinal NULLS FIRST,code,id").bind(id).fetch_all(&self.pool).await?.into_iter().map(|r|ReportIssue{id:r.get("id"),ordinal:r.get("ordinal"),resource_id:r.get("resource_id"),code:r.get("code"),case_id:r.get("case_id")}).collect();
        let current = if report.liquidity.status == "FRESH"
            && (report
                .liquidity
                .valid_until
                .is_some_and(|t| t <= Utc::now())
                || (Utc::now() - report.core_as_of).num_seconds() > 5)
        {
            "STALE"
        } else {
            &report.liquidity.status
        };
        Ok(ReportDetail {
            revisions: self.published_revision_links(id).await?,
            current_liquidity_status: current.into(),
            report,
            digest,
            issues,
        })
    }
    pub async fn report_items(
        &self,
        actor: &ActorContext,
        id: Uuid,
        after: i32,
        limit: i64,
    ) -> Result<ReportPage> {
        self.authorize_staff(actor, false).await?;
        ensure!(
            after >= 0 && (1..=100).contains(&limit),
            "Bounded report cursor/page required"
        );
        let (report, digest) = self.report_summary(id).await?;
        ensure!(
            after <= report.source_count,
            "Report cursor outside snapshot"
        );
        let rows=sqlx::query("SELECT document,digest,gl_packet FROM finance.report_items WHERE report_id=$1 AND ordinal>$2 ORDER BY ordinal LIMIT $3").bind(id).bind(after).bind(limit).fetch_all(&self.pool).await?;
        let mut items = vec![];
        for row in rows {
            let item: ReportItem = serde_json::from_value(row.get("document"))?;
            ensure!(
                fingerprint(&item)? == row.get::<String, _>("digest"),
                "Report item integrity mismatch"
            );
            if item.classification == "MATCHED" {
                let p: SignedDocument = serde_json::from_value(
                    row.get::<Option<Value>, _>("gl_packet")
                        .ok_or_else(|| anyhow::anyhow!("Report receipt missing"))?,
                )?;
                p.verify_integrity(crate::protocol::signing_key().as_bytes())?;
                let evidence: GlEvidence = serde_json::from_value(p.body)?;
                evidence.validate(
                    item.batch
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Report GL batch missing"))?,
                )?;
                ensure!(
                    fingerprint(&evidence)? == fingerprint(&item.receipt)?,
                    "Report receipt mismatch"
                );
            }
            items.push(item);
        }
        let last = items.last().map(|i| i.ordinal).unwrap_or(after);
        Ok(ReportPage {
            items,
            total: report.source_count,
            next_after: if last < report.source_count {
                Some(last)
            } else {
                None
            },
            report_digest: digest,
        })
    }
    pub async fn recover_report_cases(&self) -> Result<()> {
        let rows=sqlx::query("SELECT * FROM finance.report_issues WHERE case_id IS NULL ORDER BY report_id,ordinal NULLS FIRST,id LIMIT 50").fetch_all(&self.pool).await?;
        for row in rows {
            let id: Uuid = row.get("id");
            let report: Uuid = row.get("report_id");
            let resource: Uuid = row.get("resource_id");
            let case = Cases::new(self.controls.pool.clone())
                .ensure(
                    &service_actor(),
                    NewCase {
                        kind: CaseKind::Finance,
                        origin_id: id,
                        resource_id: resource,
                        evidence_refs: vec![report, resource, id],
                        customer: None,
                    },
                )
                .await?;
            sqlx::query(
                "UPDATE finance.report_issues SET case_id=$2 WHERE id=$1 AND case_id IS NULL",
            )
            .bind(id)
            .bind(case)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }
}
