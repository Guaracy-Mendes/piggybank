use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("unsupported currency or scale")]
    UnsupportedCurrency,
    #[error("minor units must be a canonical signed decimal integer string within i64 bounds")]
    InvalidAmount,
    #[error("amount must be positive")]
    NonPositive,
    #[error("monetary arithmetic overflow")]
    Overflow,
    #[error("currencies must match")]
    CurrencyMismatch,
    #[error("state transition lacks required authority or financial evidence")]
    InvalidTransition,
    #[error("invalid contract payload")]
    InvalidPayload,
}

macro_rules! typed_id {
    ($($name:ident),+ $(,)?) => { $(
        #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name { pub fn new() -> Self { Self(Uuid::new_v4()) } }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl std::fmt::Display for $name { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) } }
    )+ };
}
typed_id!(
    OperationId,
    PartyId,
    AccountId,
    PaymentId,
    ApplicationId,
    EventId,
    HoldId,
    JournalBatchId,
    CaseId
);

/// The local profile uses a synthetic unit, not a selected legal currency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CurrencyWire", into = "CurrencyWire")]
pub struct Currency {
    code: [u8; 3],
    scale: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CurrencyWire {
    code: String,
    scale: u8,
}

impl Currency {
    pub fn synthetic() -> Self {
        Self {
            code: *b"TST",
            scale: 2,
        }
    }
    pub fn code(&self) -> &str {
        std::str::from_utf8(&self.code).expect("currency is validated ASCII")
    }
    pub fn scale(&self) -> u8 {
        self.scale
    }
}
impl TryFrom<CurrencyWire> for Currency {
    type Error = DomainError;
    fn try_from(w: CurrencyWire) -> Result<Self, Self::Error> {
        if w.code == "TST" && w.scale == 2 {
            Ok(Self::synthetic())
        } else {
            Err(DomainError::UnsupportedCurrency)
        }
    }
}
impl From<Currency> for CurrencyWire {
    fn from(c: Currency) -> Self {
        Self {
            code: c.code().into(),
            scale: c.scale,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "MoneyWire", into = "MoneyWire")]
pub struct Money {
    minor_units: i64,
    currency: Currency,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoneyWire {
    minor_units: String,
    currency: Currency,
}

impl Money {
    pub fn new(minor_units: i64, currency: Currency) -> Self {
        Self {
            minor_units,
            currency,
        }
    }
    pub fn minor_units(&self) -> i64 {
        self.minor_units
    }
    pub fn currency(&self) -> Currency {
        self.currency
    }
    pub fn require_positive(self) -> Result<Self, DomainError> {
        if self.minor_units > 0 {
            Ok(self)
        } else {
            Err(DomainError::NonPositive)
        }
    }
    pub fn checked_add(self, other: Self) -> Result<Self, DomainError> {
        if self.currency != other.currency {
            return Err(DomainError::CurrencyMismatch);
        }
        Ok(Self::new(
            self.minor_units
                .checked_add(other.minor_units)
                .ok_or(DomainError::Overflow)?,
            self.currency,
        ))
    }
    pub fn checked_neg(self) -> Result<Self, DomainError> {
        Ok(Self::new(
            self.minor_units
                .checked_neg()
                .ok_or(DomainError::Overflow)?,
            self.currency,
        ))
    }
}
impl TryFrom<MoneyWire> for Money {
    type Error = DomainError;
    fn try_from(w: MoneyWire) -> Result<Self, Self::Error> {
        let number = w
            .minor_units
            .parse::<i64>()
            .map_err(|_| DomainError::InvalidAmount)?;
        if number.to_string() != w.minor_units {
            return Err(DomainError::InvalidAmount);
        }
        Ok(Self::new(number, w.currency))
    }
}
impl From<Money> for MoneyWire {
    fn from(m: Money) -> Self {
        Self {
            minor_units: m.minor_units.to_string(),
            currency: m.currency,
        }
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
pub struct FixedClock(pub DateTime<Utc>);
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrustContext {
    Customer,
    Workforce,
    Service,
    Partner,
}

/// Constructed from verified server-side identity, never trusted from a request body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorContext {
    pub issuer: String,
    pub subject: String,
    pub trust: TrustContext,
    pub client_id: String,
    pub person_id: Option<PartyId>,
    pub represented_party: Option<PartyId>,
    pub roles: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentDetails {
    pub payer_account_id: AccountId,
    pub beneficiary_id: String,
    pub amount: Money,
    pub fee: Money,
    pub requested_date: NaiveDate,
    pub rail: String,
    pub quote_id: String,
}

/// JSON object keys sort recursively. DTO validation precedes fingerprinting.
pub fn fingerprint<T: Serialize>(value: &T) -> Result<String, DomainError> {
    fn canonical(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(values) => {
                let sorted: std::collections::BTreeMap<_, _> =
                    values.into_iter().map(|(k, v)| (k, canonical(v))).collect();
                serde_json::Value::Object(sorted.into_iter().collect())
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(canonical).collect())
            }
            other => other,
        }
    }
    let value = canonical(serde_json::to_value(value).map_err(|_| DomainError::InvalidPayload)?);
    let bytes = serde_json::to_vec(&value).map_err(|_| DomainError::InvalidPayload)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventEnvelope {
    pub event_id: EventId,
    pub event_type: String,
    pub schema_version: u32,
    pub producer: String,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub occurred_at: DateTime<Utc>,
    pub correlation_id: Uuid,
    pub causation_id: String,
    pub data: serde_json::Value,
}
impl EventEnvelope {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version == 0
            || self.aggregate_version < 1
            || self.event_type.is_empty()
            || self.producer.is_empty()
            || self.causation_id.is_empty()
            || !self.data.is_object()
        {
            return Err(DomainError::InvalidPayload);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn money_is_exact_and_client_safe() {
        let m = Money::new(9_007_199_254_740_993, Currency::synthetic());
        let wire = serde_json::to_value(m).unwrap();
        assert_eq!(wire["minorUnits"], "9007199254740993");
        assert_eq!(serde_json::from_value::<Money>(wire).unwrap(), m);
        for bad in [
            json!(1.5),
            json!(25),
            json!("01"),
            json!("+1"),
            json!("-0"),
            json!("1.0"),
            json!("9223372036854775808"),
        ] {
            assert!(
                serde_json::from_value::<Money>(
                    json!({"minorUnits":bad,"currency":{"code":"TST","scale":2}})
                )
                .is_err()
            );
        }
        assert!(
            serde_json::from_value::<Money>(
                json!({"minorUnits":"1","currency":{"code":"USD","scale":2}})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<Currency>(json!({"code":"TST","scale":3})).is_err());
    }

    #[test]
    fn checked_money_and_positive_commands() {
        let currency = Currency::synthetic();
        assert_eq!(
            Money::new(i64::MAX, currency).checked_add(Money::new(1, currency)),
            Err(DomainError::Overflow)
        );
        assert_eq!(
            Money::new(i64::MIN, currency).checked_neg(),
            Err(DomainError::Overflow)
        );
        assert_eq!(
            Money::new(0, currency).require_positive(),
            Err(DomainError::NonPositive)
        );
        assert!(Money::new(1, currency).require_positive().is_ok());
    }

    #[test]
    fn fingerprint_is_stable_and_binds_material_details() {
        assert_eq!(
            fingerprint(&json!({"b":{"z":2,"a":1},"a":0})),
            fingerprint(&json!({"a":0,"b":{"a":1,"z":2}}))
        );
        let mut details = PaymentDetails {
            payer_account_id: AccountId::new(),
            beneficiary_id: "ben-1".into(),
            amount: Money::new(25, Currency::synthetic()),
            fee: Money::new(1, Currency::synthetic()),
            requested_date: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            rail: "SIMULATED".into(),
            quote_id: "quote-1".into(),
        };
        let original = fingerprint(&details).unwrap();
        details.beneficiary_id = "ben-2".into();
        assert_ne!(fingerprint(&details).unwrap(), original);
        details.beneficiary_id = "ben-1".into();
        details.amount = Money::new(26, Currency::synthetic());
        assert_ne!(fingerprint(&details).unwrap(), original);
    }

    #[test]
    fn event_versions_and_clock_are_explicit() {
        let fixed = FixedClock(
            DateTime::parse_from_rfc3339("2026-10-23T16:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let mut event = EventEnvelope {
            event_id: EventId::new(),
            event_type: "account.activated".into(),
            schema_version: 1,
            producer: "core".into(),
            aggregate_id: Uuid::new_v4(),
            aggregate_version: 1,
            occurred_at: fixed.now(),
            correlation_id: Uuid::new_v4(),
            causation_id: "operation-1".into(),
            data: json!({"accountId":AccountId::new()}),
        };
        assert!(event.validate().is_ok());
        event.aggregate_version = 0;
        assert_eq!(event.validate(), Err(DomainError::InvalidPayload));
        assert_eq!(fixed.now(), fixed.now());
    }
}
