//! WP-35 ATM channel (FR-CH-04) against a deterministic network simulator. Network messages and settlement files are
//! HMAC-SHA256 signed with the network key; responses are signed back. Each message id has exactly one stored response,
//! and each authorization id is the Core operation for its hold and its single `ATM_WITHDRAWAL` posting, so duplicate
//! authorization, completion or reversal messages have bounded, unique financial effects. Only the operations enabled
//! in the channel supplement run; every other operation is declined explicitly with no effect.
use crate::Payments;
use anyhow::{Result, ensure};
use bank_controls::entitlements::Action;
use bank_platform::domain::{AccountId, ActorContext, Currency, Money, TrustContext, fingerprint};
use ring::hmac;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

/// Synthetic network key (local profile only; production keys come from the chosen network, DEC-04/09).
fn network_key() -> hmac::Key {
    let secret = std::env::var("BANK_ATM_NETWORK_KEY")
        .unwrap_or_else(|_| "synthetic-atm-network-key-local-only".into());
    hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes())
}
pub fn sign(body: &[u8]) -> String {
    hmac::sign(&network_key(), body)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn verify(body: &[u8], signature: &str) -> bool {
    let Some(raw) = (0..signature.len())
        .step_by(2)
        .map(|i| {
            signature
                .get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect::<Option<Vec<u8>>>()
    else {
        return false;
    };
    hmac::verify(&network_key(), body, &raw).is_ok()
}
/// Returned when the signature is missing or wrong; the API answers 401 without processing anything.
#[derive(Debug)]
pub struct InvalidSignature;
impl std::fmt::Display for InvalidSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid network signature")
    }
}
impl std::error::Error for InvalidSignature {}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Message {
    message_id: String,
    kind: String,
    terminal_id: String,
    card_token: String,
    pin_verified: bool,
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    amount_minor: Option<i64>,
    #[serde(default)]
    authorization_id: Option<Uuid>,
}

impl Payments {
    /// Handles one signed network message and returns the signed response body.
    pub async fn atm_message(&self, body: &[u8], signature: &str) -> Result<(Vec<u8>, String)> {
        ensure!(verify(body, signature), InvalidSignature);
        let message: Message = serde_json::from_slice(body)?;
        ensure!(
            (8..=64).contains(&message.message_id.len())
                && matches!(
                    message.kind.as_str(),
                    "INQUIRY" | "AUTHORIZATION" | "COMPLETION" | "REVERSAL"
                ),
            "Invalid ATM message"
        );
        let digest = fingerprint(&message)?;
        let document = serde_json::to_value(&message)?;
        sqlx::query("INSERT INTO payments.atm_messages(message_id,kind,fingerprint,document) VALUES($1,$2,$3,$4) ON CONFLICT(message_id) DO NOTHING")
            .bind(&message.message_id)
            .bind(&message.kind)
            .bind(&digest)
            .bind(&document)
            .execute(&self.pool)
            .await?;
        let stored = sqlx::query(
            "SELECT fingerprint,response FROM payments.atm_messages WHERE message_id=$1",
        )
        .bind(&message.message_id)
        .fetch_one(&self.pool)
        .await?;
        ensure!(
            stored.get::<String, _>("fingerprint") == digest,
            "ATM message identity conflict"
        );
        if let Some(response) = stored.get::<Option<Value>, _>("response") {
            let raw = serde_json::to_vec(&response)?;
            let signature = sign(&raw);
            return Ok((raw, signature));
        }
        let mut response = self.atm_process(&message).await?;
        response["messageId"] = json!(message.message_id);
        // The first stored response wins; a concurrent duplicate returns it unchanged.
        sqlx::query("UPDATE payments.atm_messages SET response=$2,responded_at=clock_timestamp() WHERE message_id=$1 AND response IS NULL")
            .bind(&message.message_id)
            .bind(&response)
            .execute(&self.pool)
            .await?;
        let response: Value =
            sqlx::query_scalar("SELECT response FROM payments.atm_messages WHERE message_id=$1")
                .bind(&message.message_id)
                .fetch_one(&self.pool)
                .await?;
        let raw = serde_json::to_vec(&response)?;
        let signature = sign(&raw);
        Ok((raw, signature))
    }
    async fn atm_process(&self, m: &Message) -> Result<Value> {
        let decline = |reason: &str| json!({"result": "DECLINED", "reason": reason});
        let Some(card) = sqlx::query(
            "SELECT id,account_id,person_id FROM payments.atm_cards WHERE token=$1 AND state='ACTIVE'",
        )
        .bind(&m.card_token)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(decline("CARD_NOT_ACTIVE"));
        };
        // The card works only while its holder still owns the account (Core is the relationship authority).
        let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core.account_relationships WHERE account_id=$1 AND person_id=$2 AND kind='OWNER' AND revoked_at IS NULL)")
            .bind(card.get::<Uuid, _>("account_id"))
            .bind(card.get::<Uuid, _>("person_id"))
            .fetch_one(&self.core.pool)
            .await?;
        if !owned {
            return Ok(decline("CARD_NOT_ACTIVE"));
        }
        if !m.pin_verified {
            return Ok(decline("CHANNEL_AUTHENTICATION_FAILED"));
        }
        let card_id: Uuid = card.get("id");
        let account = AccountId(card.get("account_id"));
        match m.kind.as_str() {
            "INQUIRY" => {
                let restricted: bool = sqlx::query_scalar(
                    "SELECT state<>'ACTIVE' FROM core.ledger_accounts WHERE id=$1",
                )
                .bind(account.0)
                .fetch_one(&self.core.pool)
                .await?;
                if restricted {
                    return Ok(decline("ACCOUNT_RESTRICTED"));
                }
                let balance = self.core.balance(account).await?;
                Ok(
                    json!({"result": "APPROVED", "availableMinor": balance.available.minor_units().to_string()}),
                )
            }
            "AUTHORIZATION" => self.atm_authorize(m, card_id, account).await,
            "COMPLETION" | "REVERSAL" => {
                let Some(id) = m.authorization_id else {
                    return Ok(decline("AUTHORIZATION_REQUIRED"));
                };
                let Some(auth) = sqlx::query("SELECT state,amount_minor,completion_batch,expires_at>clock_timestamp() AS open FROM payments.atm_authorizations WHERE id=$1 AND card_id=$2")
                    .bind(id)
                    .bind(card_id)
                    .fetch_optional(&self.pool)
                    .await?
                else {
                    return Ok(decline("AUTHORIZATION_NOT_FOUND"));
                };
                let state: String = auth.get("state");
                let amount = Money::new(auth.get("amount_minor"), Currency::synthetic());
                if m.kind == "COMPLETION" {
                    match state.as_str() {
                        "APPROVED" if auth.get::<bool, _>("open") => {
                            let batch = self.core.atm_withdrawal(id, account, amount).await?;
                            sqlx::query("UPDATE payments.atm_authorizations SET state='COMPLETED',completion_batch=$2,updated_at=clock_timestamp() WHERE id=$1 AND state='APPROVED'")
                                .bind(id)
                                .bind(batch)
                                .execute(&self.pool)
                                .await?;
                            Ok(
                                json!({"result": "COMPLETED", "authorizationId": id, "batchId": batch}),
                            )
                        }
                        "COMPLETED" => Ok(
                            json!({"result": "COMPLETED", "authorizationId": id, "batchId": auth.get::<Option<Uuid>, _>("completion_batch")}),
                        ),
                        _ => Ok(decline("AUTHORIZATION_NOT_OPEN")),
                    }
                } else {
                    match state.as_str() {
                        "APPROVED" => {
                            self.core.release(id).await?;
                            sqlx::query("UPDATE payments.atm_authorizations SET state='REVERSED',updated_at=clock_timestamp() WHERE id=$1 AND state='APPROVED'")
                                .bind(id)
                                .execute(&self.pool)
                                .await?;
                            Ok(json!({"result": "REVERSED", "authorizationId": id}))
                        }
                        // Cash was reported dispensed: never auto-reverse; flag for reconciliation review.
                        "COMPLETED" => {
                            sqlx::query("UPDATE payments.atm_authorizations SET late_reversal_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=$1 AND state='COMPLETED' AND late_reversal_at IS NULL")
                                .bind(id)
                                .execute(&self.pool)
                                .await?;
                            Ok(
                                json!({"result": "REVIEW_REQUIRED", "authorizationId": id, "reason": "LATE_REVERSAL_AFTER_COMPLETION"}),
                            )
                        }
                        _ => Ok(
                            json!({"result": "ACCEPTED", "authorizationId": id, "reason": "NO_OPEN_HOLD"}),
                        ),
                    }
                }
            }
            _ => Ok(decline("INVALID_KIND")),
        }
    }
    async fn atm_authorize(&self, m: &Message, card_id: Uuid, account: AccountId) -> Result<Value> {
        let decline = |reason: &str| json!({"result": "DECLINED", "reason": reason});
        let operation = m.operation.clone().unwrap_or_default();
        let Some(op) = sqlx::query(
            "SELECT enabled,decline_reason FROM payments.atm_operations WHERE operation=$1",
        )
        .bind(&operation)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(decline("OPERATION_NOT_SUPPORTED"));
        };
        if !op.get::<bool, _>("enabled") {
            return Ok(decline(&op.get::<String, _>("decline_reason")));
        }
        if operation != "CASH_WITHDRAWAL" {
            return Ok(decline("OPERATION_NOT_SUPPORTED"));
        }
        let amount = m.amount_minor.unwrap_or(0);
        let limits = sqlx::query("SELECT per_withdrawal_minor,daily_minor,authorization_seconds FROM payments.atm_limits WHERE id=1")
            .fetch_one(&self.pool)
            .await?;
        if amount <= 0 || !(amount as u64).is_multiple_of(1000) {
            return Ok(decline("INVALID_AMOUNT"));
        }
        // The authorization row is written first (PENDING) so a retried message reuses the same Core operation.
        sqlx::query("INSERT INTO payments.atm_authorizations(id,message_id,card_id,account_id,terminal_id,amount_minor,state,expires_at) VALUES($1,$2,$3,$4,$5,$6,'PENDING',clock_timestamp()+make_interval(secs=>$7::double precision)) ON CONFLICT(message_id) DO NOTHING")
            .bind(Uuid::new_v4())
            .bind(&m.message_id)
            .bind(card_id)
            .bind(account.0)
            .bind(&m.terminal_id)
            .bind(amount)
            .bind(limits.get::<i32, _>("authorization_seconds"))
            .execute(&self.pool)
            .await?;
        let auth = sqlx::query(
            "SELECT id,state,decline_reason FROM payments.atm_authorizations WHERE message_id=$1",
        )
        .bind(&m.message_id)
        .fetch_one(&self.pool)
        .await?;
        let id: Uuid = auth.get("id");
        match auth.get::<String, _>("state").as_str() {
            "PENDING" => {}
            "DECLINED" => return Ok(decline(&auth.get::<String, _>("decline_reason"))),
            _ => return Ok(json!({"result": "APPROVED", "authorizationId": id})),
        }
        let used: i64 = sqlx::query_scalar("SELECT coalesce(sum(amount_minor),0)::bigint FROM payments.atm_authorizations WHERE card_id=$1 AND business_date=(clock_timestamp() AT TIME ZONE 'UTC')::date AND state IN ('APPROVED','COMPLETED')")
            .bind(card_id)
            .fetch_one(&self.pool)
            .await?;
        let reason = if amount > limits.get::<i64, _>("per_withdrawal_minor")
            || used + amount > limits.get::<i64, _>("daily_minor")
        {
            Some("LIMIT_EXCEEDED".to_owned())
        } else {
            match self
                .core
                .reserve(id, account, Money::new(amount, Currency::synthetic()))
                .await
            {
                Ok(()) => None,
                Err(_) => {
                    let restricted: bool = sqlx::query_scalar("SELECT state<>'ACTIVE' OR debit_blocked FROM core.ledger_accounts WHERE id=$1")
                        .bind(account.0)
                        .fetch_one(&self.core.pool)
                        .await?;
                    Some(
                        if restricted {
                            "ACCOUNT_RESTRICTED"
                        } else {
                            "INSUFFICIENT_FUNDS"
                        }
                        .to_owned(),
                    )
                }
            }
        };
        match reason {
            None => {
                sqlx::query("UPDATE payments.atm_authorizations SET state='APPROVED',updated_at=clock_timestamp() WHERE id=$1 AND state='PENDING'")
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                Ok(json!({"result": "APPROVED", "authorizationId": id}))
            }
            Some(reason) => {
                sqlx::query("UPDATE payments.atm_authorizations SET state='DECLINED',decline_reason=$2,updated_at=clock_timestamp() WHERE id=$1 AND state='PENDING'")
                    .bind(id)
                    .bind(&reason)
                    .execute(&self.pool)
                    .await?;
                Ok(decline(&reason))
            }
        }
    }
    /// Releases holds for authorizations the network never completed (timeout). Returns how many expired.
    pub async fn atm_expire_due(&self) -> Result<u64> {
        let due: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM payments.atm_authorizations WHERE state='APPROVED' AND expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 50")
            .fetch_all(&self.pool)
            .await?;
        let mut count = 0;
        for id in due {
            self.core.release(id).await?;
            count += sqlx::query("UPDATE payments.atm_authorizations SET state='EXPIRED',updated_at=clock_timestamp() WHERE id=$1 AND state='APPROVED'")
                .bind(id)
                .execute(&self.pool)
                .await?
                .rows_affected();
        }
        Ok(count)
    }
    /// Reconciles a signed network settlement file against the bank's completed withdrawals for the day.
    pub async fn atm_settlement(&self, body: &[u8], signature: &str) -> Result<Value> {
        ensure!(verify(body, signature), InvalidSignature);
        let document: Value = serde_json::from_slice(body)?;
        let file = document["fileId"].as_str().unwrap_or_default().to_owned();
        let date = chrono::NaiveDate::parse_from_str(
            document["businessDate"].as_str().unwrap_or_default(),
            "%Y-%m-%d",
        )?;
        Ok(
            sqlx::query_scalar("SELECT payments.atm_reconcile($1,$2,$3,$4)")
                .bind(&file)
                .bind(date)
                .bind(fingerprint(&document)?)
                .bind(&document)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    // ----- Customer card tokens (synthetic; card issuing is out of scope) -----
    pub async fn atm_issue_card(
        &self,
        actor: &ActorContext,
        key: Uuid,
        account: AccountId,
    ) -> Result<Value> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        // Owners only: ServiceAccount excludes delegates and assisted staff.
        self.authorize_service(actor, account).await?;
        let person = self.controls.person(actor).await?;
        let mut raw = [0u8; 8];
        getrandom_fill(&mut raw)?;
        let token = format!(
            "ATM-{}",
            raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        sqlx::query("INSERT INTO payments.atm_cards(id,token,account_id,person_id,issuer,subject,state,request_key) VALUES($1,$2,$3,$4,$5,$6,'ACTIVE',$7) ON CONFLICT(person_id,request_key) DO NOTHING")
            .bind(Uuid::new_v4())
            .bind(&token)
            .bind(account.0)
            .bind(person.0)
            .bind(&actor.issuer)
            .bind(&actor.subject)
            .bind(key)
            .execute(&self.pool)
            .await?;
        let row = sqlx::query("SELECT id,token,account_id,state FROM payments.atm_cards WHERE person_id=$1 AND request_key=$2")
            .bind(person.0)
            .bind(key)
            .fetch_one(&self.pool)
            .await?;
        ensure!(
            row.get::<Uuid, _>("account_id") == account.0,
            "ATM card request identity conflict"
        );
        Ok(
            json!({"id": row.get::<Uuid, _>("id"), "token": row.get::<String, _>("token"), "accountId": account.0, "state": row.get::<String, _>("state")}),
        )
    }
    pub async fn atm_cards(&self, actor: &ActorContext) -> Result<Value> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        let person = self.controls.person(actor).await?;
        Ok(sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'token',token,'accountId',account_id,'state',state,'createdAt',created_at) ORDER BY created_at DESC),'[]'::jsonb) FROM payments.atm_cards WHERE person_id=$1")
            .bind(person.0)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn atm_block_card(&self, actor: &ActorContext, id: Uuid) -> Result<()> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        let person = self.controls.person(actor).await?;
        let changed = sqlx::query("UPDATE payments.atm_cards SET state='BLOCKED',blocked_at=clock_timestamp() WHERE id=$1 AND person_id=$2 AND state='ACTIVE'")
            .bind(id)
            .bind(person.0)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if changed == 0 {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM payments.atm_cards WHERE id=$1 AND person_id=$2)",
            )
            .bind(id)
            .bind(person.0)
            .fetch_one(&self.pool)
            .await?;
            ensure!(exists, crate::ResourceUnavailable);
        }
        Ok(())
    }
    pub async fn atm_staff_view(&self, actor: &ActorContext) -> Result<Value> {
        ensure!(
            self.controls.payment_inquiry_authorized(actor).await?,
            "Current payment inquiry authority required"
        );
        Ok(sqlx::query_scalar("SELECT jsonb_build_object(
            'operations',(SELECT jsonb_agg(jsonb_build_object('operation',operation,'enabled',enabled,'declineReason',decline_reason) ORDER BY operation) FROM payments.atm_operations),
            'limits',(SELECT jsonb_build_object('perWithdrawalMinor',per_withdrawal_minor::text,'dailyMinor',daily_minor::text,'authorizationSeconds',authorization_seconds) FROM payments.atm_limits),
            'authorizations',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'terminalId',terminal_id,'amountMinor',amount_minor::text,'state',state,'declineReason',decline_reason,
              'lateReversal',late_reversal_at IS NOT NULL,'createdAt',created_at) ORDER BY created_at DESC) FROM (SELECT * FROM payments.atm_authorizations ORDER BY created_at DESC LIMIT 50) a),'[]'::jsonb),
            'settlements',coalesce((SELECT jsonb_agg(result ORDER BY received_at DESC) FROM (SELECT * FROM payments.atm_settlement_files ORDER BY received_at DESC LIMIT 20) f),'[]'::jsonb))")
            .fetch_one(&self.pool)
            .await?)
    }
    async fn authorize_service(&self, actor: &ActorContext, account: AccountId) -> Result<()> {
        let resource = self.core.authority(account, Action::ServiceAccount).await?;
        ensure!(
            self.controls
                .authorize(actor, Action::ServiceAccount, Some(&resource), account.0)
                .await?,
            crate::ResourceUnavailable
        );
        Ok(())
    }
}
fn getrandom_fill(buffer: &mut [u8]) -> Result<()> {
    use ring::rand::SecureRandom;
    ring::rand::SystemRandom::new()
        .fill(buffer)
        .map_err(|_| anyhow::anyhow!("entropy unavailable"))
}
