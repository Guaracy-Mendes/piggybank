use crate::{
    Finance,
    client::EvidenceConflict,
    protocol::{GlBatch, GlEntry, GlStatus, VerifiedGl},
    service_actor,
};
use anyhow::{Result, ensure};
use bank_controls::cases::{CaseKind, Cases, NewCase};
use bank_platform::{
    domain::fingerprint,
    operations::{JobSpec, OperationStatus},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStatus {
    pub id: Uuid,
    pub source_batch_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub state: String,
    pub reason_code: String,
    pub attempts: i32,
    pub retry_generation: i64,
    pub attempt_limit: i32,
    pub batch: GlBatch,
    pub receipt: Option<crate::protocol::GlEvidence>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Progress {
    Done { export_id: Uuid },
    Pending,
    Superseded { export_id: Uuid },
    Review { code: String },
}

impl Finance {
    /// Durable identity is the original Core batch, independent of event/file delivery identity.
    pub async fn stage_export(&self, source_id: Uuid) -> Result<ExportStatus> {
        if let Some(id) =
            sqlx::query_scalar("SELECT id FROM finance.gl_exports WHERE source_batch_id=$1")
                .bind(source_id)
                .fetch_optional(&self.pool)
                .await?
        {
            let status = self.export_status(id).await?;
            self.match_break(source_id, "SOURCE_MAPPING_MISSING")
                .await?;
            return Ok(status);
        }
        let source = self.core.financial_batch(source_id).await?;
        ensure!(
            source.journal.id == source_id,
            "Core source identity mismatch"
        );
        let chart = self
            .chart_at(&source.chart_code, source.journal.business_date)
            .await?;
        let mut entries = vec![];
        for leg in &source.journal.entries {
            let kind = source
                .account_kinds
                .get(&leg.account_id.0)
                .ok_or_else(|| anyhow::anyhow!("Missing Core account classification"))?;
            let code = chart
                .by_account
                .get(&leg.account_id.0)
                .or_else(|| {
                    if kind == "CONTROL" {
                        None
                    } else {
                        chart.by_kind.get(kind)
                    }
                })
                .ok_or(crate::chart::MappingUnavailable)?;
            let class = chart
                .accounts
                .get(code)
                .ok_or(crate::chart::MappingUnavailable)?;
            ensure!(
                match kind.as_str() {
                    "CUSTOMER" => class == "LIABILITY",
                    "SETTLEMENT" => class == "ASSET",
                    "CONTROL" => true,
                    _ => false,
                },
                crate::chart::MappingUnavailable
            );
            entries.push(GlEntry {
                core_account_id: leg.account_id.0,
                gl_account: code.clone(),
                delta: leg.amount,
            });
        }
        let (debit_total, credit_total) = GlBatch::totals(&entries);
        let batch = GlBatch {
            export_id: Uuid::new_v4(),
            source_batch_id: source_id,
            original_source_batch_id: source.journal.original_batch_id,
            core_operation_id: source.journal.operation_id,
            posting_kind: source.journal.posting_kind.clone(),
            business_date: source.journal.business_date,
            chart_id: chart.id,
            chart_version: chart.version,
            chart_digest: fingerprint(&chart)?,
            source_digest: fingerprint(&source)?,
            debit_total,
            credit_total,
            entries,
        };
        batch.validate()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,165))")
            .bind(source_id.to_string())
            .execute(&mut *tx)
            .await?;
        let inserted=sqlx::query("INSERT INTO finance.gl_exports(id,source_batch_id,chart_id,document,fingerprint) VALUES($1,$2,$3,$4,$5) ON CONFLICT(source_batch_id) DO NOTHING")
            .bind(batch.export_id).bind(source_id).bind(chart.id).bind(serde_json::to_value(&batch)?).bind(fingerprint(&batch)?).execute(&mut *tx).await?.rows_affected()==1;
        let id = sqlx::query_scalar("SELECT id FROM finance.gl_exports WHERE source_batch_id=$1")
            .bind(source_id)
            .fetch_one(&mut *tx)
            .await?;
        if inserted {
            Self::audit_in(&mut tx, &service_actor(), "finance.export.staged", id).await?;
        }
        tx.commit().await?;
        self.match_break(source_id, "SOURCE_MAPPING_MISSING")
            .await?;
        self.export_status(id).await
    }
    pub async fn export_status(&self, id: Uuid) -> Result<ExportStatus> {
        let row=sqlx::query("SELECT e.*,r.document AS receipt FROM finance.gl_exports e LEFT JOIN finance.gl_receipts r ON r.export_id=e.id WHERE e.id=$1").bind(id).fetch_one(&self.pool).await?;
        let batch: GlBatch = serde_json::from_value(row.get("document"))?;
        batch.validate()?;
        let receipt = if let Some(value) = row.get::<Option<Value>, _>("receipt") {
            let packet: bank_platform::authenticity::SignedDocument =
                serde_json::from_value(value)?;
            packet.verify_integrity(crate::protocol::signing_key().as_bytes())?;
            let evidence: crate::protocol::GlEvidence = serde_json::from_value(packet.body)?;
            evidence.validate(&batch)?;
            Some(evidence)
        } else {
            None
        };
        let state: String = row.get("state");
        ensure!(
            (state == "BOOKED") == receipt.is_some(),
            "GL state requires an authenticated receipt"
        );
        Ok(ExportStatus {
            id,
            source_batch_id: row.get("source_batch_id"),
            operation_id: row.get("operation_id"),
            state,
            reason_code: row.get("reason_code"),
            attempts: row.get("attempts"),
            retry_generation: row.get("retry_generation"),
            attempt_limit: row.get("attempt_limit"),
            batch,
            receipt,
            updated_at: row.get("updated_at"),
        })
    }
    pub async fn schedule_export(&self, id: Uuid) -> Result<OperationStatus> {
        let status = self.export_status(id).await?;
        let op = self
            .operations
            .accept(
                &service_actor(),
                "finance.export",
                &id.to_string(),
                &status.batch,
                JobSpec {
                    kind: "finance.export".into(),
                    step_key: "export".into(),
                    payload: json!({"exportId":id,"generation":0}),
                    delay_seconds: 0,
                },
            )
            .await?;
        sqlx::query(
            "UPDATE finance.gl_exports SET operation_id=$2 WHERE id=$1 AND operation_id IS NULL",
        )
        .bind(id)
        .bind(op.id)
        .execute(&self.pool)
        .await?;
        Ok(op)
    }
    /// No transaction is held over HTTP. Provider strict source deduplication also fences late requests.
    pub async fn process_export(&self, id: Uuid) -> Result<Progress> {
        let status = self.export_status(id).await?;
        self.process_export_at(id, status.retry_generation).await
    }
    pub async fn process_export_at(&self, id: Uuid, generation: i64) -> Result<Progress> {
        let status = self.export_status(id).await?;
        if status.retry_generation != generation {
            return Ok(Progress::Superseded { export_id: id });
        }
        if status.state == "BOOKED" {
            self.match_break(status.source_batch_id, "GL_EXPORT_PENDING")
                .await?;
            return Ok(Progress::Done { export_id: id });
        }
        if status.state == "REVIEW_REQUIRED" {
            self.own_break(status.source_batch_id, "GL_EXPORT_PENDING")
                .await?;
            return Ok(Progress::Review {
                code: status.reason_code,
            });
        }
        if status.attempts >= status.attempt_limit {
            return self
                .review_export(
                    id,
                    status.source_batch_id,
                    generation,
                    "GL_TRANSPORT_BUDGET_EXHAUSTED",
                )
                .await;
        }
        let lease = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM finance.gl_exports WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        if row.get::<i64, _>("retry_generation") != generation {
            tx.commit().await?;
            return Ok(Progress::Superseded { export_id: id });
        }
        if row.get::<String, _>("state") == "BOOKED" {
            tx.commit().await?;
            return Ok(Progress::Done { export_id: id });
        }
        if row.get::<String, _>("state") == "REVIEW_REQUIRED" {
            tx.commit().await?;
            return Ok(Progress::Review {
                code: "GL_REVIEW_REQUIRED".into(),
            });
        }
        if row
            .get::<Option<DateTime<Utc>>, _>("lease_until")
            .is_some_and(|t| t > Utc::now())
        {
            tx.commit().await?;
            return Ok(Progress::Pending);
        }
        if row.get::<i32, _>("attempts") >= row.get::<i32, _>("attempt_limit") {
            tx.commit().await?;
            return self
                .review_export(
                    id,
                    status.source_batch_id,
                    generation,
                    "GL_TRANSPORT_BUDGET_EXHAUSTED",
                )
                .await;
        }
        let first = row
            .get::<Option<DateTime<Utc>>, _>("attempted_at")
            .is_none();
        let fence:i64=sqlx::query_scalar("UPDATE finance.gl_exports SET attempted_at=coalesce(attempted_at,now()),state='OUTCOME_UNKNOWN',reason_code=$3,attempts=attempts+1,lease_id=$2,lease_until=now()+interval '6 seconds',lease_fence=lease_fence+1,updated_at=now() WHERE id=$1 RETURNING lease_fence")
            .bind(id).bind(lease).bind(if first{"GL_BOOKING_REQUESTED"}else{"GL_INQUIRY_REQUESTED"}).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        let response = if first {
            self.client.book(&status.batch).await
        } else {
            match self.client.inquire(&status.batch).await {
                Ok(e) if e.evidence.status == GlStatus::NotFound => {
                    // An authenticated NOT_FOUND permits replay of this exact sealed batch only.
                    if !self
                        .observe(
                            id,
                            lease,
                            fence,
                            "NOT_FOUND",
                            &serde_json::to_value(&e.document)?,
                        )
                        .await?
                    {
                        return Ok(Progress::Pending);
                    }
                    self.client.book(&status.batch).await
                }
                other => other,
            }
        };
        match response {
            Ok(e) if e.evidence.status == GlStatus::Booked => {
                self.save_receipt(&status.batch, lease, fence, e).await
            }
            Ok(e) => {
                self.observe(
                    id,
                    lease,
                    fence,
                    "NOT_FOUND",
                    &serde_json::to_value(e.document)?,
                )
                .await?;
                self.release_lease(id, lease, fence).await?;
                Ok(Progress::Pending)
            }
            Err(error) => {
                if let Some(conflict) = error.downcast_ref::<EvidenceConflict>() {
                    let owns = self
                        .observe(id, lease, fence, "QUARANTINED", &conflict.document)
                        .await?;
                    if owns {
                        return self
                            .review_export_fenced(
                                id,
                                status.source_batch_id,
                                lease,
                                fence,
                                "GL_EVIDENCE_CONFLICT",
                            )
                            .await;
                    }
                }
                self.release_lease(id, lease, fence).await?;
                Ok(Progress::Pending)
            }
        }
    }
    async fn observe(
        &self,
        id: Uuid,
        lease: Uuid,
        fence: i64,
        kind: &str,
        doc: &Value,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let owns:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM finance.gl_exports WHERE id=$1 AND lease_id=$2 AND lease_fence=$3 AND lease_until>now() AND state='OUTCOME_UNKNOWN' FOR UPDATE)").bind(id).bind(lease).bind(fence).fetch_one(&mut *tx).await?;
        if owns {
            sqlx::query("INSERT INTO finance.gl_observations(id,export_id,lease_fence,kind,document,digest) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(export_id,digest) DO NOTHING").bind(Uuid::new_v4()).bind(id).bind(fence).bind(kind).bind(doc).bind(fingerprint(doc)?).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(owns)
    }
    async fn release_lease(&self, id: Uuid, lease: Uuid, fence: i64) -> Result<()> {
        sqlx::query("UPDATE finance.gl_exports SET lease_id=NULL,lease_until=NULL,reason_code='GL_OUTCOME_UNKNOWN',updated_at=now() WHERE id=$1 AND lease_id=$2 AND lease_fence=$3 AND state='OUTCOME_UNKNOWN'").bind(id).bind(lease).bind(fence).execute(&self.pool).await?;
        Ok(())
    }
    async fn save_receipt(
        &self,
        batch: &GlBatch,
        lease: Uuid,
        fence: i64,
        e: VerifiedGl,
    ) -> Result<Progress> {
        e.evidence.validate(batch)?;
        ensure!(
            e.evidence.status == GlStatus::Booked,
            "GL booking receipt required"
        );
        let mut tx = self.pool.begin().await?;
        let owns=sqlx::query("UPDATE finance.gl_exports SET state='BOOKED',reason_code='GL_BOOKED',lease_id=NULL,lease_until=NULL,updated_at=now() WHERE id=$1 AND lease_id=$2 AND lease_fence=$3 AND lease_until>now() AND state='OUTCOME_UNKNOWN'")
            .bind(batch.export_id).bind(lease).bind(fence).execute(&mut *tx).await?.rows_affected()==1;
        if !owns {
            tx.commit().await?;
            return Ok(Progress::Pending);
        }
        sqlx::query("INSERT INTO finance.gl_receipts(export_id,evidence_id,digest,document) VALUES($1,$2,$3,$4)").bind(batch.export_id).bind(e.evidence.evidence_id).bind(fingerprint(&e.evidence)?).bind(serde_json::to_value(e.document)?).execute(&mut *tx).await?;
        Self::audit_in(
            &mut tx,
            &service_actor(),
            "finance.export.booked",
            batch.export_id,
        )
        .await?;
        tx.commit().await?;
        self.match_break(batch.source_batch_id, "GL_EXPORT_PENDING")
            .await?;
        Ok(Progress::Done {
            export_id: batch.export_id,
        })
    }
    async fn review_export_fenced(
        &self,
        id: Uuid,
        source: Uuid,
        lease: Uuid,
        fence: i64,
        code: &str,
    ) -> Result<Progress> {
        let changed=sqlx::query("UPDATE finance.gl_exports SET state='REVIEW_REQUIRED',reason_code=$4,lease_id=NULL,lease_until=NULL,updated_at=now() WHERE id=$1 AND lease_id=$2 AND lease_fence=$3 AND lease_until>now() AND state='OUTCOME_UNKNOWN'").bind(id).bind(lease).bind(fence).bind(code).execute(&self.pool).await?.rows_affected()==1;
        if changed {
            self.own_break(source, "GL_EXPORT_PENDING").await?;
            Ok(Progress::Review { code: code.into() })
        } else {
            Ok(Progress::Pending)
        }
    }
    async fn review_export(
        &self,
        id: Uuid,
        source: Uuid,
        generation: i64,
        code: &str,
    ) -> Result<Progress> {
        sqlx::query("UPDATE finance.gl_exports SET state='REVIEW_REQUIRED',reason_code=$2,updated_at=now() WHERE id=$1 AND retry_generation=$3 AND state NOT IN ('BOOKED','REVIEW_REQUIRED') AND (lease_until IS NULL OR lease_until<=now())").bind(id).bind(code).bind(generation).execute(&self.pool).await?;
        let status = self.export_status(id).await?;
        if status.retry_generation != generation {
            return Ok(Progress::Superseded { export_id: id });
        }
        ensure!(
            status.source_batch_id == source,
            "Finance review scope conflict"
        );
        if status.state == "BOOKED" {
            return Ok(Progress::Done { export_id: id });
        }
        if status.state != "REVIEW_REQUIRED" {
            return Ok(Progress::Pending);
        }
        self.own_break(source, "GL_EXPORT_PENDING").await?;
        Ok(Progress::Review {
            code: status.reason_code,
        })
    }
    pub(crate) async fn own_break(&self, source: Uuid, code: &str) -> Result<Uuid> {
        let mut tx = self.pool.begin().await?;
        let created=sqlx::query("INSERT INTO finance.export_breaks(id,source_batch_id,code,state) VALUES($1,$2,$3,'OPEN') ON CONFLICT(source_batch_id,code) DO NOTHING").bind(Uuid::new_v4()).bind(source).bind(code).execute(&mut *tx).await?.rows_affected()==1;
        let id:Uuid=sqlx::query_scalar("UPDATE finance.export_breaks SET state='OPEN',last_seen_at=now(),matched_at=NULL WHERE source_batch_id=$1 AND code=$2 RETURNING id").bind(source).bind(code).fetch_one(&mut *tx).await?;
        if created {
            Self::audit_in(&mut tx, &service_actor(), "finance.break.opened", id).await?;
        }
        tx.commit().await?;
        self.ensure_break_case(id, source).await?;
        Ok(id)
    }
    async fn ensure_break_case(&self, id: Uuid, source: Uuid) -> Result<()> {
        let case = Cases::new(self.controls.pool.clone())
            .ensure(
                &service_actor(),
                NewCase {
                    kind: CaseKind::Finance,
                    origin_id: id,
                    resource_id: source,
                    evidence_refs: vec![source, id],
                    customer: None,
                },
            )
            .await?;
        sqlx::query("UPDATE finance.export_breaks SET case_id=$2 WHERE id=$1 AND case_id IS NULL")
            .bind(id)
            .bind(case)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    async fn match_break(&self, source: Uuid, code: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let id:Option<Uuid>=sqlx::query_scalar("UPDATE finance.export_breaks SET state='MATCHED',matched_at=now(),last_seen_at=now() WHERE source_batch_id=$1 AND code=$2 AND state='OPEN' RETURNING id").bind(source).bind(code).fetch_optional(&mut *tx).await?;
        if let Some(id) = id {
            Self::audit_in(&mut tx, &service_actor(), "finance.break.matched", id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    /// Bounded round-robin scans repair crashes between discovery, staging, scheduling and ACK.
    pub async fn recover(&self) -> Result<()> {
        let after: Uuid =
            sqlx::query_scalar("SELECT last_source_id FROM finance.export_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let ids = self.core.batch_ids(after).await?;
        sqlx::query("UPDATE finance.export_cursor SET last_source_id=$1 WHERE id")
            .bind(ids.last().copied().unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        for source in ids {
            if let Err(e) = self.stage_export(source).await {
                if e.downcast_ref::<crate::chart::MappingUnavailable>()
                    .is_some()
                {
                    self.own_break(source, "SOURCE_MAPPING_MISSING").await?;
                } else {
                    return Err(e);
                }
            }
        }
        let after: Uuid =
            sqlx::query_scalar("SELECT last_export_id FROM finance.export_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.gl_exports e WHERE id>$1 AND (state<>'BOOKED' OR operation_id IS NULL OR EXISTS(SELECT 1 FROM finance.retry_commands r WHERE r.export_id=e.id AND r.operation_id IS NULL)) ORDER BY id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        sqlx::query("UPDATE finance.export_cursor SET last_export_id=$1 WHERE id")
            .bind(ids.last().copied().unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        for id in ids {
            let original = self.schedule_export(id).await?;
            // Older accepted recovery commands may have executed before their
            // scheduling acknowledgment was linked. Recover every original job identity.
            let pending:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.retry_commands WHERE export_id=$1 AND operation_id IS NULL ORDER BY generation LIMIT 50").bind(id).fetch_all(&self.pool).await?;
            for request in pending {
                self.schedule_retry(request).await?;
            }
            let status = self.export_status(id).await?;
            let op = if status.retry_generation == 0 {
                original
            } else {
                let request: Uuid = sqlx::query_scalar(
                    "SELECT id FROM finance.retry_commands WHERE export_id=$1 AND generation=$2",
                )
                .bind(id)
                .bind(status.retry_generation)
                .fetch_one(&self.pool)
                .await?;
                self.schedule_retry(request).await?
            };
            if op.state == "REVIEW_REQUIRED" {
                self.review_export(
                    id,
                    status.source_batch_id,
                    status.retry_generation,
                    "GL_JOB_BUDGET_EXHAUSTED",
                )
                .await?;
            }
        }
        // A crash after recording an exception must not strand its ownership,
        // including when the source matched before case creation was acknowledged.
        let unowned=sqlx::query("SELECT id,source_batch_id FROM finance.export_breaks WHERE case_id IS NULL ORDER BY first_seen_at,id LIMIT 50").fetch_all(&self.pool).await?;
        for row in unowned {
            self.ensure_break_case(row.get("id"), row.get("source_batch_id"))
                .await?;
        }
        self.recover_report_cases().await?;
        self.recover_reconciliation_cases().await?;
        self.recover_credit_reconciliation_cases().await?;
        self.recover_group_reconciliation_cases().await?;
        self.recover_report_revisions().await?;
        self.recover_periodic_policies().await?;
        self.recover_basis_audit().await?;
        self.recover_corrections().await?;
        self.relay_audit().await
    }
}
