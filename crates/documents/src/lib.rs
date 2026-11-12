//! Immutable synthetic terms and attributable, scope-bound acceptance evidence.
use anyhow::{Result, ensure};
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_core::Core;
use bank_platform::{
    database::Module,
    domain::{ActorContext, EventEnvelope, EventId, TrustContext},
    events::EventStore,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TermsDraft {
    pub id: Uuid,
    pub product_version_id: Uuid,
    pub version: i32,
    pub title: String,
    pub text: String,
    pub label: String,
    pub effective_from: NaiveDate,
    pub effective_until: NaiveDate,
}
impl TermsDraft {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.label == "SYNTHETIC"
                && self.version > 0
                && !self.title.trim().is_empty()
                && self.title.len() <= 100
                && !self.text.trim().is_empty()
                && self.text.len() <= 32768
                && self.effective_until > self.effective_from,
            "Invalid synthetic terms"
        );
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Acceptance {
    pub id: Uuid,
    pub terms_id: Uuid,
    pub product_version_id: Uuid,
    pub application_id: Uuid,
    pub digest: String,
    pub channel: String,
    pub accepted_at: DateTime<Utc>,
}
#[derive(Clone)]
pub struct Terms {
    pub pool: PgPool,
}
impl Terms {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn publish(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        proposal: Uuid,
        draft: &TermsDraft,
    ) -> Result<()> {
        draft.validate()?;
        let product = core.product(draft.product_version_id).await?;
        ensure!(
            draft.effective_from >= product.effective_from
                && draft.effective_until <= product.effective_until,
            "Terms must fit the published offer"
        );
        let proof = controls
            .approved_change(actor, proposal, ChangeKind::Terms, draft.id, draft)
            .await?;
        let (maker, checker) = proof.people();
        let mut tx = self.pool.begin().await?;
        let created: bool = sqlx::query_scalar("SELECT documents.publish_terms($1,$2,$3,$4)")
            .bind(serde_json::to_value(draft)?)
            .bind(proof.id())
            .bind(maker)
            .bind(checker)
            .fetch_one(&mut *tx)
            .await?;
        if created {
            self.fact(&mut tx,draft.id,"terms.published",serde_json::json!({"termsId":draft.id,"productVersionId":draft.product_version_id,"version":draft.version}),proposal).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn version(&self, id: Uuid) -> Result<TermsDraft> {
        Ok(serde_json::from_value(
            sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT document FROM documents.terms WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&self.pool)
            .await?,
        )?)
    }
    pub async fn current(&self, product: Uuid, date: NaiveDate) -> Result<TermsDraft> {
        let value:serde_json::Value=sqlx::query_scalar("SELECT document FROM documents.terms WHERE product_version_id=$1 AND effective_from<=$2 AND effective_until>$2 ORDER BY version DESC LIMIT 1").bind(product).bind(date).fetch_one(&self.pool).await?;
        Ok(serde_json::from_value(value)?)
    }
    pub async fn accept(
        &self,
        actor: &ActorContext,
        terms: Uuid,
        application: Uuid,
    ) -> Result<Acceptance> {
        ensure!(
            actor.trust == TrustContext::Customer,
            "Customer terms acceptance required"
        );
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM documents.accept_terms($1,$2,$3,$4,$5)")
            .bind(terms)
            .bind(application)
            .bind(&actor.issuer)
            .bind(&actor.subject)
            .bind(&actor.client_id)
            .fetch_one(&mut *tx)
            .await?;
        let id = row.get("acceptance_id");
        if row.get("created") {
            self.fact(
                &mut tx,
                id,
                "terms.accepted",
                serde_json::json!({"acceptanceId":id,"termsId":terms,"applicationId":application}),
                application,
            )
            .await?;
        }
        tx.commit().await?;
        self.acceptance(actor, id).await
    }
    pub async fn acceptance(&self, actor: &ActorContext, id: Uuid) -> Result<Acceptance> {
        let row = sqlx::query(
            "SELECT * FROM documents.acceptances WHERE id=$1 AND issuer=$2 AND subject=$3",
        )
        .bind(id)
        .bind(&actor.issuer)
        .bind(&actor.subject)
        .fetch_one(&self.pool)
        .await?;
        Ok(Acceptance {
            id: row.get("id"),
            terms_id: row.get("terms_id"),
            product_version_id: row.get("product_version_id"),
            application_id: row.get("application_id"),
            digest: row.get("digest"),
            channel: row.get("channel"),
            accepted_at: row.get("accepted_at"),
        })
    }
    pub async fn validate_acceptance(
        &self,
        actor: &ActorContext,
        id: Uuid,
        application: Uuid,
        product: Uuid,
    ) -> Result<Acceptance> {
        let acceptance = self.acceptance(actor, id).await?;
        let current = self.current(product, Utc::now().date_naive()).await?;
        ensure!(
            acceptance.application_id == application
                && acceptance.product_version_id == product
                && acceptance.terms_id == current.id,
            "Current acceptance scope mismatch"
        );
        use sha2::{Digest, Sha256};
        ensure!(
            acceptance.digest == format!("{:x}", Sha256::digest(current.text.as_bytes())),
            "Accepted text integrity mismatch"
        );
        Ok(acceptance)
    }
    async fn fact(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        event: &str,
        data: serde_json::Value,
        correlation: Uuid,
    ) -> Result<()> {
        EventStore::new(self.pool.clone(), Module::Documents)
            .stage(
                tx,
                &EventEnvelope {
                    event_id: EventId::new(),
                    event_type: event.into(),
                    schema_version: 1,
                    producer: "documents".into(),
                    aggregate_id: id,
                    aggregate_version: 1,
                    occurred_at: Utc::now(),
                    correlation_id: correlation,
                    causation_id: correlation.to_string(),
                    data,
                },
            )
            .await
    }
}
