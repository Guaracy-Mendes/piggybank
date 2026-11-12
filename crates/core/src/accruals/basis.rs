//! Immutable observations of Core's business-date journal prefix; no posting authority.
use super::{Calculation, PeriodicPolicy, PreviewInput};
use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, Currency, Money, PartyId, TrustContext, fingerprint};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::BTreeSet;
use uuid::Uuid;
#[derive(Debug)]
pub struct BasisConflict;
impl std::fmt::Display for BasisConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Native basis scope or artifact integrity conflict")
    }
}
impl std::error::Error for BasisConflict {}
#[derive(Debug)]
pub struct BasisInputError;
impl std::fmt::Display for BasisInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Valid observed account and exact policy period required")
    }
}
impl std::error::Error for BasisInputError {}
#[derive(Debug)]
pub struct BasisLimit;
impl std::fmt::Display for BasisLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Native basis exceeds 1000 historical account journal facts")
    }
}
impl std::error::Error for BasisLimit {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasisInput {
    pub policy_id: Uuid,
    pub account_id: Uuid,
    pub period_start: NaiveDate,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasisItem {
    pub ordinal: u32,
    pub batch_id: Uuid,
    pub operation_id: Uuid,
    pub posting_kind: String,
    pub business_date: NaiveDate,
    pub original_batch_id: Option<Uuid>,
    pub posted_at: String,
    pub delta_minor: String,
    pub journal_fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasisSummary {
    pub format_version: u8,
    pub source: String,
    pub activation: String,
    pub currency: Currency,
    pub id: Uuid,
    pub policy: PeriodicPolicy,
    pub policy_digest: String,
    pub account_id: Uuid,
    pub product_version_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub due_at: String,
    pub core_as_of: String,
    pub captured_person_id: Uuid,
    pub account_registered_at: String,
    pub account_state: String,
    pub debit_blocked: bool,
    pub credit_blocked: bool,
    pub current_booked: String,
    pub current_held: String,
    pub current_available: String,
    pub period_status: String,
    pub opening_booked: String,
    pub period_change: String,
    pub observed_closing_booked: String,
    pub source_count: u32,
    pub source_digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasisCalculation {
    pub amount: Money,
    pub exact_numerator: String,
    pub exact_denominator: String,
    pub is_due: bool,
    pub evaluated_at: String,
    pub basis_source: String,
    pub provisional: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasisDetail {
    pub basis: BasisSummary,
    pub digest: String,
    pub calculation: Option<BasisCalculation>,
    pub calculation_reason: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BasisPage {
    pub basis_id: Uuid,
    pub digest: String,
    pub items: Vec<BasisItem>,
    pub next_after: Option<u32>,
}
pub struct BasisArchive {
    pub detail: BasisDetail,
    pub items: Vec<BasisItem>,
    pub actor: ActorContext,
}
#[derive(Clone)]
pub struct BasisRecords {
    pool: PgPool,
}
impl Core {
    pub fn periodic_bases(&self) -> BasisRecords {
        BasisRecords {
            pool: self.pool.clone(),
        }
    }
}
fn instant(s: &str) -> Result<DateTime<Utc>> {
    s.parse::<DateTime<Utc>>().map_err(|_| BasisConflict.into())
}
fn wide(s: &str) -> Result<i128> {
    let value = s.parse::<i128>().map_err(|_| BasisConflict)?;
    ensure!(value.to_string() == s, BasisConflict);
    Ok(value)
}
impl BasisSummary {
    fn validate(&self, items: &[BasisItem]) -> Result<()> {
        self.policy.validate()?;
        let as_of = instant(&self.core_as_of)?;
        ensure!(
            self.format_version == 1
                && self.source == "CORE_BUSINESS_DATE_BASIS_V1"
                && self.activation == "PREVIEW_ONLY"
                && !self.id.is_nil()
                && !self.account_id.is_nil()
                && !self.captured_person_id.is_nil()
                && self.policy.product_version_id == self.product_version_id
                && fingerprint(&self.policy)? == self.policy_digest
                && self.period_start >= self.policy.effective_from
                && self.period_start <= as_of.date_naive()
                && self.period_end == self.policy.period_end(self.period_start)?
                && self.period_end <= self.policy.effective_until
                && instant(&self.due_at)? == self.policy.calendar.due(self.period_end)?
                && self.source_count as usize == items.len()
                && items.len() <= 1000
                && fingerprint(&items)? == self.source_digest
                && ["PROVISIONING", "ACTIVE", "CLOSING", "CLOSED"]
                    .contains(&self.account_state.as_str())
                && self.period_status
                    == if self.period_end <= as_of.date_naive() {
                        "CLOSED_PERIOD_OBSERVATION"
                    } else {
                        "OPEN_PERIOD_PROVISIONAL"
                    },
            BasisConflict
        );
        instant(&self.account_registered_at)?;
        let booked = self.current_booked.parse::<i64>()?;
        let held = self.current_held.parse::<i64>()?;
        ensure!(
            booked >= held
                && held >= 0
                && booked.to_string() == self.current_booked
                && held.to_string() == self.current_held
                && (i128::from(booked) - i128::from(held)).to_string() == self.current_available,
            BasisConflict
        );
        let mut opening = 0_i128;
        let mut change = 0_i128;
        let mut ids = BTreeSet::new();
        let mut prior = None;
        for (index, item) in items.iter().enumerate() {
            let delta = item.delta_minor.parse::<i64>()?;
            let posted = instant(&item.posted_at)?;
            let order = (item.business_date, posted, item.batch_id);
            ensure!(
                item.ordinal == index as u32 + 1
                    && !item.batch_id.is_nil()
                    && !item.operation_id.is_nil()
                    && ids.insert(item.batch_id)
                    && delta != 0
                    && delta.to_string() == item.delta_minor
                    && item.business_date < self.period_end
                    && prior.is_none_or(|p| p < order)
                    && item.journal_fingerprint.len() == 64,
                BasisConflict
            );
            prior = Some(order);
            if item.business_date < self.period_start {
                opening = opening
                    .checked_add(i128::from(delta))
                    .ok_or(BasisConflict)?
            } else {
                change = change.checked_add(i128::from(delta)).ok_or(BasisConflict)?
            }
        }
        ensure!(
            wide(&self.opening_booked)? == opening
                && wide(&self.period_change)? == change
                && wide(&self.observed_closing_booked)?
                    == opening.checked_add(change).ok_or(BasisConflict)?,
            BasisConflict
        );
        Ok(())
    }
    fn calculation(&self) -> Result<(Option<BasisCalculation>, Option<String>)> {
        if instant(&self.account_registered_at)?.date_naive() > self.period_start {
            return Ok((None, Some("ACCOUNT_REGISTERED_AFTER_PERIOD_START".into())));
        }
        let balance = wide(&self.observed_closing_booked)?;
        let closing_booked = match self.policy.calculation {
            Calculation::MonthlyFixedFee { .. } => None,
            Calculation::DailyCreditAccrual { .. } => {
                if balance < 0 {
                    return Ok((None, Some("NEGATIVE_CLOSING_BOOKED".into())));
                }
                let Ok(value) = i64::try_from(balance) else {
                    return Ok((None, Some("CLOSING_BOOKED_OUT_OF_I64_RANGE".into())));
                };
                Some(Money::new(value, self.currency))
            }
        };
        let preview = self.policy.preview(&PreviewInput {
            period_start: self.period_start,
            closing_booked,
            evaluated_at: instant(&self.core_as_of)?,
        })?;
        Ok((
            Some(BasisCalculation {
                amount: preview.amount,
                exact_numerator: preview.exact_numerator,
                exact_denominator: preview.exact_denominator,
                is_due: preview.is_due,
                evaluated_at: self.core_as_of.clone(),
                basis_source: self.source.clone(),
                provisional: self.period_status == "OPEN_PERIOD_PROVISIONAL",
            }),
            None,
        ))
    }
}
impl BasisRecords {
    pub async fn capture(
        &self,
        controls: &Entitlements,
        actor: &ActorContext,
        key: Uuid,
        input: &BasisInput,
    ) -> Result<BasisArchive> {
        ensure!(
            controls.finance_authorized(actor, true).await?,
            "Current Finance capture authority required"
        );
        ensure!(
            !key.is_nil() && !input.policy_id.is_nil() && !input.account_id.is_nil(),
            BasisInputError
        );
        let person = controls.person(actor).await?.0;
        let policy = Core::new(self.pool.clone())
            .periodic_policies()
            .get(input.policy_id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        policy.policy.validate()?;
        let id: Uuid =
            match sqlx::query_scalar("SELECT core.capture_periodic_basis($1,$2,$3,$4,$5,$6)")
                .bind(input.policy_id)
                .bind(input.account_id)
                .bind(input.period_start)
                .bind(key)
                .bind(serde_json::to_value(actor)?)
                .bind(person)
                .fetch_one(&self.pool)
                .await
            {
                Ok(id) => id,
                Err(sqlx::Error::Database(d)) if d.code().as_deref() == Some("P5101") => {
                    return Err(BasisLimit.into());
                }
                Err(e) => return Err(e.into()),
            };
        self.get(id).await
    }
    pub async fn get(&self, id: Uuid) -> Result<BasisArchive> {
        let row = sqlx::query("SELECT * FROM core.periodic_bases WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let raw: serde_json::Value = row.get("document");
        let digest: String = row.get("digest");
        ensure!(fingerprint(&raw)? == digest, BasisConflict);
        let mut summary = raw.clone();
        let source = summary
            .as_object_mut()
            .ok_or(BasisConflict)?
            .remove("items")
            .ok_or(BasisConflict)?;
        let basis: BasisSummary = serde_json::from_value(summary.clone())?;
        let items: Vec<BasisItem> = serde_json::from_value(source.clone())?;
        ensure!(
            fingerprint(&summary)? == fingerprint(&basis)?
                && fingerprint(&source)? == fingerprint(&items)?,
            BasisConflict
        );
        basis.validate(&items)?;
        ensure!(
            basis.id == id
                && basis.policy.id == row.get::<Uuid, _>("policy_id")
                && basis.account_id == row.get::<Uuid, _>("account_id")
                && basis.period_start == row.get::<NaiveDate, _>("period_start")
                && basis.captured_person_id == row.get::<Uuid, _>("person_id")
                && instant(&basis.core_as_of)? == row.get::<DateTime<Utc>, _>("captured_at"),
            BasisConflict
        );
        let native_policy = Core::new(self.pool.clone())
            .periodic_policies()
            .get(basis.policy.id)
            .await?
            .ok_or(BasisConflict)?;
        ensure!(native_policy.digest == basis.policy_digest, BasisConflict);
        let ids = items.iter().map(|i| i.batch_id).collect::<Vec<_>>();
        let facts:Vec<serde_json::Value>=sqlx::query_scalar("SELECT jsonb_build_object('ordinal',row_number() OVER(ORDER BY j.business_date,j.created_at,j.id),'batchId',j.id,'operationId',j.operation_id,'postingKind',j.posting_kind,'businessDate',j.business_date,'originalBatchId',j.original_batch,'postedAt',to_char(j.created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),'deltaMinor',e.delta::text,'journalFingerprint',j.fingerprint) FROM core.journal_entries e JOIN core.journal_batches j ON j.id=e.batch_id WHERE e.account_id=$1 AND j.id=ANY($2) ORDER BY j.business_date,j.created_at,j.id").bind(basis.account_id).bind(&ids).fetch_all(&self.pool).await?;
        ensure!(fingerprint(&facts)? == basis.source_digest, BasisConflict);
        let mut actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
        ensure!(
            actor.trust == TrustContext::Workforce
                && actor.issuer == row.get::<String, _>("issuer")
                && actor.subject == row.get::<String, _>("subject")
                && actor.client_id == row.get::<String, _>("client_id"),
            BasisConflict
        );
        actor.person_id = Some(PartyId(basis.captured_person_id));
        let (calculation, calculation_reason) = basis.calculation()?;
        Ok(BasisArchive {
            detail: BasisDetail {
                basis,
                digest,
                calculation,
                calculation_reason,
            },
            items,
            actor,
        })
    }
    pub async fn ids(&self, policy: Option<Uuid>, account: Option<Uuid>) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar("SELECT id FROM core.periodic_bases WHERE ($1::uuid IS NULL OR policy_id=$1) AND ($2::uuid IS NULL OR account_id=$2) ORDER BY captured_at DESC,id LIMIT 100").bind(policy).bind(account).fetch_all(&self.pool).await?)
    }
    pub async fn audit_ids(&self, after: Uuid) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM core.periodic_bases WHERE id>$1 ORDER BY id LIMIT 50",
        )
        .bind(after)
        .fetch_all(&self.pool)
        .await?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> BasisSummary {
        let policy = super::super::tests::policy();
        BasisSummary {
            format_version: 1,
            source: "CORE_BUSINESS_DATE_BASIS_V1".into(),
            activation: "PREVIEW_ONLY".into(),
            currency: Currency::synthetic(),
            id: Uuid::new_v4(),
            policy_digest: fingerprint(&policy).unwrap(),
            product_version_id: policy.product_version_id,
            policy,
            account_id: Uuid::new_v4(),
            period_start: "2028-02-29".parse().unwrap(),
            period_end: "2028-03-01".parse().unwrap(),
            due_at: "2028-03-02T17:00:00.000000Z".into(),
            core_as_of: "2028-03-02T17:00:00.000000Z".into(),
            captured_person_id: Uuid::new_v4(),
            account_registered_at: "2028-02-01T00:00:00.000000Z".into(),
            account_state: "ACTIVE".into(),
            debit_blocked: false,
            credit_blocked: false,
            current_booked: "5000".into(),
            current_held: "0".into(),
            current_available: "5000".into(),
            period_status: "CLOSED_PERIOD_OBSERVATION".into(),
            opening_booked: "0".into(),
            period_change: "5000".into(),
            observed_closing_booked: "5000".into(),
            source_count: 0,
            source_digest: fingerprint(&Vec::<BasisItem>::new()).unwrap(),
        }
    }
    #[test]
    fn negative_and_widened_basis_never_clamp_to_a_chargeable_i64_value() {
        let mut basis = sample();
        basis.observed_closing_booked = "-1".into();
        let (value, reason) = basis.calculation().unwrap();
        assert!(value.is_none());
        assert_eq!(reason.as_deref(), Some("NEGATIVE_CLOSING_BOOKED"));
        basis.observed_closing_booked = (i128::from(i64::MAX) + 1).to_string();
        let (value, reason) = basis.calculation().unwrap();
        assert!(value.is_none());
        assert_eq!(reason.as_deref(), Some("CLOSING_BOOKED_OUT_OF_I64_RANGE"));
        basis.observed_closing_booked = "05000".into();
        assert!(basis.calculation().is_err());
    }
    #[test]
    fn saved_clock_and_native_source_are_preserved_in_calculation() {
        let mut basis = sample();
        let (value, reason) = basis.calculation().unwrap();
        assert!(reason.is_none());
        let value = value.unwrap();
        assert_eq!(value.amount.minor_units(), 1);
        assert!(value.is_due);
        assert_eq!(value.evaluated_at, basis.core_as_of);
        assert_eq!(value.basis_source, "CORE_BUSINESS_DATE_BASIS_V1");
        basis.core_as_of = "2028-03-02T16:59:59.999999Z".into();
        assert!(!basis.calculation().unwrap().0.unwrap().is_due);
        basis.account_registered_at = "2028-03-01T00:00:00.000000Z".into();
        assert_eq!(
            basis.calculation().unwrap().1.as_deref(),
            Some("ACCOUNT_REGISTERED_AFTER_PERIOD_START")
        );
    }
}
