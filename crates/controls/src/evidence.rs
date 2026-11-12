use crate::audit::{AuditLog, AuditRecord};
use anyhow::{Result, ensure};
use bank_platform::domain::ActorContext;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

/// Filesystem/S3 adapters implement this synchronous port; application code
/// calls blocking adapters on a blocking executor, outside DB transactions.
pub trait BlobStore: Send + Sync + 'static {
    fn write_new(&self, id: Uuid, data: &[u8]) -> Result<()>;
    fn read_bounded(&self, id: Uuid, max: usize) -> Result<Vec<u8>>;
    fn ids(&self) -> Result<Vec<Uuid>>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub id: Uuid,
    pub digest: String,
    pub size: i64,
}
pub struct EvidenceService<T: BlobStore> {
    pub pool: PgPool,
    pub store: Arc<T>,
}
impl<T: BlobStore> EvidenceService<T> {
    pub fn new(pool: PgPool, store: Arc<T>) -> Self {
        Self { pool, store }
    }
    pub async fn put(
        &self,
        actor: &ActorContext,
        purpose: &str,
        data: Vec<u8>,
        correlation: Uuid,
    ) -> Result<EvidenceRef> {
        ensure!(data.len() <= 8_388_608, "Evidence size limit exceeded");
        ensure!(
            ["IDENTITY", "CASE", "SYNTHETIC"].contains(&purpose),
            "Unknown evidence purpose"
        );
        let reference = EvidenceRef {
            id: Uuid::new_v4(),
            digest: format!("{:x}", Sha256::digest(&data)),
            size: data.len() as i64,
        };
        sqlx::query("INSERT INTO controls.evidence_objects(id,owner_issuer,owner_subject,purpose,digest,size,state) VALUES($1,$2,$3,$4,$5,$6,'STAGED')")
            .bind(reference.id).bind(&actor.issuer).bind(&actor.subject).bind(purpose).bind(&reference.digest).bind(reference.size).execute(&self.pool).await?;
        let store = self.store.clone();
        let id = reference.id;
        tokio::task::spawn_blocking(move || store.write_new(id, &data)).await??;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE controls.evidence_objects SET state='READY' WHERE id=$1 AND state='STAGED'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        AuditLog::append_in(
            &mut tx,
            id,
            "evidence",
            &AuditRecord::new(actor, "evidence.created", id, correlation),
        )
        .await?;
        tx.commit().await?;
        Ok(reference)
    }
    pub async fn get(&self, actor: &ActorContext, id: Uuid, correlation: Uuid) -> Result<Vec<u8>> {
        let row=sqlx::query("SELECT digest,size FROM controls.evidence_objects WHERE id=$1 AND owner_issuer=$2 AND owner_subject=$3 AND state='READY'")
            .bind(id).bind(&actor.issuer).bind(&actor.subject).fetch_optional(&self.pool).await?;
        let Some(row) = row else {
            let mut record = AuditRecord::new(actor, "evidence.denied", id, correlation);
            record.outcome = "DENIED".into();
            record.reason_code = "OBJECT_UNAVAILABLE".into();
            AuditLog::new(self.pool.clone())
                .append(Uuid::new_v4(), "evidence", &record)
                .await?;
            return Err(anyhow::anyhow!("Evidence unavailable"));
        };
        let digest: String = row.get("digest");
        let size: i64 = row.get("size");
        let store = self.store.clone();
        let data =
            tokio::task::spawn_blocking(move || store.read_bounded(id, size as usize)).await??;
        ensure!(
            data.len() as i64 == size && format!("{:x}", Sha256::digest(&data)) == digest,
            "Evidence integrity check failed"
        );
        AuditLog::new(self.pool.clone())
            .append(
                Uuid::new_v4(),
                "evidence",
                &AuditRecord::new(actor, "evidence.retrieved", id, correlation),
            )
            .await?;
        Ok(data)
    }
    pub async fn get_for_case(
        &self,
        actor: &ActorContext,
        case: Uuid,
        id: Uuid,
        cases: &crate::cases::Cases,
    ) -> Result<Vec<u8>> {
        let record = cases.detail(actor, case).await?;
        ensure!(
            record.evidence_refs.contains(&id),
            "Evidence is not part of this authorized case"
        );
        let row = sqlx::query(
            "SELECT digest,size FROM controls.evidence_objects WHERE id=$1 AND state='READY'",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let size: i64 = row.get("size");
        let digest: String = row.get("digest");
        let store = self.store.clone();
        let data =
            tokio::task::spawn_blocking(move || store.read_bounded(id, size as usize)).await??;
        ensure!(
            data.len() as i64 == size && format!("{:x}", Sha256::digest(&data)) == digest,
            "Evidence integrity check failed"
        );
        AuditLog::new(self.pool.clone())
            .append(
                Uuid::new_v4(),
                "evidence",
                &AuditRecord::new(actor, "evidence.case-retrieved", id, case),
            )
            .await?;
        Ok(data)
    }
    pub async fn recover_staged(&self) -> Result<usize> {
        let rows=sqlx::query("SELECT id,owner_issuer,owner_subject,digest,size FROM controls.evidence_objects WHERE state='STAGED' AND created_at<now()-interval '1 minute'").fetch_all(&self.pool).await?;
        let mut recovered = 0;
        for row in rows {
            let id: Uuid = row.get("id");
            let size: i64 = row.get("size");
            let digest: String = row.get("digest");
            let store = self.store.clone();
            let Ok(Ok(bytes)) =
                tokio::task::spawn_blocking(move || store.read_bounded(id, size as usize)).await
            else {
                continue;
            };
            if bytes.len() as i64 != size || format!("{:x}", Sha256::digest(&bytes)) != digest {
                sqlx::query("UPDATE controls.evidence_objects SET state='QUARANTINED' WHERE id=$1 AND state='STAGED'").bind(id).execute(&self.pool).await?;
                continue;
            }
            let mut tx = self.pool.begin().await?;
            let updated = sqlx::query(
                "UPDATE controls.evidence_objects SET state='READY' WHERE id=$1 AND state='STAGED'",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() == 1 {
                let actor = ActorContext {
                    issuer: "urn:piggybank:synthetic:service".into(),
                    subject: "evidence-recovery".into(),
                    trust: bank_platform::domain::TrustContext::Service,
                    client_id: "evidence-recovery".into(),
                    person_id: None,
                    represented_party: None,
                    roles: vec![],
                };
                AuditLog::append_in(
                    &mut tx,
                    id,
                    "evidence",
                    &AuditRecord::new(&actor, "evidence.recovered", id, id),
                )
                .await?;
                recovered += 1;
            }
            tx.commit().await?;
        }
        Ok(recovered)
    }
    pub async fn unreferenced(&self) -> Result<Vec<Uuid>> {
        let store = self.store.clone();
        let files = tokio::task::spawn_blocking(move || store.ids()).await??;
        let mut unreferenced = Vec::new();
        for id in files {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM controls.evidence_objects WHERE id=$1)",
            )
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
            if !exists {
                unreferenced.push(id)
            }
        }
        Ok(unreferenced) // Inventory only: no deletion before approved retention/recovery policy.
    }
}
