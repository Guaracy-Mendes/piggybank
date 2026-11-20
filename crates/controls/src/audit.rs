use anyhow::{Result, ensure};
use bank_platform::domain::ActorContext;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditRecord {
    pub actor_issuer: String,
    pub actor_subject: String,
    pub actor_person_id: Option<bank_platform::domain::PartyId>,
    pub represented_party: Option<bank_platform::domain::PartyId>,
    pub action: String,
    pub resource: String,
    pub outcome: String,
    pub policy_version: String,
    pub reason_code: String,
    pub correlation_id: Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}
impl AuditRecord {
    pub fn new(actor: &ActorContext, action: &str, resource: Uuid, correlation_id: Uuid) -> Self {
        Self {
            actor_issuer: actor.issuer.clone(),
            actor_subject: actor.subject.clone(),
            actor_person_id: actor.person_id,
            represented_party: actor.represented_party,
            action: action.into(),
            resource: resource.to_string(),
            outcome: "RECORDED".into(),
            policy_version: "synthetic-v1".into(),
            reason_code: "DOMAIN_EVENT".into(),
            correlation_id,
            occurred_at: chrono::Utc::now(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        for code in [
            &self.action,
            &self.outcome,
            &self.policy_version,
            &self.reason_code,
        ] {
            ensure!(
                !code.is_empty()
                    && code.len() <= 128
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                "Audit codes must be bounded non-PII identifiers"
            );
        }
        ensure!(
            !self.actor_issuer.is_empty() && !self.actor_subject.is_empty(),
            "Audit actor required"
        );
        Uuid::parse_str(&self.resource)?;
        Ok(())
    }
}

/// High-volume audit streams are partitioned into a fixed set of hash chains (WP-30 capacity finding CF-01).
/// `controls.append_audit` locks a stream's head row until commit, so a single stream serializes every writer.
/// Measured locally, the single `authorization` stream capped all authenticated traffic at about 460 requests per
/// second. The shard set is fixed, so a missing or truncated chain is still detectable. Records written before the
/// split stay in the legacy unsuffixed stream, which remains verifiable.
pub const STREAM_SHARDS: u8 = 16;
pub fn shard_stream(base: &str, key: Uuid) -> String {
    format!("{base}-{:02}", key.as_bytes()[15] % STREAM_SHARDS)
}
/// The legacy unsuffixed stream followed by every shard of `base`.
pub fn shard_streams(base: &str) -> Vec<String> {
    std::iter::once(base.to_owned())
        .chain((0..STREAM_SHARDS).map(|n| format!("{base}-{n:02}")))
        .collect()
}
/// Control-check audit: one resource's checks stay in one ordered chain.
pub fn control_stream(resource: Uuid) -> String {
    shard_stream("controls", resource)
}

#[derive(Clone)]
pub struct AuditLog {
    pub pool: PgPool,
}
impl AuditLog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn append(&self, id: Uuid, stream: &str, record: &AuditRecord) -> Result<String> {
        let mut tx = self.pool.begin().await?;
        let digest = Self::append_in(&mut tx, id, stream, record).await?;
        tx.commit().await?;
        Ok(digest)
    }
    pub async fn append_in(
        tx: &mut Transaction<'_, Postgres>,
        id: Uuid,
        stream: &str,
        record: &AuditRecord,
    ) -> Result<String> {
        record.validate()?;
        ensure!(
            !stream.is_empty()
                && stream.len() <= 128
                && stream
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "Invalid audit stream"
        );
        Ok(sqlx::query_scalar("SELECT controls.append_audit($1,$2,$3)")
            .bind(id)
            .bind(stream)
            .bind(serde_json::to_value(record)?)
            .fetch_one(&mut **tx)
            .await?)
    }
    /// Verifies the legacy chain and every shard of a partitioned stream (`controls`, `authorization`).
    pub async fn verify_sharded(&self, base: &str) -> Result<bool> {
        for stream in shard_streams(base) {
            if !self.verify(&stream).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub async fn verify(&self, stream: &str) -> Result<bool> {
        let bad:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT sequence,previous_digest,digest,payload,lag(digest,1,repeat('0',64)) OVER(ORDER BY sequence) AS expected_previous,row_number() OVER(ORDER BY sequence) AS expected_sequence FROM controls.audit_events WHERE stream=$1) history WHERE previous_digest<>expected_previous OR sequence<>expected_sequence OR digest<>encode(sha256(convert_to(previous_digest||E'\\n'||sequence::text||E'\\n'||payload::text,'UTF8')),'hex')")
            .bind(stream).fetch_one(&self.pool).await?;
        let head_matches:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM controls.audit_heads h WHERE h.stream=$1 AND (h.sequence<>COALESCE((SELECT max(sequence) FROM controls.audit_events e WHERE e.stream=h.stream),0) OR h.digest<>COALESCE((SELECT digest FROM controls.audit_events e WHERE e.stream=h.stream ORDER BY sequence DESC LIMIT 1),repeat('0',64))))")
            .bind(stream).fetch_one(&self.pool).await?;
        Ok(bad == 0 && head_matches)
    }
}
