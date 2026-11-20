use crate::{
    Payments,
    types::{Quote, QuoteRecord},
};
use anyhow::{Result, ensure};
use bank_core::payments::InternalInstruction;
use bank_platform::domain::{AccountId, ActorContext, Money, PaymentDetails, fingerprint};
use chrono::{Duration, NaiveDate, Utc};
use sqlx::Row;
use uuid::Uuid;
impl Payments {
    pub async fn quote(
        &self,
        actor: &ActorContext,
        payer: AccountId,
        beneficiary: Uuid,
        amount: Money,
        date: NaiveDate,
    ) -> Result<Quote> {
        amount.require_positive()?;
        self.authorize_amount(actor, payer, &amount).await?;
        ensure!(
            date == Utc::now().date_naive(),
            "Only immediate execution is enabled"
        );
        let (recipient, payee) = self.beneficiary(actor, beneficiary).await?;
        ensure!(
            recipient.state == "VERIFIED" && payee != payer,
            "Verified recipient required"
        );
        let policy_id = if recipient.rail == "SIMULATED" {
            self.rail_client
                .verify_recipient(&recipient.receiving_reference)
                .await?;
            let snapshot = self.rail_policy().await?;
            let policy = snapshot.enabled_policy()?;
            ensure!(
                amount.minor_units() <= policy.payment_limit.minor_units(),
                "Rail limit exceeded"
            );
            Some(policy.id)
        } else {
            ensure!(
                self.core
                    .resolve_receiver(&recipient.receiving_reference)
                    .await?
                    == payee,
                "Recipient restricted"
            );
            None
        };
        let product = self.core.payment_product(payer).await?;
        ensure!(
            date >= product.effective_from && date < product.effective_until,
            "Product policy expired"
        );
        let fee = product.fee(amount)?;
        let total = amount.checked_add(fee)?;
        let id = Uuid::new_v4();
        let payment_id = Uuid::new_v4();
        let expires = Utc::now() + Duration::minutes(3);
        let details = PaymentDetails {
            payer_account_id: payer,
            beneficiary_id: beneficiary.to_string(),
            amount,
            fee,
            requested_date: date,
            rail: recipient.rail.clone(),
            quote_id: id.to_string(),
        };
        let instruction = InternalInstruction {
            operation_id: payment_id,
            details,
            payee_account_id: payee,
            product_version_id: product.id,
            beneficiary_version: recipient.version,
            rail_policy_version_id: policy_id,
        };
        sqlx::query("INSERT INTO payments.quotes(id,issuer,subject,beneficiary_id,beneficiary_version,instruction,details_digest,total_minor,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
   .bind(id).bind(&actor.issuer).bind(&actor.subject).bind(beneficiary).bind(recipient.version).bind(serde_json::to_value(&instruction)?).bind(fingerprint(&instruction)?).bind(total.minor_units()).bind(expires).execute(&self.pool).await?;
        Ok(Quote {
            id,
            payment_id,
            beneficiary_id: beneficiary,
            beneficiary_label: recipient.label,
            payer_account_id: payer,
            amount,
            fee,
            total,
            requested_date: date,
            rail: recipient.rail.clone(),
            product_version_id: product.id,
            expires_at: expires,
        })
    }
    pub(crate) async fn quote_record(&self, id: Uuid) -> Result<QuoteRecord> {
        let row = sqlx::query("SELECT * FROM payments.quotes WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let instruction: InternalInstruction = serde_json::from_value(row.get("instruction"))?;
        ensure!(
            fingerprint(&instruction)? == row.get::<String, _>("details_digest"),
            "Immutable quote integrity mismatch"
        );
        Ok(QuoteRecord {
            id,
            beneficiary_id: row.get("beneficiary_id"),
            beneficiary_version: row.get("beneficiary_version"),
            instruction,
            expires_at: row.get("expires_at"),
        })
    }
    pub(crate) async fn owned_quote(&self, actor: &ActorContext, id: Uuid) -> Result<QuoteRecord> {
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM payments.quotes WHERE id=$1 AND issuer=$2 AND subject=$3)",
        )
        .bind(id)
        .bind(&actor.issuer)
        .bind(&actor.subject)
        .fetch_one(&self.pool)
        .await?;
        ensure!(owned, crate::ResourceUnavailable);
        self.quote_record(id).await
    }
    pub(crate) async fn current_quote(&self, actor: &ActorContext, q: &QuoteRecord) -> Result<()> {
        let (b, payee) = self.beneficiary(actor, q.beneficiary_id).await?;
        self.current_bound_quote(actor, q, &b, payee).await?;
        if b.rail == "SIMULATED" {
            self.rail_client
                .verify_recipient(&b.receiving_reference)
                .await?;
            Self::validate_rail_quote(q, &self.rail_policy().await?)?;
        }
        Ok(())
    }
    pub(crate) async fn current_bound_quote(
        &self,
        actor: &ActorContext,
        q: &QuoteRecord,
        b: &crate::types::Beneficiary,
        payee: AccountId,
    ) -> Result<()> {
        // Re-checked at intent creation and confirmation: a revoked delegation or lowered authority applies at once.
        self.authorize_amount(
            actor,
            q.instruction.details.payer_account_id,
            &q.instruction.details.amount,
        )
        .await?;
        ensure!(
            q.expires_at > Utc::now()
                && q.instruction.details.requested_date == Utc::now().date_naive(),
            "Quote expired"
        );
        ensure!(
            b.state == "VERIFIED"
                && b.version == q.beneficiary_version
                && payee == q.instruction.payee_account_id
                && b.rail == q.instruction.details.rail
                && (b.rail == "SIMULATED"
                    || self.core.resolve_receiver(&b.receiving_reference).await? == payee),
            "Beneficiary changed or restricted"
        );
        let product = self
            .core
            .payment_product(q.instruction.details.payer_account_id)
            .await?;
        ensure!(
            product.id == q.instruction.product_version_id
                && product.fee(q.instruction.details.amount)? == q.instruction.details.fee
                && product.effective_from <= Utc::now().date_naive()
                && product.effective_until > Utc::now().date_naive(),
            "Current product or fees changed"
        );
        Ok(())
    }
    pub(crate) fn validate_rail_quote(
        q: &QuoteRecord,
        snapshot: &crate::policy::RailPolicySnapshot,
    ) -> Result<()> {
        let policy = snapshot.enabled_policy()?;
        ensure!(
            q.instruction.rail_policy_version_id == Some(policy.id)
                && q.instruction.details.amount.minor_units() <= policy.payment_limit.minor_units(),
            "Rail policy changed or limit exceeded"
        );
        Ok(())
    }
}
