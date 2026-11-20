//! WP-33 registered partner clients, customer consents, partner quotas and authenticated callbacks (FR-IAM-05,
//! FR-EXT-02). Grants live in the identity schema; Controls re-checks them through `identity.consent_permits` at
//! every decision, so revocation and client suspension apply to the very next partner call.
//!
//! A partner actor is built server-side from a verified client-credentials token plus an ACTIVE consent binding.
//! It carries the consenting customer's issuer and subject, so domain ownership checks still apply, together with
//! the partner `client_id` and `PARTNER` trust, so every audit record names both.
use crate::seal::KeyRing;
use anyhow::{Result, ensure};
use bank_platform::domain::{ActorContext, PartyId, TrustContext};
use ring::hmac;
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

pub const SCOPE_ACCOUNTS: &str = "accounts:read";
pub const SCOPE_PAYMENTS: &str = "payments:initiate";

/// A consent binding read at request time; `None` once the grant is revoked, expired or the client suspended.
#[derive(Clone, Debug)]
pub struct Binding {
    pub consent_id: Uuid,
    pub person: PartyId,
    pub issuer: String,
    pub subject: String,
    pub scopes: Vec<String>,
    pub account_ids: Vec<Uuid>,
}
impl Binding {
    /// The server-built actor for a consented partner call. `roles` carries `consent:<id>` plus granted scopes;
    /// Controls never trusts them alone and re-checks the grant in the database.
    pub fn actor(&self, client_id: &str) -> ActorContext {
        let mut roles = vec![format!("consent:{}", self.consent_id)];
        roles.extend(self.scopes.iter().cloned());
        ActorContext {
            issuer: self.issuer.clone(),
            subject: self.subject.clone(),
            trust: TrustContext::Partner,
            client_id: client_id.to_owned(),
            person_id: Some(self.person),
            represented_party: Some(self.person),
            roles,
        }
    }
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

#[derive(Clone)]
pub struct Partners {
    pub pool: PgPool,
}
fn person_json(actor: &ActorContext) -> Result<Value> {
    Ok(serde_json::to_value(actor)?)
}
fn aad(client_id: &str) -> String {
    format!("partner-callback:{client_id}")
}
impl Partners {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    // ----- Staff registry (security administrators; maker/checker approval) -----
    /// Proposes a client. When a callback URL is given, a fresh 32-byte signing secret is generated, sealed at rest
    /// and returned once, so it can be handed to the partner out of band.
    pub async fn propose(
        &self,
        actor: &ActorContext,
        person: PartyId,
        key: Uuid,
        data: &Value,
    ) -> Result<(Value, Option<String>)> {
        let client_id = data["clientId"].as_str().unwrap_or_default().to_owned();
        let secret = if data["callbackUrl"].as_str().is_some_and(|u| !u.is_empty()) {
            let mut raw = [0u8; 32];
            getrandom::fill(&mut raw).map_err(|_| anyhow::anyhow!("entropy unavailable"))?;
            Some(raw.iter().map(|b| format!("{b:02x}")).collect::<String>())
        } else {
            None
        };
        let sealed = match &secret {
            Some(s) => Some(
                KeyRing::load()
                    .map_err(anyhow::Error::msg)?
                    .seal(s, &aad(&client_id)),
            ),
            None => None,
        };
        let row: Value = sqlx::query_scalar("SELECT identity.partner_propose($1,$2,$3,$4,$5)")
            .bind(person_json(actor)?)
            .bind(person.0)
            .bind(key)
            .bind(data)
            .bind(sealed)
            .fetch_one(&self.pool)
            .await?;
        // A replayed proposal returns the original record; the secret is only ever shown on first creation.
        let fresh = row["state"] == "PENDING_APPROVAL" && row["version"] == 1;
        Ok((row, if fresh { secret } else { None }))
    }
    pub async fn decide(
        &self,
        actor: &ActorContext,
        person: PartyId,
        id: Uuid,
        decision: &str,
        reason: &str,
    ) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT identity.partner_decide($1,$2,$3,$4,$5)")
                .bind(person_json(actor)?)
                .bind(person.0)
                .bind(id)
                .bind(decision)
                .bind(reason)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn staff_view(&self, actor: &ActorContext, person: PartyId) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT identity.partners_view($1,$2)")
            .bind(person_json(actor)?)
            .bind(person.0)
            .fetch_one(&self.pool)
            .await?)
    }
    // ----- Partner side -----
    /// Registered ACTIVE client for a verified token's `azp`, or `None` (the caller answers 401).
    pub async fn active_client(&self, client_id: &str) -> Result<Option<Value>> {
        Ok(
            sqlx::query_scalar::<_, Option<Value>>("SELECT identity.partner_client_active($1)")
                .bind(client_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn quota_permits(&self, client_id: &str, limit: i64) -> Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT identity.partner_quota_permits($1,$2::integer)")
                .bind(client_id)
                .bind(limit)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn api_version(&self, version: &str) -> Result<Option<Value>> {
        Ok(
            sqlx::query_scalar::<_, Option<Value>>("SELECT identity.partner_api_version($1)")
                .bind(version)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn request_consent(&self, client_id: &str, key: Uuid, data: &Value) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT identity.consent_request($1,$2,$3)")
                .bind(client_id)
                .bind(key)
                .bind(data)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn partner_status(&self, client_id: &str, id: Uuid) -> Result<Option<Value>> {
        let v: Option<Value> =
            sqlx::query_scalar("SELECT identity.consent_status_for_partner($1,$2)")
                .bind(client_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        Ok(v)
    }
    pub async fn partner_withdraw(&self, client_id: &str, id: Uuid) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT identity.consent_partner_withdraw($1,$2)")
                .bind(client_id)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn binding(&self, client_id: &str, id: Uuid) -> Result<Option<Binding>> {
        let v: Option<Value> = sqlx::query_scalar("SELECT identity.consent_binding($1,$2)")
            .bind(client_id)
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let Some(v) = v else { return Ok(None) };
        let strings = |field: &str| -> Vec<String> {
            v[field]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let account_ids = strings("accountIds")
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect::<Vec<_>>();
        let person = v["personId"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("Consent binding without person"))?;
        ensure!(!account_ids.is_empty(), "Consent binding without accounts");
        Ok(Some(Binding {
            consent_id: id,
            person: PartyId(person),
            issuer: v["issuer"].as_str().unwrap_or_default().to_owned(),
            subject: v["subject"].as_str().unwrap_or_default().to_owned(),
            scopes: strings("scopes"),
            account_ids,
        }))
    }
    // ----- Customer side -----
    pub async fn customer_view(&self, person: PartyId, id: Uuid) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT identity.consent_for_customer($1,$2)")
                .bind(person.0)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn customer_list(&self, person: PartyId) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT identity.customer_consents($1)")
            .bind(person.0)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn authorize(
        &self,
        actor: &ActorContext,
        person: PartyId,
        id: Uuid,
        approve: bool,
        accounts: &[Uuid],
    ) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT identity.consent_authorize($1,$2,$3,$4,$5)")
                .bind(person_json(actor)?)
                .bind(person.0)
                .bind(id)
                .bind(approve)
                .bind(accounts)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Customer (own consent) or security administrator revocation; prospective only.
    pub async fn revoke(
        &self,
        actor: &ActorContext,
        person: PartyId,
        id: Uuid,
        reason: &str,
    ) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT identity.consent_revoke($1,$2,$3,$4)")
                .bind(person_json(actor)?)
                .bind(person.0)
                .bind(id)
                .bind(reason)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    // ----- Authenticated callbacks (worker) -----
    /// Delivers at most one due callback. The body is signed as HMAC-SHA256 over `"<timestamp>.<body>"` with the
    /// client's sealed secret and sent with `Piggybank-Signature: t=<timestamp>,v1=<hex>`. Returns whether any work
    /// was found. Only loopback callback URLs are accepted by the registry in this synthetic profile.
    pub async fn deliver_callback(&self, http: &reqwest::Client) -> Result<bool> {
        let claimed: Option<Value> = sqlx::query_scalar("SELECT identity.claim_partner_callback()")
            .fetch_one(&self.pool)
            .await?;
        let Some(job) = claimed else {
            return Ok(false);
        };
        let id = job["id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("callback without id"))?;
        let client_id = job["clientId"].as_str().unwrap_or_default();
        let url = job["url"].as_str().unwrap_or_default();
        let status = match KeyRing::load()
            .ok()
            .and_then(|ring| ring.open(job["sealedSecret"].as_str()?, &aad(client_id)))
        {
            Some(secret) if url.starts_with("http://127.0.0.1:") => {
                let body = serde_json::to_vec(&job["payload"])?;
                let stamp = chrono::Utc::now().timestamp().to_string();
                let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
                let mut signed = stamp.as_bytes().to_vec();
                signed.push(b'.');
                signed.extend_from_slice(&body);
                let tag = hmac::sign(&key, &signed);
                let hex: String = tag.as_ref().iter().map(|b| format!("{b:02x}")).collect();
                match http
                    .post(url)
                    .timeout(Duration::from_secs(5))
                    .header("Content-Type", "application/json")
                    .header("Piggybank-Event-Id", id.to_string())
                    .header("Piggybank-Signature", format!("t={stamp},v1={hex}"))
                    .body(body)
                    .send()
                    .await
                {
                    Ok(response) => i32::from(response.status().as_u16()),
                    Err(_) => 0,
                }
            }
            // An unreadable secret or a non-loopback URL is recorded as a failed attempt, never sent.
            _ => 0,
        };
        sqlx::query("SELECT identity.record_partner_callback($1,$2)")
            .bind(id)
            .bind(status)
            .execute(&self.pool)
            .await?;
        Ok(true)
    }
}
