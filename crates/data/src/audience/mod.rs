//! WP-38 audiences and journey-partner orchestration with one local partner simulator role (`SIM_JOURNEY`). Audiences are
//! built from minimized warehouse signals and checked per member for consent and restrictions; exports carry pseudonymous
//! tokens only and re-check consent at export and at delivery. A partner can only return advisory recommendations, which
//! become insights: no path here approves customers, changes entitlements or posts money.
use crate::alerts::actor_person;
use crate::analytics::Denied;
use anyhow::{Result, bail, ensure};
use bank_adapters::journeys;
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, TrustContext, fingerprint};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Missing or wrong partner signature; the API answers 401 and records nothing.
#[derive(Debug)]
pub struct InvalidSignature;
impl std::fmt::Display for InvalidSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid journey-partner signature")
    }
}
impl std::error::Error for InvalidSignature {}

#[derive(Clone)]
pub struct Audiences {
    pub pool: PgPool,
}
fn created(result: Value) -> Result<Value> {
    if result["outcome"] == "DENIED" {
        bail!(Denied(
            result["reason"].as_str().unwrap_or("DENIED").to_owned()
        ));
    }
    Ok(result)
}
impl Audiences {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn staff(&self, controls: &Entitlements, actor: &ActorContext) -> Result<(Value, Uuid)> {
        actor_person(controls, actor, TrustContext::Workforce).await
    }
    pub async fn view(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        Ok(sqlx::query_scalar("SELECT analytics.audience_view($1,$2)")
            .bind(a)
            .bind(p)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn build(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        segment: &str,
        version: i32,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        created(
            sqlx::query_scalar("SELECT analytics.build_audience($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(segment)
                .bind(version)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn export(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        audience: Uuid,
        partner: &str,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        created(
            sqlx::query_scalar("SELECT analytics.export_audience($1,$2,$3,$4)")
                .bind(a)
                .bind(p)
                .bind(audience)
                .bind(partner)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn report(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        audience: Uuid,
    ) -> Result<Value> {
        let (a, p) = self.staff(controls, actor).await?;
        created(
            sqlx::query_scalar("SELECT analytics.outcome_report($1,$2,$3)")
                .bind(a)
                .bind(p)
                .bind(audience)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Partner fetch of its own export. Returns the response bytes and their signature.
    pub async fn partner_export(
        &self,
        partner: &str,
        export: Uuid,
        signature: &str,
    ) -> Result<(Vec<u8>, String)> {
        ensure!(
            journeys::verify(
                partner,
                &journeys::export_request(partner, &export.to_string()),
                signature
            ),
            InvalidSignature
        );
        let document: Value = sqlx::query_scalar("SELECT analytics.partner_export($1,$2)")
            .bind(partner)
            .bind(export)
            .fetch_one(&self.pool)
            .await?;
        self.signed(partner, &document)
    }
    /// Partner recommendation. Only an advisory insight can result; anything else is rejected and recorded.
    pub async fn partner_recommendation(
        &self,
        partner: &str,
        body: &[u8],
        signature: &str,
    ) -> Result<(Vec<u8>, String)> {
        ensure!(journeys::verify(partner, body, signature), InvalidSignature);
        let document: Value = serde_json::from_slice(body)?;
        let message = document["messageId"]
            .as_str()
            .filter(|m| (8..=64).contains(&m.len()))
            .ok_or_else(|| anyhow::anyhow!("messageId required"))?
            .to_owned();
        ensure!(document.is_object(), "object required");
        let result: Value =
            sqlx::query_scalar("SELECT analytics.partner_recommendation($1,$2,$3,$4)")
                .bind(partner)
                .bind(&message)
                .bind(fingerprint(&document)?)
                .bind(&document)
                .fetch_one(&self.pool)
                .await?;
        self.signed(partner, &result)
    }
    fn signed(&self, partner: &str, document: &Value) -> Result<(Vec<u8>, String)> {
        let raw = serde_json::to_vec(document)?;
        let signature = journeys::sign(partner, &raw).ok_or(InvalidSignature)?;
        Ok((raw, signature))
    }
}
