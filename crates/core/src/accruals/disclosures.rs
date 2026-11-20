//! Versioned customer information derived from immutable approved preview policies.
//! No consent, enrollment, balance input, evaluation-clock input or financial command.
use super::{Calculation, PeriodicInputError, PeriodicPolicy};
use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::entitlements::{Action, Entitlements};
use bank_platform::domain::{AccountId, ActorContext, TrustContext, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug)]
pub struct DisclosureLimit;
impl std::fmt::Display for DisclosureLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("More than 100 current or upcoming periodic disclosures")
    }
}
impl std::error::Error for DisclosureLimit {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisclosureMaterial {
    pub format_version: u8,
    pub source: String,
    pub account_id: AccountId,
    pub policy: PeriodicPolicy,
    pub policy_digest: String,
    pub title: String,
    pub text: String,
}
impl DisclosureMaterial {
    /// Retain the V1 algorithm for verification of independently archived material.
    pub fn validate_v1(&self) -> Result<()> {
        let expected = render(
            self.account_id,
            self.policy.clone(),
            self.policy_digest.clone(),
            Utc::now(),
        )?;
        ensure!(fingerprint(self)? == expected.digest, PeriodicInputError);
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicDisclosure {
    pub observed_at: DateTime<Utc>,
    pub availability: String,
    pub material: DisclosureMaterial,
    pub digest: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisclosurePage {
    pub account_id: AccountId,
    pub product_version_id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub items: Vec<PeriodicDisclosure>,
}
fn render(
    account: AccountId,
    policy: PeriodicPolicy,
    digest: String,
    observed_at: DateTime<Utc>,
) -> Result<PeriodicDisclosure> {
    policy.validate()?;
    ensure!(
        !account.0.is_nil() && fingerprint(&policy)? == digest,
        PeriodicInputError
    );
    let (title, price) = match &policy.calculation {
        Calculation::MonthlyFixedFee { amount } => {
            let n = amount.minor_units();
            (
                "Monthly fee preview",
                format!(
                    "The fixed monthly amount is TST {}.{:02} ({} minor units) for each complete calendar month. There is no proration and no balance-dependent fee formula.",
                    n / 100,
                    n % 100,
                    n
                ),
            )
        }
        Calculation::DailyCreditAccrual {
            annual_basis_points,
            ..
        } => (
            "Daily credit preview",
            format!(
                "The annual rate is {}.{:02}% ({} basis points). Each calendar day's nonnegative closing booked balance is multiplied by the annual basis points and divided by 3650000. ACT/365 fixed applies, including leap years. Each day's result rounds half up to one minor unit. Held funds remain separate from booked balance. This is a calculation preview, not an interest credit.",
                annual_basis_points / 100,
                annual_basis_points % 100,
                annual_basis_points
            ),
        ),
    };
    let period = match policy.calculation {
        Calculation::MonthlyFixedFee { .. } => "Each period is one complete calendar month.",
        Calculation::DailyCreditAccrual { .. } => {
            "Each period is one calendar day, including weekends and declared holidays. Only its due date rolls."
        }
    };
    let days = policy
        .calendar
        .business_weekdays
        .iter()
        .map(|d| {
            [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ][usize::from(*d) - 1]
        })
        .collect::<Vec<_>>()
        .join(", ");
    let holidays = policy
        .calendar
        .holidays
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let text = format!(
        "Synthetic preview only. TST has no monetary value. Reading this disclosure does not enroll your account, accept terms, activate a rule, debit a fee or credit interest.\n\nRule {} version {} covers {} inclusive to {} exclusive.\n\n{}\n\nThe frozen calendar is {} version {}, timezone UTC. Coverage is {} inclusive to {} exclusive. Business weekdays: {}. Explicit holidays: {}. No other holidays are inferred.\n\n{} The due time is {} UTC on the first declared business day on or after the exclusive period end, searching from the period end through fourteen following calendar days. Equality at cutoff is due.\n\nNo customer obligation or financial completion is established. Enrollment, material terms acceptance and financial activation require separate future controls. Changes require a new approved policy version and a new disclosure.",
        policy.code,
        policy.version,
        policy.effective_from,
        policy.effective_until,
        price,
        policy.calendar.code,
        policy.calendar.version,
        policy.calendar.coverage_from,
        policy.calendar.coverage_until,
        days,
        if holidays.is_empty() {
            "None declared"
        } else {
            &holidays
        },
        period,
        policy.calendar.cutoff
    );
    let availability = if observed_at.date_naive() < policy.effective_from {
        "UPCOMING_PREVIEW"
    } else if observed_at.date_naive() >= policy.effective_until {
        "ENDED_PREVIEW"
    } else {
        "CURRENT_PREVIEW"
    }
    .into();
    let material = DisclosureMaterial {
        format_version: 1,
        source: "CORE_APPROVED_PERIODIC_DISCLOSURE_V1".into(),
        account_id: account,
        policy,
        policy_digest: digest,
        title: title.into(),
        text,
    };
    Ok(PeriodicDisclosure {
        digest: fingerprint(&material)?,
        material,
        observed_at,
        availability,
    })
}
impl Core {
    pub async fn periodic_account_product(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
    ) -> Result<Uuid> {
        if actor.trust != TrustContext::Customer {
            return Err(sqlx::Error::RowNotFound.into());
        }
        let authority = self.authority(account, Action::ReadAccount).await?;
        if !controls
            .authorize(actor, Action::ReadAccount, Some(&authority), account.0)
            .await?
        {
            return Err(sqlx::Error::RowNotFound.into());
        }
        let person = controls.person(actor).await?;
        // Recheck the actual active ownership relation in the native product lookup.
        Ok(sqlx::query_scalar("SELECT r.product_version_id FROM core.account_registry r JOIN core.account_relationships x ON x.account_id=r.id WHERE r.id=$1 AND x.person_id=$2 AND x.kind='OWNER' AND x.revoked_at IS NULL")
            .bind(account.0).bind(person.0).fetch_one(&self.pool).await?)
    }
    pub async fn periodic_disclosures(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
    ) -> Result<DisclosurePage> {
        let product = self
            .periodic_account_product(controls, actor, account)
            .await?;
        let now = Utc::now();
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM core.periodic_policies WHERE product_version_id=$1 AND effective_until>$2 ORDER BY code,version,id LIMIT 101")
            .bind(product).bind(now.date_naive()).fetch_all(&self.pool).await?;
        ensure!(ids.len() <= 100, DisclosureLimit);
        let mut items = vec![];
        for id in ids {
            let saved = self
                .periodic_policies()
                .get(id)
                .await?
                .ok_or(sqlx::Error::RowNotFound)?;
            items.push(render(account, saved.policy, saved.digest, now)?);
        }
        Ok(DisclosurePage {
            account_id: account,
            product_version_id: product,
            observed_at: now,
            items,
        })
    }
    pub async fn periodic_disclosure(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        account: AccountId,
        policy: Uuid,
    ) -> Result<PeriodicDisclosure> {
        let product = self
            .periodic_account_product(controls, actor, account)
            .await?;
        let saved = self
            .periodic_policies()
            .get(policy)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        if saved.policy.product_version_id != product {
            return Err(sqlx::Error::RowNotFound.into());
        }
        render(account, saved.policy, saved.digest, Utc::now())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_policy_template_and_material_bind_stable_digest_without_observation_clock() {
        let policy = super::super::tests::policy();
        let account = AccountId::new();
        let a = render(
            account,
            policy.clone(),
            fingerprint(&policy).unwrap(),
            "2028-02-29T00:00:00Z".parse().unwrap(),
        )
        .unwrap();
        let b = render(
            account,
            policy.clone(),
            fingerprint(&policy).unwrap(),
            "2030-02-01T00:00:00Z".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(a.digest, b.digest);
        assert_eq!(a.availability, "CURRENT_PREVIEW");
        assert_eq!(b.availability, "ENDED_PREVIEW");
        let c = render(
            AccountId::new(),
            policy.clone(),
            fingerprint(&policy).unwrap(),
            a.observed_at,
        )
        .unwrap();
        assert_ne!(a.digest, c.digest);
        assert!(a.material.text.contains("3.65% (365 basis points)"));
        assert!(a.material.text.contains("3650000"));
    }
    #[test]
    fn large_fixed_amount_is_rendered_exactly_and_tampered_policy_digest_is_rejected() {
        let mut policy = super::super::tests::policy();
        policy.effective_from = "2028-02-01".parse().unwrap();
        policy.effective_until = "2028-03-01".parse().unwrap();
        policy.calculation = Calculation::MonthlyFixedFee {
            amount: bank_platform::domain::Money::new(
                9_007_199_254_740_993,
                bank_platform::domain::Currency::synthetic(),
            ),
        };
        let result = render(
            AccountId::new(),
            policy.clone(),
            fingerprint(&policy).unwrap(),
            Utc::now(),
        )
        .unwrap();
        assert!(
            result
                .material
                .text
                .contains("TST 90071992547409.93 (9007199254740993 minor units)")
        );
        assert!(render(AccountId::new(), policy, "0".repeat(64), Utc::now()).is_err());
    }
}
