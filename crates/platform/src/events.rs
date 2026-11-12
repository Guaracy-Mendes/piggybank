use crate::{
    database::Module,
    domain::{ActorContext, EventEnvelope, EventId, fingerprint},
};
use anyhow::{Result, ensure};
use futures_util::future::BoxFuture;
use serde_json::json;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Clone)]
pub struct EventStore {
    pub pool: PgPool,
    pub module: Module,
}
pub trait ReplayAuthorizer: Send + Sync {
    fn authorize_replay<'a>(&'a self, actor: &'a ActorContext) -> BoxFuture<'a, Result<bool>>;
}
impl EventStore {
    pub fn new(pool: PgPool, module: Module) -> Self {
        Self { pool, module }
    }
    // Only an enum-owned schema and static SQL are substituted; all data is bound.
    fn sql(&self, template: &'static str) -> sqlx::AssertSqlSafe<String> {
        sqlx::AssertSqlSafe(template.replace("{schema}", self.module.name()))
    }
    pub async fn stage(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        event: &EventEnvelope,
    ) -> Result<()> {
        event.validate()?;
        ensure!(
            event.event_type.len() <= 128
                && event
                    .event_type
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            "Invalid event subject"
        );
        ensure!(
            event.producer == self.module.name() && event.schema_version == 1,
            "Producer/schema mismatch"
        );
        let document = serde_json::to_value(event)?;
        ensure!(
            serde_json::to_vec(&document)?.len() <= 65536,
            "Event exceeds transport bound"
        );
        let digest = fingerprint(&document)?;
        sqlx::query(self.sql("INSERT INTO {schema}.outbox(event_id,aggregate_id,aggregate_version,document,fingerprint) VALUES($1,$2,$3,$4,$5) ON CONFLICT(event_id) DO NOTHING"))
            .bind(event.event_id.0).bind(event.aggregate_id).bind(event.aggregate_version).bind(document).bind(&digest).execute(&mut **tx).await?;
        let saved: String = sqlx::query_scalar(
            self.sql("SELECT fingerprint FROM {schema}.outbox WHERE event_id=$1"),
        )
        .bind(event.event_id.0)
        .fetch_one(&mut **tx)
        .await?;
        ensure!(
            saved == digest,
            "Event identity reused with different content"
        );
        Ok(())
    }
    pub async fn publish_one(&self, jetstream: &async_nats::jetstream::Context) -> Result<bool> {
        self.publish_one_to(jetstream, &subject_prefix()?).await
    }
    pub async fn publish_one_to(
        &self,
        jetstream: &async_nats::jetstream::Context,
        prefix: &str,
    ) -> Result<bool> {
        validate_transport(prefix)?;
        let lease = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query(self.sql("WITH candidate AS (SELECT o.event_id FROM {schema}.outbox o WHERE o.delivered_at IS NULL AND (o.lease_until IS NULL OR o.lease_until<now()) AND NOT EXISTS(SELECT 1 FROM {schema}.outbox prior WHERE prior.aggregate_id=o.aggregate_id AND prior.aggregate_version<o.aggregate_version AND prior.delivered_at IS NULL) ORDER BY o.created_at,o.event_id LIMIT 1 FOR UPDATE SKIP LOCKED) UPDATE {schema}.outbox o SET lease_id=$1,lease_until=now()+interval '10 seconds' FROM candidate WHERE o.event_id=candidate.event_id RETURNING o.document"))
            .bind(lease).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        let Some(row) = row else { return Ok(false) };
        let event: EventEnvelope = serde_json::from_value(row.get("document"))?;
        let mut headers = async_nats::HeaderMap::new();
        headers.insert("Nats-Msg-Id", event.event_id.to_string());
        // Publish and broker acknowledgement occur outside a database transaction.
        jetstream
            .publish_with_headers(
                format!("{prefix}.{}.{}", event.producer, event.event_type),
                headers,
                serde_json::to_vec(&event)?.into(),
            )
            .await?
            .await?;
        sqlx::query(self.sql("UPDATE {schema}.outbox SET delivered_at=now(),lease_until=NULL WHERE event_id=$1 AND lease_id=$2"))
            .bind(event.event_id.0).bind(lease).execute(&self.pool).await?;
        Ok(true)
    }
    pub async fn quarantine(
        &self,
        consumer: &str,
        event: &EventEnvelope,
        reason: &str,
    ) -> Result<()> {
        sqlx::query(self.sql("INSERT INTO {schema}.dead_letters(consumer,event_id,reason,document) VALUES($1,$2,$3,$4) ON CONFLICT(consumer,event_id) DO UPDATE SET reason=EXCLUDED.reason"))
            .bind(consumer).bind(event.event_id.0).bind(reason).bind(serde_json::to_value(event)?).execute(&self.pool).await?;
        Ok(())
    }
    /// Retain malformed deliveries with a digest identity; never silently ACK a poison message.
    pub async fn quarantine_raw(&self, consumer: &str, bytes: &[u8]) -> Result<()> {
        use sha2::{Digest, Sha256};
        ensure!(
            bytes.len() <= 1_048_576,
            "Raw event exceeds quarantine bound"
        );
        let digest = format!("{:x}", Sha256::digest(bytes));
        sqlx::query(self.sql("INSERT INTO {schema}.invalid_deliveries(consumer,digest,payload) VALUES($1,$2,$3) ON CONFLICT DO NOTHING"))
            .bind(consumer).bind(digest).bind(bytes).execute(&self.pool).await?;
        Ok(())
    }
    /// Initial registered handler: governed operational projection only. Other
    /// domain event types need explicit handlers; they are never silently applied.
    pub async fn consume_operation(&self, consumer: &str, event: &EventEnvelope) -> Result<bool> {
        ensure!(
            self.module.name() == "data",
            "Operation projection belongs to Data"
        );
        event.validate()?;
        if event.schema_version != 1
            || event.producer != "platform"
            || event.event_type != "operation.changed"
        {
            self.quarantine(consumer, event, "UNSUPPORTED_SCHEMA_OR_TYPE")
                .await?;
            return Ok(false);
        }
        let state = event
            .data
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let kind = event
            .data
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if ![
            "ACCEPTED",
            "RUNNING",
            "SUCCEEDED",
            "FAILED",
            "REVIEW_REQUIRED",
        ]
        .contains(&state)
            || kind.is_empty()
            || kind.len() > 128
        {
            self.quarantine(consumer, event, "INVALID_OPERATION_PAYLOAD")
                .await?;
            return Ok(false);
        }
        let digest = fingerprint(event)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(self.sql("INSERT INTO {schema}.consumer_versions(consumer,producer,aggregate_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING"))
            .bind(consumer).bind(&event.producer).bind(event.aggregate_id).execute(&mut *tx).await?;
        let version:i64=sqlx::query_scalar(self.sql("SELECT version FROM {schema}.consumer_versions WHERE consumer=$1 AND producer=$2 AND aggregate_id=$3 FOR UPDATE"))
            .bind(consumer).bind(&event.producer).bind(event.aggregate_id).fetch_one(&mut *tx).await?;
        let prior: Option<String> = sqlx::query_scalar(
            self.sql("SELECT fingerprint FROM {schema}.inbox WHERE consumer=$1 AND event_id=$2"),
        )
        .bind(consumer)
        .bind(event.event_id.0)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(prior) = prior {
            if prior != digest {
                tx.rollback().await?;
                self.quarantine(consumer, event, "CONFLICTING_EVENT_IDENTITY")
                    .await?;
                return Ok(false);
            }
            tx.commit().await?;
            return Ok(true);
        }
        if event.aggregate_version != version + 1 {
            tx.rollback().await?;
            self.quarantine(consumer, event, "AGGREGATE_VERSION_GAP")
                .await?;
            return Ok(false);
        }
        sqlx::query("INSERT INTO data.operation_views(operation_id,version,state,kind) VALUES($1,$2,$3,$4) ON CONFLICT(operation_id) DO UPDATE SET version=EXCLUDED.version,state=EXCLUDED.state,kind=EXCLUDED.kind,updated_at=now() WHERE data.operation_views.version<EXCLUDED.version")
            .bind(event.aggregate_id).bind(event.aggregate_version).bind(state).bind(kind).execute(&mut *tx).await?;
        sqlx::query(
            self.sql("INSERT INTO {schema}.inbox(consumer,event_id,fingerprint) VALUES($1,$2,$3)"),
        )
        .bind(consumer)
        .bind(event.event_id.0)
        .bind(digest)
        .execute(&mut *tx)
        .await?;
        sqlx::query(self.sql("UPDATE {schema}.consumer_versions SET version=$4 WHERE consumer=$1 AND producer=$2 AND aggregate_id=$3"))
            .bind(consumer).bind(&event.producer).bind(event.aggregate_id).bind(event.aggregate_version).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn replay<A: ReplayAuthorizer>(
        &self,
        consumer: &str,
        id: Uuid,
        actor: &ActorContext,
        authorization: &A,
    ) -> Result<bool> {
        ensure!(
            authorization.authorize_replay(actor).await?,
            "Event replay denied"
        );
        let document: serde_json::Value =
            sqlx::query_scalar(self.sql(
                "SELECT document FROM {schema}.dead_letters WHERE consumer=$1 AND event_id=$2",
            ))
            .bind(consumer)
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let request = Uuid::new_v4();
        sqlx::query(self.sql("INSERT INTO {schema}.replay_audit(id,consumer,event_id,actor_issuer,actor_subject,state) VALUES($1,$2,$3,$4,$5,'REQUESTED')"))
            .bind(request).bind(consumer).bind(id).bind(&actor.issuer).bind(&actor.subject).execute(&self.pool).await?;
        let applied = self
            .consume_operation(consumer, &serde_json::from_value(document)?)
            .await?;
        sqlx::query(self.sql("UPDATE {schema}.replay_audit SET state=$2 WHERE id=$1"))
            .bind(request)
            .bind(if applied { "APPLIED" } else { "PENDING" })
            .execute(&self.pool)
            .await?;
        if applied {
            sqlx::query(self.sql("UPDATE {schema}.dead_letters SET state='RESOLVED' WHERE consumer=$1 AND event_id=$2")).bind(consumer).bind(id).execute(&self.pool).await?;
        }
        Ok(applied)
    }
}

pub async fn operation_fact(
    tx: &mut Transaction<'_, Postgres>,
    pool: &PgPool,
    id: Uuid,
) -> Result<()> {
    let row = sqlx::query("SELECT * FROM platform.operations WHERE id=$1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    let version: i64 = row.get("version");
    let event = EventEnvelope {
        event_id: EventId::new(),
        event_type: "operation.changed".into(),
        schema_version: 1,
        producer: "platform".into(),
        aggregate_id: id,
        aggregate_version: version,
        occurred_at: chrono::Utc::now(),
        correlation_id: id,
        causation_id: id.to_string(),
        data: json!({"operationId":id,"state":row.get::<String,_>("state"),"kind":row.get::<String,_>("kind")}),
    };
    EventStore::new(pool.clone(), Module::Platform)
        .stage(tx, &event)
        .await
}

fn validate_transport(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value.split('.').all(|s| !s.is_empty())
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
        "Invalid local event namespace"
    );
    Ok(())
}
pub fn stream_name() -> Result<String> {
    let value = std::env::var("BANK_EVENT_STREAM").unwrap_or_else(|_| "BANK_EVENTS".into());
    validate_transport(&value)?;
    ensure!(!value.contains('.'), "Stream name cannot contain a dot");
    Ok(value)
}
pub fn subject_prefix() -> Result<String> {
    let value = std::env::var("BANK_EVENT_NAMESPACE").unwrap_or_else(|_| "bank.events".into());
    validate_transport(&value)?;
    Ok(value)
}
pub async fn jetstream(url: &str) -> Result<async_nats::jetstream::Context> {
    jetstream_named(url, &stream_name()?, &subject_prefix()?).await
}
pub async fn jetstream_named(
    url: &str,
    name: &str,
    prefix: &str,
) -> Result<async_nats::jetstream::Context> {
    validate_transport(name)?;
    validate_transport(prefix)?;
    ensure!(!name.contains('.'), "Invalid stream name");
    let context = async_nats::jetstream::new(async_nats::connect(url).await?);
    context
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: name.into(),
            subjects: vec![format!("{prefix}.>")],
            storage: async_nats::jetstream::stream::StorageType::File,
            max_bytes: 128 * 1024 * 1024,
            max_age: std::time::Duration::from_secs(7 * 86400),
            ..Default::default()
        })
        .await?;
    Ok(context)
}
