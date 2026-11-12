//! Durable exact-source intents. Only explicit current-maker execution invokes the narrow Core command.
use crate::Finance;
use anyhow::{Result, ensure};
use bank_controls::governance::ChangeKind;
use bank_core::corrections::{
    CorrectionConflict, CorrectionDetails, CorrectionInputError, CorrectionReceipt,
    CorrectionSource,
};
use bank_platform::domain::{ActorContext, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CorrectionInput {
    pub original_batch_id: Uuid,
    pub reason: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionAttempt {
    pub id: Uuid,
    pub observed_reason: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionRequest {
    pub id: Uuid,
    pub source: CorrectionSource,
    pub details: CorrectionDetails,
    pub details_digest: String,
    pub maker_person: Uuid,
    pub checker_person: Option<Uuid>,
    pub state: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub registered_at: Option<DateTime<Utc>>,
    pub registration_reason: String,
    pub receipt: Option<CorrectionReceipt>,
    pub attempts: Vec<CorrectionAttempt>,
    pub original_export_id: Option<Uuid>,
    pub correction_export_id: Option<Uuid>,
}
struct Draft {
    id: Uuid,
    source: CorrectionSource,
    details: CorrectionDetails,
    digest: String,
    maker: Uuid,
    actor: ActorContext,
    expires: DateTime<Utc>,
}
fn draft(r: &sqlx::postgres::PgRow) -> Result<Draft> {
    let raw_source: serde_json::Value = r.get("source");
    let source: CorrectionSource = serde_json::from_value(raw_source.clone())?;
    ensure!(
        fingerprint(&raw_source)? == fingerprint(&source)?,
        CorrectionConflict
    );
    let details: CorrectionDetails = serde_json::from_value(r.get("details"))?;
    let id = r.get("id");
    let digest: String = r.get("details_digest");
    ensure!(
        details.execution_id == id
            && details.original_batch_id == r.get::<Uuid, _>("original_batch_id")
            && fingerprint(&source.details(id, &details.reason)?)? == digest
            && fingerprint(&details)? == digest,
        CorrectionConflict
    );
    Ok(Draft {
        id,
        source,
        details,
        digest,
        maker: r.get("maker_person"),
        actor: serde_json::from_value(r.get("actor_context"))?,
        expires: r.get("expires_at"),
    })
}
impl Finance {
    pub async fn propose_correction(
        &self,
        actor: &ActorContext,
        key: Uuid,
        input: &CorrectionInput,
    ) -> Result<CorrectionRequest> {
        self.authorize_staff(actor, true).await?;
        ensure!(
            !key.is_nil() && !input.original_batch_id.is_nil(),
            CorrectionInputError
        );
        let maker = self.controls.person(actor).await?.0;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,178))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        if let Some(r)=sqlx::query("SELECT * FROM finance.correction_requests WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await?{
   let d=draft(&r)?;ensure!(d.details.original_batch_id==input.original_batch_id && d.details.reason==input.reason && d.maker==maker,CorrectionConflict);tx.commit().await?;return self.correction_request(actor,d.id).await
  }
        let source = self.corrections.source(input.original_batch_id).await?;
        ensure!(
            !self.corrections.corrected(input.original_batch_id).await?,
            CorrectionConflict
        );
        let id = Uuid::new_v4();
        let details = source.details(id, &input.reason)?;
        sqlx::query("INSERT INTO finance.correction_requests(id,original_batch_id,issuer,subject,client_id,request_key,actor_context,maker_person,source,details,details_digest,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,clock_timestamp()+interval '24 hours')").bind(id).bind(input.original_batch_id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(serde_json::to_value(actor)?).bind(maker).bind(serde_json::to_value(&source)?).bind(serde_json::to_value(&details)?).bind(fingerprint(&details)?).execute(&mut *tx).await?;
        Self::audit_in(&mut tx, actor, "finance.correction.requested", id).await?;
        tx.commit().await?;
        self.correction_request(actor, id).await
    }
    pub async fn register_correction(&self, id: Uuid) -> Result<()> {
        let r = sqlx::query("SELECT * FROM finance.correction_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let d = draft(&r)?;
        self.authorize_staff(&d.actor, true).await?;
        ensure!(
            self.controls.person(&d.actor).await?.0 == d.maker && d.expires > Utc::now(),
            CorrectionConflict
        );
        self.controls
            .propose_change_id(
                &d.actor,
                id,
                ChangeKind::FinanceCorrection,
                d.details.original_batch_id,
                &d.details,
                d.expires,
            )
            .await?;
        sqlx::query("UPDATE finance.correction_requests SET registered_at=coalesce(registered_at,clock_timestamp()),registration_reason='REGISTERED' WHERE id=$1").bind(id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn correction_request(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<CorrectionRequest> {
        self.authorize_staff(actor, false).await?;
        let r = sqlx::query("SELECT * FROM finance.correction_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let d = draft(&r)?;
        let control = self
            .controls
            .change_record(id, ChangeKind::FinanceCorrection)
            .await?;
        if let Some(c) = &control {
            ensure!(
                c.resource_id == d.details.original_batch_id
                    && c.digest == d.digest
                    && c.maker_person == d.maker
                    && c.expires_at == d.expires,
                CorrectionConflict
            );
        }
        let receipt = self.corrections.receipt(id, &d.details).await?;
        if let (Some(c), Some(control)) = (&receipt, &control) {
            ensure!(
                control.checker_person == Some(c.checker_person) && control.state == "APPROVED",
                CorrectionConflict
            );
        }
        if let Some(c) = &receipt {
            ensure!(
                c.proposal_id == id && c.maker_person == d.maker && c.details_digest == d.digest,
                CorrectionConflict
            );
        }
        let state = if receipt.is_some() {
            "EXECUTED".into()
        } else if self
            .corrections
            .corrected(d.details.original_batch_id)
            .await?
        {
            "SOURCE_ALREADY_CORRECTED".into()
        } else if d.expires <= Utc::now() {
            "EXPIRED".into()
        } else {
            control
                .as_ref()
                .map(|c| c.state.clone())
                .unwrap_or_else(|| {
                    if r.get::<Option<DateTime<Utc>>, _>("registered_at").is_some() {
                        "CONTROL_RECORD_UNAVAILABLE".into()
                    } else {
                        "STAGED".into()
                    }
                })
        };
        let attempts=sqlx::query("SELECT id,observed_reason,created_at FROM finance.correction_attempts WHERE request_id=$1 ORDER BY created_at DESC,id LIMIT 50").bind(id).fetch_all(&self.pool).await?.into_iter().map(|a|CorrectionAttempt{id:a.get("id"),observed_reason:a.get("observed_reason"),created_at:a.get("created_at")}).collect();
        let original_export_id =
            sqlx::query_scalar("SELECT id FROM finance.gl_exports WHERE source_batch_id=$1")
                .bind(d.details.original_batch_id)
                .fetch_optional(&self.pool)
                .await?;
        let correction_export_id = if let Some(receipt) = &receipt {
            sqlx::query_scalar("SELECT id FROM finance.gl_exports WHERE source_batch_id=$1")
                .bind(receipt.batch_id)
                .fetch_optional(&self.pool)
                .await?
        } else {
            None
        };
        Ok(CorrectionRequest {
            id,
            source: d.source,
            details: d.details,
            details_digest: d.digest,
            maker_person: d.maker,
            checker_person: receipt
                .as_ref()
                .map(|c| c.checker_person)
                .or_else(|| control.and_then(|c| c.checker_person)),
            state,
            expires_at: d.expires,
            created_at: r.get("created_at"),
            registered_at: r.get("registered_at"),
            registration_reason: r.get("registration_reason"),
            receipt,
            attempts,
            original_export_id,
            correction_export_id,
        })
    }
    pub async fn correction_requests(
        &self,
        actor: &ActorContext,
    ) -> Result<Vec<CorrectionRequest>> {
        self.authorize_staff(actor, false).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM finance.correction_requests ORDER BY created_at DESC,id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.correction_request(actor, id).await?);
        }
        Ok(out)
    }
    pub async fn approve_correction(
        &self,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<CorrectionRequest> {
        ensure!(
            self.controls.finance_approval_authorized(actor).await?,
            "Current Finance checker authority required"
        );
        let r = self.correction_request(actor, id).await?;
        ensure!(
            r.details_digest == digest
                && ["STAGED", "PENDING", "APPROVED"].contains(&r.state.as_str()),
            CorrectionConflict
        );
        self.register_correction(id).await?;
        self.controls.approve_change(actor, id).await?;
        self.correction_request(actor, id).await
    }
    pub async fn execute_correction(
        &self,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<CorrectionRequest> {
        self.authorize_staff(actor, true).await?;
        let r = self.correction_request(actor, id).await?;
        ensure!(
            r.details_digest == digest
                && r.maker_person == self.controls.person(actor).await?.0
                && ["APPROVED", "EXECUTED"].contains(&r.state.as_str()),
            CorrectionConflict
        );
        // Persist the attempt before the Core boundary. Missing response never asserts financial failure.
        sqlx::query("INSERT INTO finance.correction_attempts(id,request_id,actor_context,observed_reason) VALUES($1,$2,$3,'EXECUTION_REQUESTED_RECEIPT_NOT_YET_OBSERVED')").bind(Uuid::new_v4()).bind(id).bind(serde_json::to_value(actor)?).execute(&self.pool).await?;
        let result = self
            .corrections
            .execute(&self.controls, actor, id, &r.details)
            .await;
        let observed = self.corrections.receipt(id, &r.details).await;
        let confirmed = matches!(&observed, Ok(Some(_)));
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO finance.correction_attempts(id,request_id,actor_context,observed_reason) VALUES($1,$2,$3,$4)").bind(Uuid::new_v4()).bind(id).bind(serde_json::to_value(actor)?).bind(if confirmed{"NATIVE_RECEIPT_OBSERVED"}else if observed.is_err(){"NATIVE_RECEIPT_INQUIRY_UNAVAILABLE"}else{"NATIVE_RECEIPT_NOT_OBSERVED"}).execute(&mut *tx).await?;
        Self::audit_in(
            &mut tx,
            actor,
            if confirmed {
                "finance.correction.receipt-observed"
            } else {
                "finance.correction.unconfirmed"
            },
            id,
        )
        .await?;
        tx.commit().await?;
        if !confirmed {
            observed?;
            result?;
            anyhow::bail!(CorrectionConflict)
        }
        self.correction_request(actor, id).await
    }
    pub async fn recover_corrections(&self) -> Result<()> {
        let after: Uuid =
            sqlx::query_scalar("SELECT last_id FROM finance.correction_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.correction_requests WHERE id>$1 AND registered_at IS NULL AND expires_at>clock_timestamp() ORDER BY id LIMIT 50").bind(after).fetch_all(&self.pool).await?;
        sqlx::query("UPDATE finance.correction_cursor SET last_id=$1 WHERE id")
            .bind(ids.last().copied().unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        for id in ids {
            let result = self.register_correction(id).await;
            sqlx::query("UPDATE finance.correction_requests SET registration_attempts=registration_attempts+1,registration_reason=CASE WHEN registered_at IS NOT NULL THEN 'REGISTERED' ELSE $2 END WHERE id=$1").bind(id).bind(if result.is_ok(){"REGISTERED"}else{"CURRENT_AUTHORITY_OR_DEPENDENCY_UNAVAILABLE"}).execute(&self.pool).await?;
        }
        Ok(())
    }
}
