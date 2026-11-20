//! Separately approved enrollment terms and explicit consent; Core owns activation and money.
use crate::Terms;
use anyhow::{Result, ensure};
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_core::{Core, accruals::PeriodicPolicy};
use bank_platform::domain::{AccountId, ActorContext, TrustContext, fingerprint};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug)]
pub struct OfferInputError;
impl std::fmt::Display for OfferInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Exact offer material and explicit enrollment choice required")
    }
}
impl std::error::Error for OfferInputError {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfferMaterial {
    pub format_version: u8,
    pub source: String,
    pub id: Uuid,
    pub kind: String,
    pub product_version_id: Uuid,
    pub policy: PeriodicPolicy,
    pub policy_digest: String,
    pub title: String,
    pub text: String,
}
impl OfferMaterial {
    pub fn validate(&self) -> Result<()> {
        self.policy.validate()?;
        ensure!(
            self.format_version == 1
                && self.source == "DOCUMENTS_PERIODIC_ENROLLMENT_TERMS_V1"
                && self.kind == "SYNTHETIC_PERIODIC_ENROLLMENT_OFFER"
                && !self.id.is_nil()
                && self.product_version_id == self.policy.product_version_id
                && fingerprint(&self.policy)? == self.policy_digest
                && !self.title.is_empty()
                && !self.text.is_empty()
                && self.text.len() <= 32768,
            "Offer material integrity conflict"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishedOffer {
    pub material: OfferMaterial,
    pub digest: String,
    pub published_at: String,
    pub publication_digest: String,
}
impl PublishedOffer {
    pub fn validate(&self) -> Result<()> {
        self.material.validate()?;
        let _: DateTime<Utc> = self.published_at.parse()?;
        ensure!(
            fingerprint(&self.material)? == self.digest && self.publication_digest.len() == 64,
            "Published offer integrity conflict"
        );
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfferInput {
    pub policy_id: Uuid,
    pub reason: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferRequest {
    pub id: Uuid,
    pub material: OfferMaterial,
    pub digest: String,
    pub reason: String,
    pub maker_person: Uuid,
    pub checker_person: Option<Uuid>,
    pub state: String,
    pub expires_at: DateTime<Utc>,
    pub publication: Option<PublishedOffer>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentInput {
    pub offer_digest: String,
    pub accepted: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WithdrawalInput {
    pub consent_digest: String,
    pub withdraw: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentConsent {
    pub format_version: u8,
    pub source: String,
    pub kind: String,
    pub id: Uuid,
    pub account_id: Uuid,
    pub offer: PublishedOffer,
    pub accepted_at: String,
    pub earliest_period_start: NaiveDate,
    pub attribution_digest: String,
    pub statement: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentWithdrawal {
    pub format_version: u8,
    pub source: String,
    pub kind: String,
    pub id: Uuid,
    pub consent_id: Uuid,
    pub consent_digest: String,
    pub withdrawn_at: String,
    pub attribution_digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentView {
    pub consent: EnrollmentConsent,
    pub consent_digest: String,
    pub withdrawal: Option<EnrollmentWithdrawal>,
    pub withdrawal_digest: Option<String>,
    pub status: String,
    pub financial_activation: String,
    pub activation: Option<serde_json::Value>,
}
impl EnrollmentView {
    pub fn validate(&self) -> Result<()> {
        let c = &self.consent;
        c.offer.validate()?;
        let at: DateTime<Utc> = c.accepted_at.parse()?;
        let policy = &c.offer.material.policy;
        let first = match policy.calculation {
            bank_core::accruals::Calculation::MonthlyFixedFee { .. } => {
                use chrono::Datelike;
                at.date_naive()
                    .with_day(1)
                    .unwrap()
                    .checked_add_months(chrono::Months::new(1))
                    .ok_or(OfferInputError)?
            }
            _ => at.date_naive().succ_opt().ok_or(OfferInputError)?,
        }
        .max(policy.effective_from);
        ensure!(
            c.format_version == 1
                && c.source == "DOCUMENTS_PERIODIC_ENROLLMENT_CONSENT_V1"
                && c.kind == "PERIODIC_ENROLLMENT_CONSENT"
                && !c.id.is_nil()
                && !c.account_id.is_nil()
                && fingerprint(c)? == self.consent_digest
                && c.earliest_period_start == first
                && first < policy.effective_until
                && c.attribution_digest.len() == 64
                && ["DISABLED", "ENABLED", "PAUSED"].contains(&self.financial_activation.as_str()),
            "Enrollment receipt integrity conflict"
        );
        match &self.activation {
            None => ensure!(
                self.financial_activation == "DISABLED"
                    && ["PENDING_ACTIVATION", "WITHDRAWN"].contains(&self.status.as_str()),
                "Missing activation proof"
            ),
            Some(raw) => {
                let a: bank_core::accruals::execution::ActivationReceipt =
                    serde_json::from_value(raw.clone())?;
                ensure!(
                    fingerprint(&a.activation)? == a.digest
                        && a.activation["consentId"] == serde_json::json!(c.id)
                        && a.activation["consentDigest"] == self.consent_digest
                        && a.activation["accountId"] == serde_json::json!(c.account_id)
                        && a.activation["policyDigest"] == c.offer.material.policy_digest
                        && a.activation["policy"] == serde_json::to_value(policy)?
                        && self.withdrawal.is_none()
                        && if a.stopped {
                            self.status == "PAUSED" && self.financial_activation == "PAUSED"
                        } else {
                            self.status == "ACTIVE" && self.financial_activation == "ENABLED"
                        },
                    "Activation receipt scope conflict"
                );
            }
        }
        match (&self.withdrawal, &self.withdrawal_digest) {
            (None, None) => ensure!(
                ["PENDING_ACTIVATION", "ACTIVE", "PAUSED"].contains(&self.status.as_str()),
                "Pending state conflict"
            ),
            (Some(w), Some(digest)) => {
                let _: DateTime<Utc> = w.withdrawn_at.parse()?;
                ensure!(
                    self.status == "WITHDRAWN"
                        && w.format_version == 1
                        && w.source == "DOCUMENTS_PERIODIC_ENROLLMENT_WITHDRAWAL_V1"
                        && w.kind == "PERIODIC_ENROLLMENT_WITHDRAWN"
                        && w.consent_id == c.id
                        && w.consent_digest == self.consent_digest
                        && fingerprint(w)? == *digest
                        && w.attribution_digest.len() == 64,
                    "Withdrawal integrity conflict"
                );
            }
            _ => anyhow::bail!("Incomplete withdrawal evidence"),
        }
        Ok(())
    }
}
fn decode_offer(raw: Option<serde_json::Value>) -> Result<PublishedOffer> {
    let raw = raw.ok_or(sqlx::Error::RowNotFound)?;
    let result: PublishedOffer = serde_json::from_value(raw.clone())?;
    result.validate()?;
    ensure!(
        fingerprint(&raw)? == fingerprint(&result)?,
        "Offer serialization conflict"
    );
    Ok(result)
}
fn decode_enrollment(raw: Option<serde_json::Value>) -> Result<EnrollmentView> {
    let raw = raw.ok_or(sqlx::Error::RowNotFound)?;
    let result: EnrollmentView = serde_json::from_value(raw.clone())?;
    result.validate()?;
    ensure!(
        fingerprint(&raw)? == fingerprint(&result)?,
        "Enrollment serialization conflict"
    );
    Ok(result)
}
impl Terms {
    async fn offer_staff(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        write: bool,
    ) -> Result<()> {
        ensure!(
            controls.finance_authorized(actor, write).await?,
            "Current Finance authority required"
        );
        Ok(())
    }
    pub async fn stage_periodic_offer(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        key: Uuid,
        input: &OfferInput,
    ) -> Result<OfferRequest> {
        self.offer_staff(controls, actor, true).await?;
        ensure!(
            !key.is_nil()
                && !input.policy_id.is_nil()
                && (4..=240).contains(&input.reason.chars().count())
                && input.reason.trim() == input.reason
                && !input.reason.chars().any(char::is_control),
            OfferInputError
        );
        let person = controls.person(actor).await?;
        let mut tx = self.pool.begin().await?;
        let id: Uuid = sqlx::query_scalar("SELECT documents.stage_periodic_offer($1,$2,$3,$4,$5)")
            .bind(input.policy_id)
            .bind(key)
            .bind(&input.reason)
            .bind(serde_json::to_value(actor)?)
            .bind(person.0)
            .fetch_one(&mut *tx)
            .await?;
        let material: serde_json::Value = sqlx::query_scalar(
            "SELECT material FROM documents.periodic_offer_requests WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        serde_json::from_value::<OfferMaterial>(material)?.validate()?;
        tx.commit().await?;
        self.periodic_offer_request(controls, actor, id).await
    }
    pub async fn periodic_offer_request(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<OfferRequest> {
        self.offer_staff(controls, actor, false).await?;
        let row = sqlx::query("SELECT * FROM documents.periodic_offer_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let material: OfferMaterial = serde_json::from_value(row.get("material"))?;
        material.validate()?;
        let digest: String = row.get("digest");
        let maker: Uuid = row.get("maker_person");
        let expires: DateTime<Utc> = row.get("expires_at");
        ensure!(
            material.id == id
                && material.policy.id == row.get::<Uuid, _>("policy_id")
                && fingerprint(&material)? == digest,
            "Offer request integrity conflict"
        );
        let control = controls
            .change_record(id, ChangeKind::PeriodicOffer)
            .await?;
        if let Some(c) = &control {
            ensure!(
                c.resource_id == id
                    && c.digest == digest
                    && c.maker_person == maker
                    && c.expires_at == expires,
                "Offer approval scope conflict"
            );
        }
        let raw: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT documents.published_periodic_offer($1)")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let publication = raw.map(|v| decode_offer(Some(v))).transpose()?;
        let state = if publication.is_some() {
            "PUBLISHED".into()
        } else if expires <= Utc::now() {
            "EXPIRED".into()
        } else {
            control
                .as_ref()
                .map(|c| c.state.clone())
                .unwrap_or("STAGED".into())
        };
        Ok(OfferRequest {
            id,
            material,
            digest,
            reason: row.get("reason"),
            maker_person: maker,
            checker_person: control.and_then(|c| c.checker_person),
            state,
            expires_at: expires,
            publication,
        })
    }
    pub async fn periodic_offer_requests(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
    ) -> Result<Vec<OfferRequest>> {
        self.offer_staff(controls, actor, false).await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM documents.periodic_offer_requests ORDER BY created_at DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        let mut out = vec![];
        for id in ids {
            out.push(self.periodic_offer_request(controls, actor, id).await?);
        }
        Ok(out)
    }
    pub async fn approve_periodic_offer(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<OfferRequest> {
        ensure!(
            controls.finance_approval_authorized(actor).await?,
            "Current Finance checker required"
        );
        let request = self.periodic_offer_request(controls, actor, id).await?;
        ensure!(
            request.digest == digest
                && ["STAGED", "PENDING", "APPROVED"].contains(&request.state.as_str()),
            "Exact current offer required"
        );
        let original: serde_json::Value = sqlx::query_scalar(
            "SELECT actor_context FROM documents.periodic_offer_requests WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let original: ActorContext = serde_json::from_value(original)?;
        self.offer_staff(controls, &original, true).await?;
        ensure!(
            controls.person(&original).await?.0 == request.maker_person,
            "Original canonical maker required"
        );
        controls
            .propose_change_id(
                &original,
                id,
                ChangeKind::PeriodicOffer,
                id,
                &request.material,
                request.expires_at,
            )
            .await?;
        controls.approve_change(actor, id).await?;
        self.periodic_offer_request(controls, actor, id).await
    }
    pub async fn publish_periodic_offer(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        id: Uuid,
        digest: &str,
    ) -> Result<OfferRequest> {
        self.offer_staff(controls, actor, true).await?;
        let request = self.periodic_offer_request(controls, actor, id).await?;
        let person = controls.person(actor).await?;
        ensure!(
            person.0 == request.maker_person && request.digest == digest,
            "Original exact maker publication required"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT documents.publish_periodic_offer($1,$2,$3)")
            .bind(id)
            .bind(serde_json::to_value(actor)?)
            .bind(person.0)
            .execute(&mut *tx)
            .await?;
        let raw: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT documents.published_periodic_offer($1)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        decode_offer(raw)?;
        tx.commit().await?;
        self.periodic_offer_request(controls, actor, id).await
    }
    pub async fn customer_periodic_offer(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
        id: Uuid,
    ) -> Result<PublishedOffer> {
        let product = core
            .periodic_account_product(controls, actor, account)
            .await?;
        let raw: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT documents.published_periodic_offer($1)")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let result = decode_offer(raw)?;
        if result.material.product_version_id != product {
            return Err(sqlx::Error::RowNotFound.into());
        }
        Ok(result)
    }
    pub async fn customer_periodic_offers(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
    ) -> Result<Vec<PublishedOffer>> {
        let product = core
            .periodic_account_product(controls, actor, account)
            .await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM documents.periodic_offers WHERE product_version_id=$1 AND (material->'policy'->>'effectiveUntil')::date>$2 ORDER BY policy_id,id LIMIT 101").bind(product).bind(Utc::now().date_naive()).fetch_all(&self.pool).await?;
        ensure!(ids.len() <= 100, OfferInputError);
        let mut out = vec![];
        for id in ids {
            out.push(
                self.customer_periodic_offer(core, controls, actor, account, id)
                    .await?,
            );
        }
        Ok(out)
    }
    pub async fn periodic_enrollment(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
        offer: Uuid,
    ) -> Result<EnrollmentView> {
        core.periodic_account_product(controls, actor, account)
            .await?;
        let person = controls.person(actor).await?;
        let raw: Option<serde_json::Value> =
            sqlx::query_scalar("SELECT documents.periodic_enrollment($1,$2,$3)")
                .bind(account.0)
                .bind(offer)
                .bind(person.0)
                .fetch_one(&self.pool)
                .await?;
        decode_enrollment(raw)
    }
    pub async fn request_periodic_enrollment(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        scope: (AccountId, Uuid, Uuid),
        input: &EnrollmentInput,
    ) -> Result<EnrollmentView> {
        self.periodic_enrollment_command(
            core,
            controls,
            actor,
            scope,
            ("ENROLL", &input.offer_digest, input.accepted),
        )
        .await
    }
    pub async fn withdraw_periodic_enrollment(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        scope: (AccountId, Uuid, Uuid),
        input: &WithdrawalInput,
    ) -> Result<EnrollmentView> {
        self.periodic_enrollment_command(
            core,
            controls,
            actor,
            scope,
            ("WITHDRAW", &input.consent_digest, input.withdraw),
        )
        .await
    }
    async fn periodic_enrollment_command(
        &self,
        core: &Core,
        controls: &Entitlements,
        actor: &ActorContext,
        scope: (AccountId, Uuid, Uuid),
        action: (&str, &str, bool),
    ) -> Result<EnrollmentView> {
        let (account, offer, key) = scope;
        let (kind, digest, explicit) = action;
        ensure!(
            actor.trust == TrustContext::Customer
                && !key.is_nil()
                && explicit
                && digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            OfferInputError
        );
        core.periodic_account_product(controls, actor, account)
            .await?;
        let person = controls.person(actor).await?;
        let mut tx = self.pool.begin().await?;
        let raw: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT documents.request_periodic_enrollment($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(account.0)
        .bind(offer)
        .bind(key)
        .bind(serde_json::to_value(actor)?)
        .bind(person.0)
        .bind(kind)
        .bind(digest)
        .bind(explicit)
        .fetch_one(&mut *tx)
        .await?;
        let result = decode_enrollment(raw)?;
        ensure!(
            result.consent.account_id == account.0 && result.consent.offer.material.id == offer,
            "Original enrollment scope required"
        );
        tx.commit().await?;
        Ok(result)
    }
}
