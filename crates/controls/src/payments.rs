use crate::entitlements::{Action, Entitlements, ResourceAuthority};
use anyhow::{Result, ensure};
use bank_platform::domain::{ActorContext, PaymentDetails, TrustContext, fingerprint};
use chrono::{DateTime, Utc};
use uuid::Uuid;
impl Entitlements {
    pub async fn approved_payment_until(
        &self,
        actor: &ActorContext,
        challenge: Uuid,
        operation: Uuid,
        details: &PaymentDetails,
        authority: &ResourceAuthority,
    ) -> Result<DateTime<Utc>> {
        ensure!(
            self.authorize(actor, Action::CreatePayment, Some(authority), operation)
                .await?,
            "Current payment authority denied"
        );
        let person = self.person(actor).await?;
        Ok(sqlx::query_scalar("SELECT expires_at FROM controls.approval_challenges WHERE id=$1 AND operation_id=$2 AND issuer=$3 AND subject=$4 AND person_id=$5 AND intent_digest=$6 AND state='APPROVED' AND expires_at>now()")
   .bind(challenge).bind(operation).bind(&actor.issuer).bind(&actor.subject).bind(person.0).bind(fingerprint(details)?).fetch_one(&self.pool).await?)
    }
    pub async fn payment_inquiry_authorized(&self, actor: &ActorContext) -> Result<bool> {
        if actor.trust != TrustContext::Workforce
            || !self
                .authorize(actor, Action::InquirePayment, None, Uuid::new_v4())
                .await?
        {
            return Ok(false);
        }
        let person = self.person(actor).await?;
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.case_queue_members WHERE person_id=$1 AND queue='payments-operations' AND capability='REVIEW' AND revoked_at IS NULL)").bind(person.0).fetch_one(&self.pool).await?)
    }
}
