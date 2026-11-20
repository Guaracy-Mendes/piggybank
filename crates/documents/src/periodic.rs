//! Retained review of exact preview material. No financial consent or enrollment.
use crate::Terms;
use anyhow::{Result, ensure};
use bank_controls::entitlements::{Action, Entitlements};
use bank_core::{Core, accruals::disclosures::DisclosureMaterial};
use bank_platform::domain::{AccountId, ActorContext, PartyId, TrustContext, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const REVIEW_STATEMENT: &str = "I acknowledge reviewing this exact synthetic preview disclosure. This does not enroll my account, accept financial terms, authorize a charge or interest credit, or activate a policy.";
#[derive(Debug)]
pub struct ReviewInputError;
impl std::fmt::Display for ReviewInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Explicit review of the exact displayed preview is required")
    }
}
impl std::error::Error for ReviewInputError {}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewInput {
    pub disclosure_digest: String,
    pub acknowledged: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewReview {
    pub format_version: u8,
    pub source: String,
    pub kind: String,
    pub id: Uuid,
    pub account_id: Uuid,
    pub policy_id: Uuid,
    pub material: DisclosureMaterial,
    pub material_digest: String,
    pub reviewed_at: String,
    pub attribution_digest: String,
    pub statement: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewReviewReceipt {
    pub review: PreviewReview,
    pub digest: String,
}
impl PreviewReviewReceipt {
    pub fn validate(&self) -> Result<()> {
        let r = &self.review;
        r.material.validate_v1()?;
        let _: DateTime<Utc> = r.reviewed_at.parse()?;
        ensure!(
            r.format_version == 1
                && r.source == "DOCUMENTS_PERIODIC_PREVIEW_REVIEW_V1"
                && r.kind == "PREVIEW_REVIEW_ONLY"
                && !r.id.is_nil()
                && r.account_id == r.material.account_id.0
                && r.policy_id == r.material.policy.id
                && r.statement == REVIEW_STATEMENT
                && r.attribution_digest.len() == 64
                && r.attribution_digest
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                && fingerprint(&r.material)? == r.material_digest
                && fingerprint(r)? == self.digest,
            "Retained preview review integrity conflict"
        );
        Ok(())
    }
}
impl Terms {
    async fn preview_owner(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
    ) -> Result<PartyId> {
        if actor.trust != TrustContext::Customer {
            return Err(sqlx::Error::RowNotFound.into());
        }
        let authority = core.authority(account, Action::ReadAccount).await?;
        if !controls
            .authorize(actor, Action::ReadAccount, Some(&authority), account.0)
            .await?
        {
            return Err(sqlx::Error::RowNotFound.into());
        }
        controls.person(actor).await
    }
    async fn preview_receipt(
        &self,
        person: PartyId,
        account: AccountId,
        policy: Uuid,
    ) -> Result<PreviewReviewReceipt> {
        let raw: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT documents.periodic_preview_review($1,$2,$3)")
                .bind(account.0)
                .bind(policy)
                .bind(person.0)
                .fetch_one(&self.pool)
                .await?;
        Self::decode_preview_receipt(raw)
    }
    fn decode_preview_receipt(raw: Option<serde_json::Value>) -> Result<PreviewReviewReceipt> {
        let raw = raw.ok_or(sqlx::Error::RowNotFound)?;
        let receipt: PreviewReviewReceipt = serde_json::from_value(raw.clone())?;
        receipt.validate()?;
        ensure!(
            fingerprint(&raw)? == fingerprint(&receipt)?,
            "Preview receipt serialization conflict"
        );
        Ok(receipt)
    }
    pub async fn periodic_preview_review(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
        policy: Uuid,
    ) -> Result<PreviewReviewReceipt> {
        let person = self.preview_owner(core, controls, actor, account).await?;
        self.preview_receipt(person, account, policy).await
    }
    pub async fn acknowledge_periodic_preview(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        scope: (AccountId, Uuid, Uuid),
        input: &ReviewInput,
    ) -> Result<PreviewReviewReceipt> {
        let (account, policy, key) = scope;
        ensure!(
            input.acknowledged
                && !key.is_nil()
                && input.disclosure_digest.len() == 64
                && input
                    .disclosure_digest
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            ReviewInputError
        );
        let person = self.preview_owner(core, controls, actor, account).await?;
        let mut tx = self.pool.begin().await?;
        let id: Uuid = sqlx::query_scalar(
            "SELECT documents.acknowledge_periodic_preview($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(account.0)
        .bind(policy)
        .bind(key)
        .bind(serde_json::to_value(actor)?)
        .bind(person.0)
        .bind(&input.disclosure_digest)
        .bind(input.acknowledged)
        .fetch_one(&mut *tx)
        .await?;
        let raw: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT documents.periodic_preview_review($1,$2,$3)")
                .bind(account.0)
                .bind(policy)
                .bind(person.0)
                .fetch_one(&mut *tx)
                .await?;
        let receipt = Self::decode_preview_receipt(raw)?;
        ensure!(
            receipt.review.id == id,
            "Original preview review receipt required"
        );
        tx.commit().await?;
        Ok(receipt)
    }
}
