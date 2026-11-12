//! Independent synthetic cash evidence. Ledger stocks are never substituted for a cash feed.
use crate::{Finance, client::GlClient};
use anyhow::{Result, ensure};
use bank_core::finance_snapshot::PositionSnapshot;
use bank_platform::{
    authenticity::SignedDocument,
    domain::{ActorContext, Currency, fingerprint},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub const LIQUIDITY_SCOPE: &str = "SYNTHETIC_TST_CLEARING";
pub fn liquidity_key() -> String {
    std::env::var("BANK_LIQUIDITY_SIGNING_KEY")
        .unwrap_or_else(|_| "synthetic-local-liquidity-authentication-key-2026".into())
}
pub fn unsigned(s: &str) -> Result<i128> {
    let n = s.parse::<i128>()?;
    ensure!(
        n >= 0 && s == n.to_string(),
        "Canonical nonnegative aggregate minor units required"
    );
    Ok(n)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiquidityRequest {
    pub request_id: Uuid,
    pub scope: String,
    pub currency: Currency,
}
impl LiquidityRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.request_id.is_nil()
                && self.scope == LIQUIDITY_SCOPE
                && self.currency == Currency::synthetic(),
            "Exact synthetic liquidity scope required"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiquidityEvidence {
    pub evidence_id: Uuid,
    pub provider: String,
    pub scope: String,
    pub currency: Currency,
    pub version: i64,
    pub as_of: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub settled: String,
    pub available: String,
    pub reserved: String,
    pub expected_incoming: String,
    pub expected_outgoing: String,
}
impl LiquidityEvidence {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.evidence_id.is_nil()
                && self.provider == "SYNTHETIC_LIQUIDITY"
                && self.scope == LIQUIDITY_SCOPE
                && self.currency == Currency::synthetic()
                && self.version > 0,
            "Liquidity evidence identity/scope mismatch"
        );
        ensure!(
            self.as_of <= Utc::now() + chrono::Duration::seconds(5)
                && self.valid_until > self.as_of
                && self.valid_until <= self.as_of + chrono::Duration::seconds(60),
            "Bounded liquidity evidence lifetime required"
        );
        let a = unsigned(&self.available)?;
        let r = unsigned(&self.reserved)?;
        ensure!(
            a.checked_add(r) == Some(unsigned(&self.settled)?),
            "Liquidity cash controls do not balance"
        );
        unsigned(&self.expected_incoming)?;
        unsigned(&self.expected_outgoing)?;
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiquidityReply {
    pub request: LiquidityRequest,
    pub position: LiquidityEvidence,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiquiditySnapshot {
    pub observed_at: DateTime<Utc>,
    pub status: String,
    pub scope: String,
    pub currency: Currency,
    pub core_as_of: DateTime<Utc>,
    pub core_reserved_principal: String,
    pub core_submitted_principal: String,
    pub provider_as_of: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub evidence_id: Option<Uuid>,
    pub evidence_digest: Option<String>,
    pub provider_version: Option<i64>,
    pub available: Option<String>,
    pub provider_available: Option<String>,
    pub provider_reserved: Option<String>,
    pub externally_settled: Option<String>,
    pub expected_incoming: Option<String>,
    pub expected_outgoing: Option<String>,
    pub reason_codes: Vec<String>,
    pub availability_rule: String,
}
pub(crate) struct Observation {
    pub view: LiquiditySnapshot,
    pub packet: Option<serde_json::Value>,
}
impl GlClient {
    async fn liquidity_packet(&self, request: &LiquidityRequest) -> Result<Option<SignedDocument>> {
        let p = SignedDocument::sign(liquidity_key().as_bytes(), request)?;
        let mut r = self
            .http
            .post(format!("{}/liquidity/inquiry", self.base))
            .json(&p)
            .send()
            .await?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(r.status().is_success(), "Liquidity provider unavailable");
        let mut bytes = vec![];
        while let Some(chunk) = r.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 65536,
                "Liquidity reply bound exceeded"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(serde_json::from_slice(&bytes)?))
    }
}
impl Finance {
    pub(crate) async fn observe_liquidity(
        &self,
        positions: &PositionSnapshot,
    ) -> Result<Observation> {
        let request = LiquidityRequest {
            request_id: Uuid::new_v4(),
            scope: LIQUIDITY_SCOPE.into(),
            currency: Currency::synthetic(),
        };
        let mut view = LiquiditySnapshot {
            observed_at: Utc::now(),
            status: "UNAVAILABLE".into(),
            scope: LIQUIDITY_SCOPE.into(),
            currency: Currency::synthetic(),
            core_as_of: positions.as_of,
            core_reserved_principal: positions.reserved_outgoing_principal.clone(),
            core_submitted_principal: positions.submitted_outgoing_principal.clone(),
            provider_as_of: None,
            valid_until: None,
            evidence_id: None,
            evidence_digest: None,
            provider_version: None,
            available: None,
            provider_available: None,
            provider_reserved: None,
            externally_settled: None,
            expected_incoming: None,
            expected_outgoing: None,
            reason_codes: vec![],
            availability_rule: "MAX_ZERO_PROVIDER_AVAILABLE_MINUS_ALL_CORE_OUTGOING_COMMITMENTS"
                .into(),
        };
        let mut packet = None;
        match self.client.liquidity_packet(&request).await {
            Ok(Some(p)) => {
                packet = Some(serde_json::to_value(&p)?);
                let verified = (|| -> Result<LiquidityEvidence> {
                    p.verify(liquidity_key().as_bytes())?;
                    let r: LiquidityReply = serde_json::from_value(p.body)?;
                    r.request.validate()?;
                    ensure!(
                        fingerprint(&r.request)? == fingerprint(&request)?,
                        "Liquidity reply request binding mismatch"
                    );
                    r.position.validate()?;
                    Ok(r.position)
                })();
                match verified {
                    Ok(e) => {
                        view.observed_at = Utc::now();
                        view.provider_as_of = Some(e.as_of);
                        view.valid_until = Some(e.valid_until);
                        view.evidence_id = Some(e.evidence_id);
                        view.evidence_digest = Some(fingerprint(&e)?);
                        view.provider_version = Some(e.version);
                        if e.valid_until <= view.observed_at {
                            view.status = "STALE".into();
                            view.reason_codes.push("PROVIDER_EVIDENCE_EXPIRED".into());
                        } else {
                            view.status = "FRESH".into();
                            view.available = Some(
                                (unsigned(&e.available)?
                                    - unsigned(&positions.reserved_outgoing_principal)?)
                                .max(0)
                                .to_string(),
                            );
                            view.provider_available = Some(e.available);
                            view.provider_reserved = Some(e.reserved);
                            view.externally_settled = Some(e.settled);
                            view.expected_incoming = Some(e.expected_incoming);
                            view.expected_outgoing = Some(e.expected_outgoing);
                        }
                    }
                    Err(_) => {
                        view.status = "INVALID".into();
                        view.reason_codes
                            .push("PROVIDER_AUTHENTICATION_OR_SCOPE_INVALID".into());
                    }
                }
            }
            Ok(None) => {
                view.status = "MISSING".into();
                view.reason_codes.push("PROVIDER_EVIDENCE_MISSING".into());
            }
            Err(_) => {
                view.reason_codes
                    .push("PROVIDER_TRANSPORT_UNAVAILABLE".into());
            }
        }
        if (Utc::now() - positions.as_of).num_seconds() > 5 {
            view.status = "STALE".into();
            view.available = None;
            view.reason_codes.push("CORE_POSITIONS_STALE".into());
        }
        Ok(Observation { view, packet })
    }
    pub async fn staff_liquidity(&self, actor: &ActorContext) -> Result<LiquiditySnapshot> {
        self.authorize_staff(actor, false).await?;
        let positions = self.core.positions().await?;
        Ok(self.observe_liquidity(&positions).await?.view)
    }
}
