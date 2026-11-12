use crate::domain::{ActorContext, fingerprint};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Clone)]
pub struct Operations {
    pub pool: PgPool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStatus {
    pub id: Uuid,
    pub kind: String,
    pub state: String,
    pub version: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
#[derive(Clone, Serialize)]
pub struct JobSpec {
    pub kind: String,
    pub step_key: String,
    pub payload: serde_json::Value,
    pub delay_seconds: i64,
}
#[derive(Clone)]
pub struct Lease {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
    pub owner: Uuid,
    pub fence: i64,
    pub attempts: i32,
}
#[derive(Serialize)]
pub enum Completion {
    Done(serde_json::Value),
    Continue(JobSpec),
    Review(String),
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
impl Operations {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// The domain must perform current authorization before accepting or replaying
    /// a command. This method never grants account or financial authority.
    pub async fn accept<T: Serialize>(
        &self,
        actor: &ActorContext,
        kind: &str,
        key: &str,
        command: &T,
        job: JobSpec,
    ) -> Result<OperationStatus> {
        ensure!(
            identifier(kind) && identifier(key),
            "Invalid operation kind or idempotency key"
        );
        let digest = fingerprint(command)?;
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        let created=sqlx::query("INSERT INTO platform.operations(id,issuer,subject,client_id,kind,idempotency_key,fingerprint,state) VALUES($1,$2,$3,$4,$5,$6,$7,'ACCEPTED') ON CONFLICT(issuer,subject,client_id,kind,idempotency_key) DO NOTHING RETURNING id")
            .bind(id).bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(kind).bind(key).bind(&digest).fetch_optional(&mut *tx).await?;
        let row=sqlx::query("SELECT * FROM platform.operations WHERE issuer=$1 AND subject=$2 AND client_id=$3 AND kind=$4 AND idempotency_key=$5")
            .bind(&actor.issuer).bind(&actor.subject).bind(&actor.client_id).bind(kind).bind(key).fetch_one(&mut *tx).await?;
        ensure!(
            row.get::<String, _>("fingerprint") == digest,
            "Idempotency key reused with different request"
        );
        let status = from_row(&row);
        if created.is_some() {
            Self::enqueue(&mut tx, status.id, job).await?;
            crate::events::operation_fact(&mut tx, &self.pool, status.id).await?;
        }
        tx.commit().await?;
        Ok(status)
    }
    /// Internal workflow recovery after the owning domain revalidates current gates.
    /// A stable new step identity retains the original accepted operation.
    pub async fn resume_review(
        &self,
        actor: &ActorContext,
        id: Uuid,
        job: JobSpec,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let state:Option<String>=sqlx::query_scalar("SELECT state FROM platform.operations WHERE id=$1 AND issuer=$2 AND subject=$3 FOR UPDATE").bind(id).bind(&actor.issuer).bind(&actor.subject).fetch_optional(&mut *tx).await?;
        ensure!(state.is_some(), "Operation unavailable");
        if state.as_deref() != Some("REVIEW_REQUIRED") {
            tx.commit().await?;
            return Ok(false);
        }
        let used: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM platform.jobs WHERE operation_id=$1 AND step_key=$2)",
        )
        .bind(id)
        .bind(&job.step_key)
        .fetch_one(&mut *tx)
        .await?;
        if used {
            tx.commit().await?;
            return Ok(false);
        }
        Self::enqueue(&mut tx, id, job).await?;
        sqlx::query("UPDATE platform.operations SET state='ACCEPTED',version=version+1,updated_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await?;
        crate::events::operation_fact(&mut tx, &self.pool, id).await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn status(&self, actor: &ActorContext, id: Uuid) -> Result<OperationStatus> {
        // A caller sees only their durable status, never an unverified cached
        // domain response. Domain resources enforce their own current authority.
        let row = sqlx::query(
            "SELECT * FROM platform.operations WHERE id=$1 AND issuer=$2 AND subject=$3",
        )
        .bind(id)
        .bind(&actor.issuer)
        .bind(&actor.subject)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Operation unavailable"))?;
        Ok(from_row(&row))
    }
    async fn enqueue(
        tx: &mut Transaction<'_, Postgres>,
        operation: Uuid,
        job: JobSpec,
    ) -> Result<()> {
        ensure!(
            identifier(&job.kind)
                && identifier(&job.step_key)
                && (0..=2_592_000).contains(&job.delay_seconds)
                && job.payload.is_object(),
            "Invalid durable job"
        );
        sqlx::query("INSERT INTO platform.jobs(id,operation_id,kind,step_key,payload,state,available_at) VALUES($1,$2,$3,$4,$5,'READY',now()+make_interval(secs=>$6::double precision))")
            .bind(Uuid::new_v4()).bind(operation).bind(job.kind).bind(job.step_key).bind(job.payload).bind(job.delay_seconds as f64).execute(&mut **tx).await?;
        Ok(())
    }
    pub async fn claim(&self, owner: Uuid, lease_seconds: i64) -> Result<Option<Lease>> {
        ensure!(
            (1..=300).contains(&lease_seconds),
            "Invalid job lease interval"
        );
        let mut tx = self.pool.begin().await?;
        let exhausted=sqlx::query("UPDATE platform.jobs SET state='DEAD',last_error_code='RETRY_EXHAUSTED' WHERE attempts>=max_attempts AND (state='READY' OR (state='RUNNING' AND lease_until<now())) RETURNING operation_id").fetch_all(&mut *tx).await?;
        for row in exhausted {
            let id = row.get::<Uuid, _>("operation_id");
            let changed=sqlx::query("UPDATE platform.operations SET state='REVIEW_REQUIRED',version=version+1,updated_at=now() WHERE id=$1 AND state IN ('ACCEPTED','RUNNING')").bind(id).execute(&mut *tx).await?;
            if changed.rows_affected() == 1 {
                crate::events::operation_fact(&mut tx, &self.pool, id).await?;
            }
        }
        let row=sqlx::query("WITH candidate AS (SELECT id FROM platform.jobs WHERE attempts<max_attempts AND ((state='READY' AND available_at<=now()) OR (state='RUNNING' AND lease_until<now())) ORDER BY available_at,id LIMIT 1 FOR UPDATE SKIP LOCKED) UPDATE platform.jobs j SET state='RUNNING',lease_owner=$1,lease_until=now()+make_interval(secs=>$2::double precision),fence=fence+1,attempts=attempts+1 FROM candidate WHERE j.id=candidate.id RETURNING j.*")
            .bind(owner).bind(lease_seconds as f64).fetch_optional(&mut *tx).await?;
        let lease = row.map(|row| Lease {
            id: row.get("id"),
            operation_id: row.get("operation_id"),
            kind: row.get("kind"),
            payload: row.get("payload"),
            owner,
            fence: row.get("fence"),
            attempts: row.get("attempts"),
        });
        if let Some(lease) = &lease {
            sqlx::query("UPDATE platform.operations SET state='RUNNING',version=version+1,updated_at=now() WHERE id=$1 AND state IN ('ACCEPTED','RUNNING')").bind(lease.operation_id).execute(&mut *tx).await?;
            crate::events::operation_fact(&mut tx, &self.pool, lease.operation_id).await?;
        }
        tx.commit().await?;
        Ok(lease)
    }
    pub async fn complete(&self, lease: &Lease, completion: Completion) -> Result<()> {
        let completion_digest = fingerprint(&completion)?;
        let mut tx = self.pool.begin().await?;
        let updated=sqlx::query("UPDATE platform.jobs SET state='DONE',lease_until=NULL,completion_fingerprint=$4 WHERE id=$1 AND lease_owner=$2 AND fence=$3 AND state='RUNNING' AND lease_until>=now()")
            .bind(lease.id).bind(lease.owner).bind(lease.fence).bind(&completion_digest).execute(&mut *tx).await?;
        if updated.rows_affected() == 0 {
            let same:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform.jobs WHERE id=$1 AND lease_owner=$2 AND fence=$3 AND state='DONE' AND completion_fingerprint=$4)")
                .bind(lease.id).bind(lease.owner).bind(lease.fence).bind(&completion_digest).fetch_one(&mut *tx).await?;
            ensure!(same, "Job lease lost or completion identity conflict");
            tx.commit().await?;
            return Ok(());
        }
        ensure!(
            updated.rows_affected() == 1,
            "Job lease lost; stale worker is fenced"
        );
        match completion {
            Completion::Done(result) => {
                sqlx::query("UPDATE platform.operations SET state='SUCCEEDED',result=$2,version=version+1,updated_at=now() WHERE id=$1").bind(lease.operation_id).bind(result).execute(&mut *tx).await?;
                crate::events::operation_fact(&mut tx, &self.pool, lease.operation_id).await?;
            }
            Completion::Review(reason) => {
                ensure!(identifier(&reason), "Invalid review reason");
                sqlx::query("UPDATE platform.operations SET state='REVIEW_REQUIRED',result=$2,version=version+1,updated_at=now() WHERE id=$1").bind(lease.operation_id).bind(serde_json::json!({"reasonCode":reason})).execute(&mut *tx).await?;
                crate::events::operation_fact(&mut tx, &self.pool, lease.operation_id).await?;
            }
            Completion::Continue(job) => {
                Self::enqueue(&mut tx, lease.operation_id, job).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn retry(&self, lease: &Lease, code: &str, delay_seconds: i64) -> Result<()> {
        ensure!(
            identifier(code) && (0..=86_400).contains(&delay_seconds),
            "Invalid safe retry metadata"
        );
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("UPDATE platform.jobs SET state=CASE WHEN attempts>=max_attempts THEN 'DEAD' ELSE 'READY' END,available_at=now()+make_interval(secs=>$5::double precision),last_error_code=$4,lease_until=NULL WHERE id=$1 AND lease_owner=$2 AND fence=$3 AND state='RUNNING' AND lease_until>=now() RETURNING state")
            .bind(lease.id).bind(lease.owner).bind(lease.fence).bind(code).bind(delay_seconds as f64).fetch_optional(&mut *tx).await?.ok_or_else(||anyhow::anyhow!("Job lease lost"))?;
        if row.get::<String, _>("state") == "DEAD" {
            sqlx::query("UPDATE platform.operations SET state='REVIEW_REQUIRED',version=version+1,updated_at=now() WHERE id=$1").bind(lease.operation_id).execute(&mut *tx).await?;
            crate::events::operation_fact(&mut tx, &self.pool, lease.operation_id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
fn from_row(row: &sqlx::postgres::PgRow) -> OperationStatus {
    OperationStatus {
        id: row.get("id"),
        kind: row.get("kind"),
        state: row.get("state"),
        version: row.get("version"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
