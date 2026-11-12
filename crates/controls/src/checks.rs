//! Distinct, expiring control authorities. A positive decision never substitutes for another check.
use crate::audit::{AuditLog, AuditRecord};
use anyhow::{Result, ensure};
use bank_platform::{
    authenticity::SignedDocument,
    database::Module,
    domain::{ActorContext, EventEnvelope, EventId, TrustContext, fingerprint},
    events::EventStore,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckKind {
    Identity,
    Sanctions,
    Risk,
    Fraud,
}
impl CheckKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Identity => "IDENTITY",
            Self::Sanctions => "SANCTIONS",
            Self::Risk => "RISK",
            Self::Fraud => "FRAUD",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    Allow,
    Deny,
    Review,
    Pending,
}
impl Outcome {
    fn code(self) -> &'static str {
        match self {
            Self::Allow => "ALLOW",
            Self::Deny => "DENY",
            Self::Review => "REVIEW",
            Self::Pending => "PENDING",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckRequest {
    pub id: Uuid,
    pub resource_id: Uuid,
    pub kind: CheckKind,
    pub input_digest: String,
    pub policy_version: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckResult {
    pub request: CheckRequest,
    pub outcome: Outcome,
    pub evidence_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub reason_code: String,
}
#[derive(Clone, Copy)]
pub enum Purpose {
    Onboarding,
    Payment,
}
impl Purpose {
    pub fn required(self) -> &'static [CheckKind] {
        match self {
            Self::Onboarding => &[CheckKind::Identity, CheckKind::Sanctions, CheckKind::Risk],
            Self::Payment => &[CheckKind::Sanctions, CheckKind::Risk, CheckKind::Fraud],
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateSummary {
    pub outcome: Outcome,
    pub revision: String,
    pub valid_until: DateTime<Utc>,
    pub check_ids: Vec<Uuid>,
}
#[derive(Clone)]
pub struct Checks {
    pub pool: PgPool,
    client: reqwest::Client,
    base: String,
    key: String,
}
pub const POLICY: &str = "synthetic-controls-v1";
pub fn simulator_key() -> String {
    std::env::var("BANK_SIMULATOR_KEY")
        .unwrap_or_else(|_| "synthetic-provider-authentication-key-only-2026".into())
}
impl Checks {
    pub fn new(pool: PgPool) -> Result<Self> {
        let base =
            std::env::var("BANK_SIMULATOR_URL").unwrap_or_else(|_| "http://127.0.0.1:4182".into());
        let value = Self {
            pool,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(1500))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            base: String::new(),
            key: simulator_key(),
        };
        ensure!(value.key.len() >= 32, "Simulator key too short");
        value.with_provider_base(base)
    }
    pub fn with_provider_base(mut self, base: String) -> Result<Self> {
        let url = reqwest::Url::parse(&base)?;
        ensure!(
            url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
            "Synthetic provider endpoint must be loopback"
        );
        self.base = base;
        Ok(self)
    }
    pub async fn request(
        &self,
        actor: &ActorContext,
        resource: Uuid,
        kind: CheckKind,
        input_digest: &str,
    ) -> Result<CheckRequest> {
        ensure!(input_digest.len() == 64, "Control input digest required");
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,41))")
            .bind(format!("{resource}:{}", kind.code()))
            .execute(&mut *tx)
            .await?;
        let old=sqlx::query("SELECT r.*,e.expires_at FROM controls.check_heads h JOIN controls.check_requests r ON r.id=h.request_id LEFT JOIN controls.check_results e ON e.request_id=r.id WHERE h.resource_id=$1 AND h.kind=$2").bind(resource).bind(kind.code()).fetch_optional(&mut *tx).await?;
        let customer = if actor.trust == TrustContext::Customer {
            (Some(actor.issuer.clone()), Some(actor.subject.clone()))
        } else {
            old.as_ref()
                .map(|r| {
                    (
                        r.get::<Option<String>, _>("customer_issuer"),
                        r.get::<Option<String>, _>("customer_subject"),
                    )
                })
                .unwrap_or((None, None))
        };
        let generation = old
            .as_ref()
            .map(|r| r.get::<i32, _>("generation") + 1)
            .unwrap_or(1);
        if let Some(row) = old {
            let until: Option<DateTime<Utc>> = row.get("expires_at");
            if row.get::<String, _>("input_digest") == input_digest
                && row.get::<String, _>("policy_version") == POLICY
                && until.unwrap_or(row.get("deadline")) > Utc::now()
            {
                let request = CheckRequest {
                    id: row.get("id"),
                    resource_id: resource,
                    kind,
                    input_digest: input_digest.into(),
                    policy_version: POLICY.into(),
                };
                tx.commit().await?;
                return Ok(request);
            }
        }
        let request = CheckRequest {
            id: Uuid::new_v4(),
            resource_id: resource,
            kind,
            input_digest: input_digest.into(),
            policy_version: POLICY.into(),
        };
        sqlx::query("INSERT INTO controls.check_requests(id,resource_id,kind,input_digest,policy_version,generation,customer_issuer,customer_subject) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(request.id).bind(resource).bind(kind.code()).bind(input_digest).bind(POLICY).bind(generation).bind(customer.0).bind(customer.1).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO controls.check_heads VALUES($1,$2,$3) ON CONFLICT(resource_id,kind) DO UPDATE SET request_id=EXCLUDED.request_id").bind(resource).bind(kind.code()).bind(request.id).execute(&mut *tx).await?;
        self.fact(&mut tx, request.id, actor).await?;
        tx.commit().await?;
        Ok(request)
    }
    pub async fn poll_due(&self) -> Result<bool> {
        // A short durable claim bounds repeated provider work. Stable provider request IDs
        // and receipt deduplication handle a crash after the call or concurrent recovery.
        let id:Option<Uuid>=sqlx::query_scalar("WITH candidate AS (SELECT r.id FROM controls.check_requests r JOIN controls.check_heads h ON h.request_id=r.id WHERE r.state='PENDING' AND r.attempts<8 AND r.deadline>now() AND r.next_poll_at<=now() ORDER BY r.next_poll_at LIMIT 1 FOR UPDATE OF r SKIP LOCKED) UPDATE controls.check_requests r SET next_poll_at=now()+interval '5 seconds',attempts=attempts+1 FROM candidate WHERE r.id=candidate.id RETURNING r.id").fetch_optional(&self.pool).await?;
        if let Some(id) = id {
            self.poll(id).await?;
            return Ok(true);
        }
        sqlx::query("UPDATE controls.check_requests SET next_action='MANUAL_REVIEW' WHERE state='PENDING' AND (attempts>=8 OR deadline<=now()) AND next_action<>'MANUAL_REVIEW'").execute(&self.pool).await?;
        Ok(false)
    }
    pub async fn rescreen_due(&self) -> Result<bool> {
        let row=sqlx::query("SELECT r.resource_id,r.kind,r.input_digest FROM controls.check_heads h JOIN controls.check_requests r ON r.id=h.request_id JOIN controls.check_results e ON e.request_id=r.id WHERE e.expires_at<=now() ORDER BY e.expires_at LIMIT 1").fetch_optional(&self.pool).await?;
        if let Some(row) = row {
            let kind = serde_json::from_value(serde_json::json!(row.get::<String, _>("kind")))?;
            self.request(
                &provider_actor(),
                row.get("resource_id"),
                kind,
                &row.get::<String, _>("input_digest"),
            )
            .await?;
            return Ok(true);
        }
        Ok(false)
    }
    pub async fn poll(&self, id: Uuid) -> Result<Outcome> {
        let row = sqlx::query("SELECT * FROM controls.check_requests WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let kind: CheckKind =
            serde_json::from_value(serde_json::json!(row.get::<String, _>("kind")))?;
        let request = CheckRequest {
            id,
            resource_id: row.get("resource_id"),
            kind,
            input_digest: row.get("input_digest"),
            policy_version: row.get("policy_version"),
        };
        // No transaction is held during provider calls, including response-loss faults.
        let response = self
            .client
            .post(format!("{}/checks", self.base))
            .header("X-Simulator-Key", &self.key)
            .json(&request)
            .send()
            .await;
        let packet = match response {
            Ok(mut r) if r.status().is_success() => {
                let mut bytes = Vec::new();
                let mut valid = true;
                loop {
                    match r.chunk().await {
                        Ok(Some(chunk)) if bytes.len() + chunk.len() <= 65536 => {
                            bytes.extend_from_slice(&chunk)
                        }
                        Ok(None) => break,
                        _ => {
                            valid = false;
                            break;
                        }
                    }
                }
                if valid {
                    serde_json::from_slice::<SignedDocument>(&bytes).ok()
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(packet) = packet else {
            sqlx::query("INSERT INTO controls.check_attempts(id,request_id,outcome) VALUES($1,$2,'PROVIDER_PENDING')").bind(Uuid::new_v4()).bind(id).execute(&self.pool).await?;
            return Ok(Outcome::Pending);
        };
        self.record(&packet).await
    }
    pub async fn record(&self, packet: &SignedDocument) -> Result<Outcome> {
        packet.verify(self.key.as_bytes())?;
        let result: CheckResult = serde_json::from_value(packet.body.clone())?;
        ensure!(
            !result.reason_code.is_empty()
                && result.reason_code.len() <= 64
                && result
                    .reason_code
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c == b'_'),
            "Invalid control reason code"
        );
        let digest = fingerprint(&result)?;
        let request = &result.request;
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("SELECT r.*,h.request_id AS current_id FROM controls.check_requests r JOIN controls.check_heads h ON h.resource_id=r.resource_id AND h.kind=r.kind WHERE r.id=$1 FOR UPDATE OF r").bind(request.id).fetch_one(&mut *tx).await?;
        let reason = if row.get::<Uuid, _>("current_id") != request.id {
            "SUPERSEDED_REQUEST"
        } else if row.get::<Uuid, _>("resource_id") != request.resource_id
            || row.get::<String, _>("kind") != request.kind.code()
            || row.get::<String, _>("input_digest") != request.input_digest
            || row.get::<String, _>("policy_version") != request.policy_version
        {
            "REQUEST_SCOPE_MISMATCH"
        } else if result.expires_at <= Utc::now()
            || result.expires_at > Utc::now() + chrono::Duration::days(1)
        {
            "EXPIRED_OR_INVALID_EVIDENCE"
        } else {
            ""
        };
        if !reason.is_empty() {
            sqlx::query("INSERT INTO controls.check_rejections(request_id,reason_code,digest) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(request.id).bind(reason).bind(&digest).execute(&mut *tx).await?;
            tx.commit().await?;
            anyhow::bail!("Control evidence rejected: {reason}");
        }
        if result.outcome == Outcome::Pending {
            tx.commit().await?;
            return Ok(Outcome::Pending);
        }
        let prior: Option<String> = sqlx::query_scalar(
            "SELECT fingerprint FROM controls.check_results WHERE request_id=$1",
        )
        .bind(request.id)
        .fetch_optional(&mut *tx)
        .await?;
        let actor = provider_actor();
        if let Some(prior) = prior {
            if prior != digest {
                sqlx::query("INSERT INTO controls.check_rejections(request_id,reason_code,digest) VALUES($1,'CONFLICTING_RESULT',$2) ON CONFLICT DO NOTHING").bind(request.id).bind(digest).execute(&mut *tx).await?;
                let changed=sqlx::query("UPDATE controls.check_requests SET state='REVIEW',revision=revision+1 WHERE id=$1 AND state<>'REVIEW'").bind(request.id).execute(&mut *tx).await?;
                if changed.rows_affected() == 1 {
                    self.fact(&mut tx, request.id, &actor).await?;
                }
                tx.commit().await?;
                anyhow::bail!("Conflicting control evidence requires review");
            }
            let state: String = row.get("state");
            tx.commit().await?;
            return Ok(if state == "REVIEW" {
                Outcome::Review
            } else {
                result.outcome
            });
        }
        sqlx::query("INSERT INTO controls.check_results(request_id,fingerprint,document,outcome,evidence_id,expires_at) VALUES($1,$2,$3,$4,$5,$6)").bind(request.id).bind(digest).bind(serde_json::to_value(&result)?).bind(result.outcome.code()).bind(result.evidence_id).bind(result.expires_at).execute(&mut *tx).await?;
        sqlx::query("UPDATE controls.check_requests SET state=$2,revision=revision+1 WHERE id=$1")
            .bind(request.id)
            .bind(result.outcome.code())
            .execute(&mut *tx)
            .await?;
        self.fact(&mut tx, request.id, &actor).await?;
        tx.commit().await?;
        Ok(result.outcome)
    }
    pub async fn evaluate(
        &self,
        resource: Uuid,
        purpose: Purpose,
        input_digest: &str,
    ) -> Result<GateSummary> {
        let mut state = Outcome::Allow;
        let mut until = Utc::now() + chrono::Duration::days(1);
        let mut checks = vec![];
        let mut versions = vec![];
        for kind in purpose.required() {
            let row=sqlx::query("SELECT r.id,r.input_digest,r.policy_version,r.state,r.revision,e.expires_at,EXISTS(SELECT 1 FROM controls.check_reviews v WHERE v.request_id=r.id AND v.result_digest=e.fingerprint AND v.expires_at>now() AND v.decision='ALLOW') AS reviewed FROM controls.check_heads h JOIN controls.check_requests r ON r.id=h.request_id LEFT JOIN controls.check_results e ON e.request_id=r.id WHERE h.resource_id=$1 AND h.kind=$2").bind(resource).bind(kind.code()).fetch_optional(&self.pool).await?;
            let outcome = if let Some(row) = row {
                checks.push(row.get("id"));
                versions.push((row.get::<Uuid, _>("id"), row.get::<i64, _>("revision")));
                let expiry: Option<DateTime<Utc>> = row.get("expires_at");
                if let Some(expiry) = expiry {
                    until = until.min(expiry);
                }
                if row.get::<String, _>("input_digest") != input_digest
                    || row.get::<String, _>("policy_version") != POLICY
                    || expiry.is_none_or(|e| e <= Utc::now())
                {
                    Outcome::Pending
                } else {
                    match row.get::<String, _>("state").as_str() {
                        "ALLOW" => Outcome::Allow,
                        "DENY" | "MANUAL_DENY" => Outcome::Deny,
                        "MANUAL_ALLOW" if row.get::<bool, _>("reviewed") => Outcome::Allow,
                        "MANUAL_ALLOW" | "REVIEW" => Outcome::Review,
                        _ => Outcome::Pending,
                    }
                }
            } else {
                Outcome::Pending
            };
            state = match (state, outcome) {
                (Outcome::Deny, _) | (_, Outcome::Deny) => Outcome::Deny,
                (Outcome::Review, _) | (_, Outcome::Review) => Outcome::Review,
                (Outcome::Pending, _) | (_, Outcome::Pending) => Outcome::Pending,
                _ => Outcome::Allow,
            };
        }
        Ok(GateSummary {
            outcome: state,
            revision: fingerprint(&versions)?,
            valid_until: until,
            check_ids: checks,
        })
    }
    pub(crate) async fn fact(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        actor: &ActorContext,
    ) -> Result<()> {
        let row = sqlx::query(
            "SELECT resource_id,kind,state,revision FROM controls.check_requests WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
        let correlation: Uuid = row.get("resource_id");
        EventStore::new(self.pool.clone(),Module::Controls).stage(tx,&EventEnvelope{event_id:EventId::new(),event_type:"control.changed".into(),schema_version:1,producer:"controls".into(),aggregate_id:id,aggregate_version:row.get("revision"),occurred_at:Utc::now(),correlation_id:correlation,causation_id:id.to_string(),data:serde_json::json!({"requestId":id,"resourceId":correlation,"kind":row.get::<String,_>("kind"),"state":row.get::<String,_>("state")})}).await?;
        AuditLog::append_in(
            tx,
            Uuid::new_v4(),
            "controls",
            &AuditRecord::new(actor, "control.recorded", id, correlation),
        )
        .await?;
        Ok(())
    }
}
fn provider_actor() -> ActorContext {
    ActorContext {
        issuer: "urn:piggybank:simulator".into(),
        subject: "control-provider".into(),
        trust: TrustContext::Service,
        client_id: "provider-simulator".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
