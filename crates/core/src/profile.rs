use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::{
    checks::{Checks, Outcome, Purpose},
    entitlements::Entitlements,
};
use bank_platform::{
    database::Module,
    domain::{ActorContext, EventEnvelope, EventId, PartyId, TrustContext, fingerprint},
    events::EventStore,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactInput {
    pub party_id: PartyId,
    pub email: String,
    pub expected_version: i64,
}
impl ContactInput {
    pub fn digest(&self) -> Result<String> {
        ensure!(
            self.email.len() <= 128
                && self.email.ends_with("@example.invalid")
                && self.email.bytes().filter(|b| *b == b'@').count() == 1
                && !self.email.starts_with('@'),
            "Synthetic contact address required"
        );
        Ok(fingerprint(self)?)
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerProfile {
    pub party_id: PartyId,
    pub version: i64,
    pub email: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
    pub verification_source: &'static str,
}
impl Core {
    pub async fn profile(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
    ) -> Result<CustomerProfile> {
        ensure!(
            actor.trust == TrustContext::Customer,
            "Customer profile context required"
        );
        let person = controls.person(actor).await?;
        let row=sqlx::query("SELECT p.id,p.profile_version,c.email,c.verified_at FROM core.parties p LEFT JOIN core.contacts c ON c.party_id=p.id WHERE p.id=$1 AND p.issuer=$2 AND p.subject=$3").bind(person.0).bind(&actor.issuer).bind(&actor.subject).fetch_one(&self.pool).await?;
        Ok(CustomerProfile {
            party_id: person,
            version: row.get("profile_version"),
            email: row.get("email"),
            verified_at: row.get("verified_at"),
            verification_source: "SIMULATED_CONTROL_PROVIDER",
        })
    }
    pub async fn apply_contact(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        checks: &Checks,
        id: Uuid,
        input: &ContactInput,
    ) -> Result<i64> {
        let profile = self.profile(actor, controls).await?;
        ensure!(
            profile.party_id == input.party_id,
            "Contact owner scope mismatch"
        );
        let gate = checks
            .evaluate(id, Purpose::Onboarding, &input.digest()?)
            .await?;
        ensure!(
            gate.outcome == Outcome::Allow,
            "Current contact verification and risk/screening required"
        );
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("SELECT * FROM core.change_contact($1,$2,$3,$4,$5)").bind(id).bind(input.party_id.0).bind(&input.email).bind(input.expected_version).bind(serde_json::json!({"revision":gate.revision,"checkIds":gate.check_ids,"expiresAt":gate.valid_until})).fetch_one(&mut *tx).await?;
        let version: i64 = row.get("version");
        if row.get("created") {
            EventStore::new(self.pool.clone(),Module::Core).stage(&mut tx,&EventEnvelope{event_id:EventId::new(),event_type:"profile.changed".into(),schema_version:1,producer:"core".into(),aggregate_id:input.party_id.0,aggregate_version:version,occurred_at:Utc::now(),correlation_id:id,causation_id:id.to_string(),data:serde_json::json!({"partyId":input.party_id,"verificationRef":id,"profileVersion":version})}).await?;
        }
        tx.commit().await?;
        Ok(version)
    }
    pub async fn contact_result(
        &self,
        actor: &ActorContext,
        controls: &Entitlements,
        id: Uuid,
    ) -> Result<Option<i64>> {
        let profile = self.profile(actor, controls).await?;
        Ok(sqlx::query_scalar(
            "SELECT version FROM core.contact_changes WHERE id=$1 AND party_id=$2",
        )
        .bind(id)
        .bind(profile.party_id.0)
        .fetch_optional(&self.pool)
        .await?)
    }
}
