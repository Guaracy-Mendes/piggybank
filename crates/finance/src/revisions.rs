//! Approved links between immutable observation artifacts. No money command exists here.
use crate::{Finance, reports::ReportDetail};
use anyhow::{Result, ensure};
use bank_controls::governance::ChangeKind;
use bank_core::finance_snapshot::ReportRange;
use bank_platform::domain::{ActorContext, Currency, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
#[derive(Debug)]
pub struct RevisionConflict;
impl std::fmt::Display for RevisionConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Report revision scope or state conflict")
    }
}
impl std::error::Error for RevisionConflict {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevisionInput {
    pub original_report_id: Uuid,
    pub replacement_report_id: Uuid,
    pub reason: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevisionDetails {
    pub original_report_id: Uuid,
    pub replacement_report_id: Uuid,
    pub original_digest: String,
    pub replacement_digest: String,
    pub range: ReportRange,
    pub currency: Currency,
    pub reason: String,
    pub debit_delta: String,
    pub credit_delta: String,
    pub matched_debit_delta: String,
    pub source_count_delta: i32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionRecord {
    pub id: Uuid,
    pub details: RevisionDetails,
    pub details_digest: String,
    pub maker_person: Uuid,
    pub checker_person: Option<Uuid>,
    pub state: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub registered_at: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    pub registration_reason: String,
}
struct Draft {
    id: Uuid,
    details: RevisionDetails,
    digest: String,
    maker: Uuid,
    actor: ActorContext,
    expires: DateTime<Utc>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedRevision {
    pub proposal_id: Uuid,
    pub original_report_id: Uuid,
    pub replacement_report_id: Uuid,
    pub details_digest: String,
    pub maker_person: Uuid,
    pub checker_person: Uuid,
    pub published_at: DateTime<Utc>,
    pub reason: String,
}
fn draft(row: &sqlx::postgres::PgRow) -> Result<Draft> {
    let details: RevisionDetails = serde_json::from_value(row.get("details"))?;
    let digest: String = row.get("details_digest");
    ensure!(
        fingerprint(&details)? == digest
            && details.original_report_id == row.get::<Uuid, _>("original_report_id")
            && details.replacement_report_id == row.get::<Uuid, _>("replacement_report_id"),
        "Report revision artifact integrity mismatch"
    );
    Ok(Draft {
        id: row.get("id"),
        details,
        digest,
        maker: row.get("maker_person"),
        actor: serde_json::from_value(row.get("actor_context"))?,
        expires: row.get("expires_at"),
    })
}
fn compatible(original: &ReportDetail, replacement: &ReportDetail) -> Result<()> {
    let (a, b) = (&original.report, &replacement.report);
    ensure!(
        a.id != b.id
            && a.range == b.range
            && a.currency == b.currency
            && a.format_version == b.format_version
            && a.created_at < b.created_at
            && a.core_as_of <= b.core_as_of
            && a.finance_as_of <= b.finance_as_of,
        RevisionConflict
    );
    Ok(())
}
impl Finance {
    pub(crate) async fn published_revision_links(
        &self,
        report: Uuid,
    ) -> Result<Vec<PublishedRevision>> {
        Ok(sqlx::query("SELECT p.*,q.details->>'reason' AS reason FROM finance.report_revisions p JOIN finance.report_revision_requests q ON q.id=p.proposal_id WHERE p.original_report_id=$1 OR p.replacement_report_id=$1 ORDER BY p.published_at,p.proposal_id").bind(report).fetch_all(&self.pool).await?.into_iter().map(|r|PublishedRevision{proposal_id:r.get("proposal_id"),original_report_id:r.get("original_report_id"),replacement_report_id:r.get("replacement_report_id"),details_digest:r.get("details_digest"),maker_person:r.get("maker_person"),checker_person:r.get("checker_person"),published_at:r.get("published_at"),reason:r.get("reason")}).collect())
    }
    pub async fn propose_report_revision(
        &self,
        actor: &ActorContext,
        key: Uuid,
        input: &RevisionInput,
    ) -> Result<RevisionRecord> {
        self.authorize_staff(actor, true).await?;
        ensure!(
            !key.is_nil()
                && !input.original_report_id.is_nil()
                && !input.replacement_report_id.is_nil()
                && (4..=240).contains(&input.reason.trim().chars().count())
                && !input.reason.chars().any(char::is_control),
            RevisionConflict
        );
        // Fast replay still uses current authority; stored artifact digests and reason remain sealed.
        if let Some(row)=sqlx::query("SELECT * FROM finance.report_revision_requests WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&self.pool).await? {
            let d=draft(&row)?;ensure!(d.details.original_report_id==input.original_report_id&&d.details.replacement_report_id==input.replacement_report_id&&d.details.reason==input.reason,RevisionConflict);return self.report_revision(actor,d.id).await;
        }
        let a = self.report_detail(actor, input.original_report_id).await?;
        let b = self
            .report_detail(actor, input.replacement_report_id)
            .await?;
        compatible(&a, &b)?;
        let missing:bool=sqlx::query_scalar("SELECT EXISTS(SELECT source_batch_id FROM finance.report_items WHERE report_id=$1 EXCEPT SELECT source_batch_id FROM finance.report_items WHERE report_id=$2)").bind(a.report.id).bind(b.report.id).fetch_one(&self.pool).await?;
        ensure!(!missing, RevisionConflict);
        let details = RevisionDetails {
            original_report_id: a.report.id,
            replacement_report_id: b.report.id,
            original_digest: a.digest,
            replacement_digest: b.digest,
            range: a.report.range,
            currency: a.report.currency,
            reason: input.reason.clone(),
            debit_delta: (b.report.core_debit_total.parse::<i128>()?
                - a.report.core_debit_total.parse::<i128>()?)
            .to_string(),
            credit_delta: (b.report.core_credit_total.parse::<i128>()?
                - a.report.core_credit_total.parse::<i128>()?)
            .to_string(),
            matched_debit_delta: (b.report.confirmed_gl_debit_total.parse::<i128>()?
                - a.report.confirmed_gl_debit_total.parse::<i128>()?)
            .to_string(),
            source_count_delta: b.report.source_count - a.report.source_count,
        };
        let person = self.controls.person(actor).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,170))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        if let Some(row)=sqlx::query("SELECT * FROM finance.report_revision_requests WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await? {
            let d=draft(&row)?;ensure!(d.details.reason==input.reason&&d.details.original_report_id==input.original_report_id&&d.details.replacement_report_id==input.replacement_report_id,RevisionConflict);tx.commit().await?;return self.report_revision(actor,d.id).await;
        }
        let taken:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM finance.report_revisions WHERE original_report_id=$1 OR replacement_report_id=$2)").bind(a.report.id).bind(b.report.id).fetch_one(&mut *tx).await?;
        ensure!(!taken, RevisionConflict);
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO finance.report_revision_requests(id,original_report_id,replacement_report_id,issuer,subject,client_id,request_key,actor_context,maker_person,details,details_digest,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,now()+interval '24 hours')").bind(id).bind(a.report.id).bind(b.report.id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(serde_json::to_value(actor)?).bind(person.0).bind(serde_json::to_value(&details)?).bind(fingerprint(&details)?).execute(&mut *tx).await?;
        Self::audit_in(&mut tx, actor, "finance.report.revision-requested", id).await?;
        tx.commit().await?;
        self.report_revision(actor, id).await
    }
    pub async fn register_report_revision(&self, id: Uuid) -> Result<()> {
        let row = sqlx::query("SELECT * FROM finance.report_revision_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let d = draft(&row)?;
        self.authorize_staff(&d.actor, true).await?;
        ensure!(
            self.controls.person(&d.actor).await?.0 == d.maker && d.expires > Utc::now(),
            RevisionConflict
        );
        self.controls
            .propose_change_id(
                &d.actor,
                id,
                ChangeKind::FinanceReport,
                d.details.original_report_id,
                &d.details,
                d.expires,
            )
            .await?;
        sqlx::query("UPDATE finance.report_revision_requests SET registered_at=coalesce(registered_at,now()),registration_reason='REGISTERED' WHERE id=$1").bind(id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn report_revision(&self, actor: &ActorContext, id: Uuid) -> Result<RevisionRecord> {
        self.authorize_staff(actor, false).await?;
        let row=sqlx::query("SELECT q.*,p.published_at,p.checker_person FROM finance.report_revision_requests q LEFT JOIN finance.report_revisions p ON p.proposal_id=q.id WHERE q.id=$1").bind(id).fetch_one(&self.pool).await?;
        let d = draft(&row)?;
        let record = self
            .controls
            .change_record(id, ChangeKind::FinanceReport)
            .await?;
        if let Some(r) = &record {
            ensure!(
                r.digest == d.digest
                    && r.resource_id == d.details.original_report_id
                    && r.maker_person == d.maker
                    && r.expires_at == d.expires,
                RevisionConflict
            );
        }
        let published: Option<DateTime<Utc>> = row.get("published_at");
        let state = if published.is_some() {
            "PUBLISHED".into()
        } else if d.expires <= Utc::now() {
            "EXPIRED".into()
        } else {
            record.as_ref().map(|r| r.state.clone()).unwrap_or_else(|| {
                if row
                    .get::<Option<DateTime<Utc>>, _>("registered_at")
                    .is_some()
                {
                    "CONTROL_RECORD_UNAVAILABLE"
                } else {
                    "STAGED"
                }
                .into()
            })
        };
        Ok(RevisionRecord {
            id,
            details: d.details,
            details_digest: d.digest,
            maker_person: d.maker,
            checker_person: row
                .get::<Option<Uuid>, _>("checker_person")
                .or_else(|| record.and_then(|r| r.checker_person)),
            state,
            expires_at: d.expires,
            created_at: row.get("created_at"),
            registered_at: row.get("registered_at"),
            published_at: published,
            registration_reason: row.get("registration_reason"),
        })
    }
    pub async fn report_revisions(&self, actor: &ActorContext) -> Result<Vec<RevisionRecord>> {
        self.authorize_staff(actor, false).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM finance.report_revision_requests ORDER BY created_at DESC,id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut result = vec![];
        for id in ids {
            result.push(self.report_revision(actor, id).await?);
        }
        Ok(result)
    }
    pub async fn approve_report_revision(
        &self,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<RevisionRecord> {
        ensure!(
            self.controls.finance_approval_authorized(actor).await?,
            "Current Finance approval role and queue required"
        );
        let revision = self.report_revision(actor, id).await?;
        ensure!(revision.details_digest == digest, RevisionConflict);
        ensure!(
            ["STAGED", "PENDING", "APPROVED"].contains(&revision.state.as_str()),
            RevisionConflict
        );
        self.register_report_revision(id).await?;
        self.controls.approve_change(actor, id).await?;
        self.report_revision(actor, id).await
    }
    pub async fn publish_report_revision(
        &self,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<RevisionRecord> {
        self.authorize_staff(actor, true).await?;
        let revision = self.report_revision(actor, id).await?;
        ensure!(revision.details_digest == digest, RevisionConflict);
        ensure!(
            ["APPROVED", "PUBLISHED"].contains(&revision.state.as_str()),
            RevisionConflict
        );
        // Verify both full frozen artifacts before taking the publication lock.
        let a = self
            .report_detail(actor, revision.details.original_report_id)
            .await?;
        let b = self
            .report_detail(actor, revision.details.replacement_report_id)
            .await?;
        compatible(&a, &b)?;
        ensure!(
            a.digest == revision.details.original_digest
                && b.digest == revision.details.replacement_digest,
            RevisionConflict
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,171))")
            .bind(a.report.id.to_string())
            .execute(&mut *tx)
            .await?;
        let row =
            sqlx::query("SELECT * FROM finance.report_revision_requests WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        let d = draft(&row)?;
        self.authorize_staff(actor, true).await?;
        ensure!(
            self.controls.person(actor).await?.0 == d.maker && d.digest == digest,
            RevisionConflict
        );
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM finance.report_revisions WHERE proposal_id=$1)",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            let proof = self
                .controls
                .approved_change(
                    actor,
                    id,
                    ChangeKind::FinanceReport,
                    d.details.original_report_id,
                    &d.details,
                )
                .await?;
            let (maker, checker) = proof.people();
            ensure!(
                maker == d.maker && self.controls.finance_checker_current(checker).await?,
                "Current independent Finance checker required"
            );
            sqlx::query("INSERT INTO finance.report_revisions(proposal_id,original_report_id,replacement_report_id,details_digest,maker_person,checker_person) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(d.details.original_report_id).bind(d.details.replacement_report_id).bind(&d.digest).bind(maker).bind(checker).execute(&mut *tx).await?;
            Self::audit_in(&mut tx, actor, "finance.report.revision-published", id).await?;
        }
        tx.commit().await?;
        self.report_revision(actor, id).await
    }
    pub async fn recover_report_revisions(&self) -> Result<()> {
        let after: Uuid =
            sqlx::query_scalar("SELECT last_id FROM finance.revision_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.report_revision_requests WHERE id>$1 AND registered_at IS NULL AND expires_at>now() ORDER BY id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        sqlx::query("UPDATE finance.revision_cursor SET last_id=$1 WHERE id")
            .bind(ids.last().copied().unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        for id in ids {
            let result = self.register_report_revision(id).await;
            sqlx::query("UPDATE finance.report_revision_requests SET registration_attempts=registration_attempts+1,registration_reason=CASE WHEN registered_at IS NOT NULL THEN 'REGISTERED' ELSE $2 END WHERE id=$1").bind(id).bind(if result.is_ok(){"REGISTERED"}else{"CURRENT_AUTHORITY_OR_DEPENDENCY_UNAVAILABLE"}).execute(&self.pool).await?;
        }
        Ok(())
    }
}
