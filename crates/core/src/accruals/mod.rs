//! Immutable preview policies. This capability cannot post money or enroll an account.
pub mod basis;
use crate::{Core, products::ProductVersion};
use anyhow::{Result, ensure};
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_platform::domain::{ActorContext, Money, fingerprint};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug)]
pub struct PeriodicConflict;
impl std::fmt::Display for PeriodicConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Periodic policy identity, version or approval conflict")
    }
}
impl std::error::Error for PeriodicConflict {}
#[derive(Debug)]
pub struct PeriodicInputError;
impl std::fmt::Display for PeriodicInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Invalid periodic preview policy, calendar or basis")
    }
}
impl std::error::Error for PeriodicInputError {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessCalendar {
    pub id: Uuid,
    pub code: String,
    pub version: i32,
    pub timezone: String,
    pub coverage_from: NaiveDate,
    pub coverage_until: NaiveDate,
    /// ISO weekday numbers, strictly ascending. Holidays use the same timezone.
    pub business_weekdays: Vec<u8>,
    pub holidays: Vec<NaiveDate>,
    /// Exact HH:MM:SS in the declared timezone. Equality is due.
    pub cutoff: String,
    pub roll_convention: String,
}
fn code(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.as_bytes()[0].is_ascii_uppercase()
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
impl BusinessCalendar {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_nil()
                && code(&self.code)
                && self.version > 0
                && self.timezone == "UTC"
                && self.roll_convention == "FOLLOWING_14_DAYS"
                && (1..=400).contains(&(self.coverage_until - self.coverage_from).num_days())
                && !self.business_weekdays.is_empty()
                && self.business_weekdays.len() <= 7
                && self.business_weekdays.iter().all(|d| (1..=7).contains(d))
                && self.business_weekdays.windows(2).all(|w| w[0] < w[1])
                && self.holidays.len() <= 366
                && self.holidays.windows(2).all(|w| w[0] < w[1])
                && self
                    .holidays
                    .iter()
                    .all(|d| *d >= self.coverage_from && *d < self.coverage_until),
            PeriodicInputError
        );
        let time =
            NaiveTime::parse_from_str(&self.cutoff, "%H:%M:%S").map_err(|_| PeriodicInputError)?;
        ensure!(
            time.nanosecond() == 0 && time.format("%H:%M:%S").to_string() == self.cutoff,
            PeriodicInputError
        );
        Ok(())
    }
    fn due(&self, end: NaiveDate) -> Result<DateTime<Utc>> {
        for offset in 0..=14 {
            let date = end
                .checked_add_signed(Duration::days(offset))
                .ok_or(PeriodicInputError)?;
            ensure!(
                date >= self.coverage_from && date < self.coverage_until,
                PeriodicInputError
            );
            if self
                .business_weekdays
                .contains(&(date.weekday().number_from_monday() as u8))
                && self.holidays.binary_search(&date).is_err()
            {
                return Ok(date
                    .and_time(NaiveTime::parse_from_str(&self.cutoff, "%H:%M:%S")?)
                    .and_utc());
            }
        }
        anyhow::bail!(PeriodicInputError)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Calculation {
    #[serde(rename_all = "camelCase")]
    MonthlyFixedFee { amount: Money },
    #[serde(rename_all = "camelCase")]
    DailyCreditAccrual {
        annual_basis_points: u16,
        day_count: String,
        basis: String,
        rounding: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeriodicPolicy {
    pub id: Uuid,
    pub code: String,
    pub version: i32,
    pub label: String,
    pub activation: String,
    pub product_version_id: Uuid,
    pub effective_from: NaiveDate,
    pub effective_until: NaiveDate,
    pub calendar: BusinessCalendar,
    pub calculation: Calculation,
}
impl PeriodicPolicy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_nil()
                && !self.product_version_id.is_nil()
                && code(&self.code)
                && self.version > 0
                && self.label == "SYNTHETIC"
                && self.activation == "PREVIEW_ONLY"
                && self.effective_from.year() >= 2000
                && self.effective_until.year() <= 2200
                && (1..=366).contains(&(self.effective_until - self.effective_from).num_days()),
            PeriodicInputError
        );
        self.calendar.validate()?;
        ensure!(
            self.calendar.coverage_from <= self.effective_from
                && self.calendar.coverage_until
                    > self
                        .effective_until
                        .checked_add_signed(Duration::days(14))
                        .ok_or(PeriodicInputError)?,
            PeriodicInputError
        );
        match &self.calculation {
            Calculation::MonthlyFixedFee { amount } => {
                ensure!(
                    amount.minor_units() > 0
                        && self.effective_from.day() == 1
                        && self.effective_until.day() == 1,
                    PeriodicInputError
                );
            }
            Calculation::DailyCreditAccrual {
                annual_basis_points,
                day_count,
                basis,
                rounding,
            } => {
                ensure!(
                    *annual_basis_points > 0
                        && *annual_basis_points <= 10000
                        && day_count == "ACT_365_FIXED"
                        && basis == "NONNEGATIVE_CLOSING_BOOKED"
                        && rounding == "HALF_UP_EACH_DAY",
                    PeriodicInputError
                );
            }
        }
        // Validate every possible close, including holiday closures, before publication.
        let mut date = self.effective_from;
        while date < self.effective_until {
            let end = self.period_end(date)?;
            self.calendar.due(end)?;
            date = end;
        }
        Ok(())
    }
    fn period_end(&self, start: NaiveDate) -> Result<NaiveDate> {
        match self.calculation {
            Calculation::MonthlyFixedFee { .. } => {
                ensure!(start.day() == 1, PeriodicInputError);
                Ok(start
                    .checked_add_months(chrono::Months::new(1))
                    .ok_or(PeriodicInputError)?)
            }
            Calculation::DailyCreditAccrual { .. } => {
                Ok(start.succ_opt().ok_or(PeriodicInputError)?)
            }
        }
    }
    pub fn preview(&self, input: &PreviewInput) -> Result<PeriodicPreview> {
        self.validate()?;
        let end = self.period_end(input.period_start)?;
        ensure!(
            input.period_start >= self.effective_from && end <= self.effective_until,
            PeriodicInputError
        );
        let (amount, numerator, denominator, basis) = match &self.calculation {
            Calculation::MonthlyFixedFee { amount } => {
                ensure!(input.closing_booked.is_none(), PeriodicInputError);
                (*amount, amount.minor_units().to_string(), "1".into(), None)
            }
            Calculation::DailyCreditAccrual {
                annual_basis_points,
                ..
            } => {
                let basis = input.closing_booked.ok_or(PeriodicInputError)?;
                ensure!(basis.minor_units() >= 0, PeriodicInputError);
                let numerator = i128::from(basis.minor_units()) * i128::from(*annual_basis_points);
                let denominator = 10_000_i128 * 365;
                let rounded = i64::try_from((numerator + denominator / 2) / denominator)?;
                (
                    Money::new(rounded, basis.currency()),
                    numerator.to_string(),
                    denominator.to_string(),
                    Some(basis),
                )
            }
        };
        let due_at = self.calendar.due(end)?;
        Ok(PeriodicPreview {
            policy_id: self.id,
            policy_digest: fingerprint(self)?,
            activation: self.activation.clone(),
            period_start: input.period_start,
            period_end: end,
            due_at,
            evaluated_at: input.evaluated_at,
            is_due: input.evaluated_at >= due_at,
            basis_source: "OPERATOR_SUPPLIED_PREVIEW_ONLY".into(),
            closing_booked: basis,
            amount,
            exact_numerator: numerator,
            exact_denominator: denominator,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewInput {
    pub period_start: NaiveDate,
    pub closing_booked: Option<Money>,
    pub evaluated_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicPreview {
    pub policy_id: Uuid,
    pub policy_digest: String,
    pub activation: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub due_at: DateTime<Utc>,
    pub evaluated_at: DateTime<Utc>,
    pub is_due: bool,
    pub basis_source: String,
    pub closing_booked: Option<Money>,
    pub amount: Money,
    pub exact_numerator: String,
    pub exact_denominator: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedPolicy {
    pub policy: PeriodicPolicy,
    pub digest: String,
    pub proposal_id: Uuid,
    pub maker_person: Uuid,
    pub checker_person: Uuid,
    pub published_at: DateTime<Utc>,
}
/// Narrow configuration capability. FinanceRecords remains observation-only.
#[derive(Clone)]
pub struct PolicyRegistry {
    pool: PgPool,
}
impl Core {
    pub fn periodic_policies(&self) -> PolicyRegistry {
        PolicyRegistry {
            pool: self.pool.clone(),
        }
    }
}
impl PolicyRegistry {
    pub async fn get(&self, id: Uuid) -> Result<Option<PublishedPolicy>> {
        let Some(r) = sqlx::query("SELECT * FROM core.periodic_policies WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };
        let policy: PeriodicPolicy = serde_json::from_value(r.get("document"))?;
        policy.validate()?;
        let digest: String = r.get("digest");
        ensure!(
            fingerprint(&policy)? == digest
                && policy.id == id
                && policy.code == r.get::<String, _>("code")
                && policy.version == r.get::<i32, _>("version")
                && policy.product_version_id == r.get::<Uuid, _>("product_version_id")
                && policy.effective_from == r.get::<NaiveDate, _>("effective_from")
                && policy.effective_until == r.get::<NaiveDate, _>("effective_until"),
            PeriodicConflict
        );
        Ok(Some(PublishedPolicy {
            policy,
            digest,
            proposal_id: r.get("approval_id"),
            maker_person: r.get("maker_person"),
            checker_person: r.get("checker_person"),
            published_at: r.get("published_at"),
        }))
    }
    pub async fn resolve(
        &self,
        product: Uuid,
        code: &str,
        date: NaiveDate,
    ) -> Result<Option<PublishedPolicy>> {
        let id:Option<Uuid> = sqlx::query_scalar("SELECT id FROM core.periodic_policies WHERE product_version_id=$1 AND code=$2 AND effective_from<=$3 AND effective_until>$3").bind(product).bind(code).bind(date).fetch_optional(&self.pool).await?;
        match id {
            Some(id) => self.get(id).await,
            None => Ok(None),
        }
    }
    pub async fn publish(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        proposal: Uuid,
        policy: &PeriodicPolicy,
    ) -> Result<PublishedPolicy> {
        self.validate_product(policy).await?;
        ensure!(
            controls.finance_authorized(actor, true).await?,
            "Current Finance maker authority required"
        );
        let maker = controls.person(actor).await?.0;
        if let Some(saved) = self.get(policy.id).await? {
            ensure!(
                saved.digest == fingerprint(policy)?
                    && saved.proposal_id == proposal
                    && saved.maker_person == maker,
                PeriodicConflict
            );
            return Ok(saved);
        }
        let approved = controls
            .approved_change(
                actor,
                proposal,
                ChangeKind::PeriodicPolicy,
                policy.id,
                policy,
            )
            .await?;
        let (approved_maker, checker) = approved.people();
        ensure!(approved_maker == maker, PeriodicConflict);
        sqlx::query("SELECT core.publish_periodic_policy($1,$2,$3,$4,$5)")
            .bind(serde_json::to_value(policy)?)
            .bind(fingerprint(policy)?)
            .bind(proposal)
            .bind(maker)
            .bind(checker)
            .execute(&self.pool)
            .await?;
        self.get(policy.id)
            .await?
            .ok_or_else(|| PeriodicConflict.into())
    }
    pub async fn validate_product(&self, policy: &PeriodicPolicy) -> Result<()> {
        policy.validate()?;
        let doc: serde_json::Value =
            sqlx::query_scalar("SELECT document FROM core.product_versions WHERE id=$1")
                .bind(policy.product_version_id)
                .fetch_one(&self.pool)
                .await?;
        let product: ProductVersion = serde_json::from_value(doc)?;
        product.validate()?;
        ensure!(
            policy.effective_from >= product.effective_from
                && policy.effective_until <= product.effective_until,
            PeriodicInputError
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests;
