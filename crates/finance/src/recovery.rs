use crate::{Finance, service_actor};
use anyhow::{Result, ensure};
use bank_platform::{
    domain::{ActorContext, fingerprint},
    operations::{JobSpec, OperationStatus},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryCommand {
    pub id: Uuid,
    pub export_id: Uuid,
    pub generation: i64,
    pub reason: String,
    pub operation_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
fn command(row: sqlx::postgres::PgRow) -> RetryCommand {
    RetryCommand {
        id: row.get("id"),
        export_id: row.get("export_id"),
        generation: row.get("generation"),
        reason: row.get("reason"),
        operation_id: row.get("operation_id"),
        created_at: row.get("created_at"),
    }
}
impl Finance {
    pub async fn staff_breaks(&self, actor: &ActorContext) -> Result<serde_json::Value> {
        self.authorize_staff(actor, false).await?;
        let rows=sqlx::query("SELECT * FROM finance.export_breaks ORDER BY (state='OPEN') DESC,last_seen_at DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        Ok(json!(rows.into_iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"sourceBatchId":r.get::<Uuid,_>("source_batch_id"),"code":r.get::<String,_>("code"),"state":r.get::<String,_>("state"),"caseId":r.get::<Option<Uuid>,_>("case_id"),"firstSeenAt":r.get::<DateTime<Utc>,_>("first_seen_at"),"lastSeenAt":r.get::<DateTime<Utc>,_>("last_seen_at"),"matchedAt":r.get::<Option<DateTime<Utc>>,_>("matched_at")})).collect::<Vec<_>>()))
    }
    pub async fn authorize_staff(&self, actor: &ActorContext, write: bool) -> Result<()> {
        ensure!(
            self.controls.finance_authorized(actor, write).await?,
            "Current Finance role and queue membership required"
        );
        Ok(())
    }
    pub async fn retry_export(
        &self,
        actor: &ActorContext,
        id: Uuid,
        key: Uuid,
        reason: &str,
    ) -> Result<RetryCommand> {
        self.authorize_staff(actor, true).await?;
        ensure!(
            !key.is_nil()
                && (4..=240).contains(&reason.chars().count())
                && !reason.chars().any(char::is_control),
            "Valid Finance recovery reason and request identity required"
        );
        let digest = fingerprint(&json!({"exportId":id,"reason":reason}))?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,167))")
            .bind(format!(
                "{}/{}/{}/{}",
                actor.issuer, actor.subject, actor.client_id, key
            ))
            .execute(&mut *tx)
            .await?;
        if let Some(row)=sqlx::query("SELECT * FROM finance.retry_commands WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND request_key=$4").bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).fetch_optional(&mut *tx).await? {
            ensure!(row.get::<String,_>("fingerprint")==digest,"Finance recovery identity conflict");let r=command(row);tx.commit().await?;return Ok(r);
        }
        let row = sqlx::query("SELECT * FROM finance.gl_exports WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        self.authorize_staff(actor, true).await?;
        ensure!(
            row.get::<String, _>("state") == "REVIEW_REQUIRED"
                && row
                    .get::<Option<DateTime<Utc>>, _>("lease_until")
                    .is_none_or(|t| t <= Utc::now()),
            "GL recovery requires a review state without a live transport lease"
        );
        let generation = row
            .get::<i64, _>("retry_generation")
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Recovery generation exhausted"))?;
        let ceiling = row
            .get::<i32, _>("attempts")
            .checked_add(8)
            .ok_or_else(|| anyhow::anyhow!("Recovery budget exhausted"))?;
        let request = Uuid::new_v4();
        let row=sqlx::query("INSERT INTO finance.retry_commands(id,export_id,generation,issuer,subject,client_id,request_key,reason,fingerprint) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING *")
            .bind(request).bind(id).bind(generation).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(key).bind(reason).bind(digest).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE finance.gl_exports SET state='OUTCOME_UNKNOWN',reason_code='GL_OPERATOR_INQUIRY_REQUESTED',retry_generation=$2,attempt_limit=$3,updated_at=now() WHERE id=$1").bind(id).bind(generation).bind(ceiling).execute(&mut *tx).await?;
        Self::audit_in(&mut tx, actor, "finance.export.recovery-requested", request).await?;
        tx.commit().await?;
        Ok(command(row))
    }
    pub(crate) async fn schedule_retry(&self, id: Uuid) -> Result<OperationStatus> {
        let row = sqlx::query("SELECT * FROM finance.retry_commands WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let request = command(row);
        let op=self.operations.accept(&service_actor(),"finance.retry",&id.to_string(),&json!({"requestId":id,"exportId":request.export_id,"generation":request.generation,"reason":request.reason}),JobSpec{kind:"finance.export".into(),step_key:"recover".into(),payload:json!({"exportId":request.export_id,"generation":request.generation}),delay_seconds:0}).await?;
        sqlx::query("UPDATE finance.retry_commands SET operation_id=$2 WHERE id=$1 AND operation_id IS NULL").bind(id).bind(op.id).execute(&self.pool).await?;
        Ok(op)
    }
    pub async fn staff_exports(&self, actor: &ActorContext) -> Result<Vec<crate::ExportStatus>> {
        self.authorize_staff(actor, false).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM finance.gl_exports ORDER BY created_at DESC,id LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut items = vec![];
        for id in ids {
            items.push(self.export_status(id).await?);
        }
        Ok(items)
    }
    pub async fn staff_detail(&self, actor: &ActorContext, id: Uuid) -> Result<serde_json::Value> {
        self.authorize_staff(actor, false).await?;
        let status = self.export_status(id).await?;
        let history=sqlx::query("SELECT state,reason_code,attempts,lease_fence,recorded_at FROM finance.gl_history WHERE export_id=$1 ORDER BY ordinal DESC LIMIT 100").bind(id).fetch_all(&self.pool).await?.into_iter().map(|r|json!({"state":r.get::<String,_>("state"),"reasonCode":r.get::<String,_>("reason_code"),"attempts":r.get::<i32,_>("attempts"),"leaseFence":r.get::<i64,_>("lease_fence"),"recordedAt":r.get::<DateTime<Utc>,_>("recorded_at")})).collect::<Vec<_>>();
        let requests=sqlx::query("SELECT * FROM finance.retry_commands WHERE export_id=$1 ORDER BY generation DESC LIMIT 100").bind(id).fetch_all(&self.pool).await?.into_iter().map(command).collect::<Vec<_>>();
        let breaks=sqlx::query("SELECT id,code,state,case_id FROM finance.export_breaks WHERE source_batch_id=$1 ORDER BY first_seen_at,id").bind(status.source_batch_id).fetch_all(&self.pool).await?.into_iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"code":r.get::<String,_>("code"),"state":r.get::<String,_>("state"),"caseId":r.get::<Option<Uuid>,_>("case_id")})).collect::<Vec<_>>();
        Ok(json!({"export":status,"history":history,"recoveryRequests":requests,"breaks":breaks}))
    }
}
