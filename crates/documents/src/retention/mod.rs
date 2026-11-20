//! WP-25 fixture retention: legal holds with independent release, permitted disposal evidence
//! and verified data-subject requests with traceable allowed/restricted outcomes.
use crate::statements::{Bind, Records};
use anyhow::{Result, ensure};
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, TrustContext};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Disposal was refused by retention, legal hold or policy. The refusal is recorded as evidence.
#[derive(Debug)]
pub struct Restricted(pub Value);
impl std::fmt::Display for Restricted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Disposal restricted")
    }
}
impl std::error::Error for Restricted {}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HoldInput {
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person_id: Option<Uuid>,
    pub reason: String,
    pub reference: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataRequestInput {
    pub kind: String,
    #[serde(default)]
    pub detail: String,
}
async fn workforce(controls: &Entitlements, actor: &ActorContext) -> Result<(Value, Uuid)> {
    ensure!(actor.trust == TrustContext::Workforce, "Workforce required");
    Ok((
        serde_json::to_value(actor)?,
        controls.person(actor).await?.0,
    ))
}
impl Records {
    pub async fn place_hold(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        key: Uuid,
        input: &HoldInput,
    ) -> Result<Value> {
        let (a, p) = workforce(controls, actor).await?;
        self.call(
            "SELECT documents.place_legal_hold($1,$2,$3,$4)",
            vec![
                Bind::Json(a),
                Bind::Id(p),
                Bind::Id(key),
                Bind::Json(serde_json::to_value(input)?),
            ],
        )
        .await
    }
    pub async fn release_hold(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        hold: Uuid,
        reason: &str,
    ) -> Result<Value> {
        ensure!(
            (3..=500).contains(&reason.trim().len()),
            "Release reason required"
        );
        let (a, p) = workforce(controls, actor).await?;
        self.call(
            "SELECT documents.release_legal_hold($1,$2,$3,$4)",
            vec![
                Bind::Json(a),
                Bind::Id(p),
                Bind::Id(hold),
                Bind::Text(reason.into()),
            ],
        )
        .await
    }
    pub async fn dispose(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        version: Uuid,
        reason: &str,
    ) -> Result<Value> {
        let (a, p) = workforce(controls, actor).await?;
        let result = self
            .call(
                "SELECT documents.dispose_statement($1,$2,$3,$4)",
                vec![
                    Bind::Json(a),
                    Bind::Id(p),
                    Bind::Id(version),
                    Bind::Text(reason.into()),
                ],
            )
            .await?;
        if result["status"]
            .as_str()
            .is_some_and(|s| s.starts_with("RESTRICTED"))
        {
            return Err(Restricted(result).into());
        }
        Ok(result)
    }
    pub async fn dispose_export(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        request: Uuid,
        reason: &str,
    ) -> Result<Value> {
        let (a, p) = workforce(controls, actor).await?;
        let result = self
            .call(
                "SELECT documents.dispose_data_export($1,$2,$3,$4)",
                vec![
                    Bind::Json(a),
                    Bind::Id(p),
                    Bind::Id(request),
                    Bind::Text(reason.into()),
                ],
            )
            .await?;
        if result["status"]
            .as_str()
            .is_some_and(|s| s.starts_with("RESTRICTED"))
        {
            return Err(Restricted(result).into());
        }
        Ok(result)
    }
    pub async fn submit_request(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        key: Uuid,
        input: &DataRequestInput,
    ) -> Result<Value> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        let person = controls.person(actor).await?;
        self.call(
            "SELECT documents.submit_data_request($1,$2,$3,$4,$5)",
            vec![
                Bind::Json(serde_json::to_value(actor)?),
                Bind::Id(person.0),
                Bind::Id(key),
                Bind::Text(input.kind.clone()),
                Bind::Text(input.detail.clone()),
            ],
        )
        .await
    }
    pub async fn process_request(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        request: Uuid,
    ) -> Result<Value> {
        let (a, p) = workforce(controls, actor).await?;
        self.call(
            "SELECT documents.process_data_request($1,$2,$3)",
            vec![Bind::Json(a), Bind::Id(p), Bind::Id(request)],
        )
        .await
    }
    pub async fn data_requests(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<Value> {
        let (a, p) = workforce(controls, actor).await?;
        self.call(
            "SELECT documents.staff_data_requests($1,$2)",
            vec![Bind::Json(a), Bind::Id(p)],
        )
        .await
    }
}
