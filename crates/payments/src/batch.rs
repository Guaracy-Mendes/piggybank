//! WP-36 synthetic ACH-style batch credit rail (`SIM_BATCH`), the template for later rails. A customer drafts an
//! entry to a batch recipient; approval reuses the common action-bound challenge and step-up; confirmation reserves a
//! Core hold keyed by the entry id. A cutoff seals queued entries into an outbound file. The network acknowledges
//! (rejections release holds), settles on the settlement date (one `BATCH_CREDIT` per entry) and may return entries
//! within the return window (one `BATCH_RETURN`, linked to the credit). Late returns are flagged, never posted.
//! Network messages are HMAC-signed with stored, replayable responses, like the ATM channel.
use crate::Payments;
use anyhow::{Result, ensure};
use bank_controls::entitlements::{Action, ApprovalRef};
use bank_platform::domain::{
    AccountId, ActorContext, Currency, Money, PaymentDetails, TrustContext, fingerprint,
};
use ring::hmac;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

pub const RAIL: &str = "SIM_BATCH";
fn key() -> hmac::Key {
    let secret = std::env::var("BANK_BATCH_NETWORK_KEY")
        .unwrap_or_else(|_| "synthetic-batch-network-key-local-only".into());
    hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes())
}
pub fn sign(body: &[u8]) -> String {
    hmac::sign(&key(), body)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn verify(body: &[u8], signature: &str) -> bool {
    let raw: Option<Vec<u8>> = (0..signature.len())
        .step_by(2)
        .map(|i| {
            signature
                .get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        })
        .collect();
    raw.is_some_and(|raw| hmac::verify(&key(), body, &raw).is_ok())
}
/// ABA-style routing checksum: 3·(d1+d4+d7) + 7·(d2+d5+d8) + (d3+d6+d9) ≡ 0 (mod 10).
pub fn valid_routing(routing: &str) -> bool {
    let d: Vec<u32> = routing.chars().filter_map(|c| c.to_digit(10)).collect();
    d.len() == 9
        && routing.len() == 9
        && (3 * (d[0] + d[3] + d[6]) + 7 * (d[1] + d[4] + d[7]) + (d[2] + d[5] + d[8]))
            .is_multiple_of(10)
}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NetworkMessage {
    message_id: String,
    kind: String,
    #[serde(default)]
    file_id: Option<Uuid>,
    #[serde(default)]
    rejected: Vec<Rejection>,
    #[serde(default)]
    entry_id: Option<Uuid>,
    #[serde(default)]
    reason_code: Option<String>,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Rejection {
    entry_id: Uuid,
    reason_code: String,
}

impl Payments {
    fn batch_customer(actor: &ActorContext) -> Result<()> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        Ok(())
    }
    pub async fn batch_add_recipient(
        &self,
        actor: &ActorContext,
        key: Uuid,
        label: &str,
        routing: &str,
        account: &str,
    ) -> Result<Value> {
        Self::batch_customer(actor)?;
        ensure!(valid_routing(routing), "Invalid routing number");
        ensure!(
            account.len() >= 4
                && account.len() <= 17
                && account.chars().all(|c| c.is_ascii_digit()),
            "Invalid account number"
        );
        let person = self.controls.person(actor).await?;
        sqlx::query("INSERT INTO payments.batch_recipients(id,person_id,issuer,subject,label,routing_number,account_number,request_key) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(person_id,request_key) DO NOTHING")
            .bind(Uuid::new_v4()).bind(person.0).bind(&actor.issuer).bind(&actor.subject).bind(label.trim()).bind(routing).bind(account).bind(key)
            .execute(&self.pool).await?;
        let row = sqlx::query("SELECT id,label,routing_number,account_number FROM payments.batch_recipients WHERE person_id=$1 AND request_key=$2")
            .bind(person.0).bind(key).fetch_one(&self.pool).await?;
        ensure!(
            row.get::<String, _>("routing_number") == routing
                && row.get::<String, _>("account_number") == account,
            "Recipient request identity conflict"
        );
        Ok(recipient_json(&row))
    }
    pub async fn batch_recipients(&self, actor: &ActorContext) -> Result<Value> {
        Self::batch_customer(actor)?;
        let person = self.controls.person(actor).await?;
        let rows = sqlx::query("SELECT id,label,routing_number,account_number FROM payments.batch_recipients WHERE person_id=$1 ORDER BY created_at DESC LIMIT 100")
            .bind(person.0).fetch_all(&self.pool).await?;
        Ok(Value::Array(rows.iter().map(recipient_json).collect()))
    }
    fn batch_details(
        account: AccountId,
        recipient: Uuid,
        amount: i64,
        entry: Uuid,
        created: chrono::NaiveDate,
    ) -> PaymentDetails {
        PaymentDetails {
            payer_account_id: account,
            beneficiary_id: recipient.to_string(),
            amount: Money::new(amount, Currency::synthetic()),
            fee: Money::new(0, Currency::synthetic()),
            requested_date: created,
            rail: RAIL.into(),
            quote_id: entry.to_string(),
        }
    }
    /// Drafts an entry and opens its action-bound approval challenge (same step-up as every payment).
    pub async fn batch_create(
        &self,
        actor: &ActorContext,
        key: Uuid,
        account: AccountId,
        recipient: Uuid,
        amount: i64,
    ) -> Result<Value> {
        Self::batch_customer(actor)?;
        let person = self.controls.person(actor).await?;
        let money = Money::new(amount, Currency::synthetic());
        money.require_positive()?;
        let max: i64 = sqlx::query_scalar(
            "SELECT per_entry_max_minor FROM payments.batch_supplement WHERE id=1",
        )
        .fetch_one(&self.pool)
        .await?;
        ensure!(amount <= max, "Batch entry limit exceeded");
        // Common rules: ownership or delegated PAY authority within its limit, account restrictions.
        self.authorize_amount(actor, account, &money).await?;
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM payments.batch_recipients WHERE id=$1 AND person_id=$2)",
        )
        .bind(recipient)
        .bind(person.0)
        .fetch_one(&self.pool)
        .await?;
        ensure!(owned, crate::ResourceUnavailable);
        sqlx::query("INSERT INTO payments.batch_entries(id,person_id,issuer,subject,actor_context,account_id,recipient_id,amount_minor,request_key,state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'DRAFT') ON CONFLICT(person_id,request_key) DO NOTHING")
            .bind(Uuid::new_v4()).bind(person.0).bind(&actor.issuer).bind(&actor.subject).bind(serde_json::to_value(actor)?).bind(account.0).bind(recipient).bind(amount).bind(key)
            .execute(&self.pool).await?;
        let row = sqlx::query("SELECT id,account_id,recipient_id,amount_minor,challenge_id,created_at FROM payments.batch_entries WHERE person_id=$1 AND request_key=$2")
            .bind(person.0).bind(key).fetch_one(&self.pool).await?;
        ensure!(
            row.get::<Uuid, _>("account_id") == account.0
                && row.get::<Uuid, _>("recipient_id") == recipient
                && row.get::<i64, _>("amount_minor") == amount,
            "Batch entry request identity conflict"
        );
        let id: Uuid = row.get("id");
        if row.get::<Option<Uuid>, _>("challenge_id").is_none() {
            let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
            let details = Self::batch_details(account, recipient, amount, id, created.date_naive());
            let authority = self.core.authority(account, Action::CreatePayment).await?;
            let challenge = self
                .controls
                .challenge(actor, id, &details, &authority, id)
                .await?;
            sqlx::query("UPDATE payments.batch_entries SET challenge_id=$2,updated_at=clock_timestamp() WHERE id=$1 AND challenge_id IS NULL")
                .bind(id).bind(challenge.id).execute(&self.pool).await?;
        }
        self.batch_detail(actor, id).await
    }
    pub async fn batch_detail(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        Self::batch_customer(actor)?;
        let person = self.controls.person(actor).await?;
        let v: Option<Value> = sqlx::query_scalar(ENTRY_JSON)
            .bind(id)
            .bind(person.0)
            .fetch_optional(&self.pool)
            .await?;
        v.ok_or_else(|| crate::ResourceUnavailable.into())
    }
    pub async fn batch_list(&self, actor: &ActorContext) -> Result<Value> {
        Self::batch_customer(actor)?;
        let person = self.controls.person(actor).await?;
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM payments.batch_entries WHERE person_id=$1 ORDER BY created_at DESC LIMIT 50")
            .bind(person.0).fetch_all(&self.pool).await?;
        let mut list = vec![];
        for id in ids {
            list.push(self.batch_detail(actor, id).await?);
        }
        Ok(Value::Array(list))
    }
    /// Consumes the step-up proof (common approval) and reserves the hold; the entry joins the next file.
    pub async fn batch_confirm(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        Self::batch_customer(actor)?;
        let person = self.controls.person(actor).await?;
        let row = sqlx::query("SELECT state,account_id,recipient_id,amount_minor,challenge_id,created_at FROM payments.batch_entries WHERE id=$1 AND person_id=$2")
            .bind(id).bind(person.0).fetch_optional(&self.pool).await?.ok_or(crate::ResourceUnavailable)?;
        if row.get::<String, _>("state") != "DRAFT" {
            return self.batch_detail(actor, id).await;
        }
        let account = AccountId(row.get("account_id"));
        let amount: i64 = row.get("amount_minor");
        let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
        let details = Self::batch_details(
            account,
            row.get("recipient_id"),
            amount,
            id,
            created.date_naive(),
        );
        let challenge: Uuid = row
            .get::<Option<Uuid>, _>("challenge_id")
            .ok_or_else(|| anyhow::anyhow!("Action approval required"))?;
        // Re-checked at confirmation: a revoked delegation or new restriction applies now.
        self.authorize_amount(actor, account, &details.amount)
            .await?;
        let authority = self.core.authority(account, Action::CreatePayment).await?;
        self.controls
            .approve_payment(
                &self.identity,
                actor,
                ApprovalRef {
                    challenge_id: challenge,
                    operation_id: id,
                },
                &details,
                &authority,
                id,
            )
            .await?;
        self.core.reserve(id, account, details.amount).await?;
        sqlx::query("UPDATE payments.batch_entries SET state='QUEUED',settlement_date=payments.batch_settlement_date(clock_timestamp()),updated_at=clock_timestamp() WHERE id=$1 AND state='DRAFT'")
            .bind(id).execute(&self.pool).await?;
        self.batch_detail(actor, id).await
    }
    /// Before the file is sealed the customer may cancel; the hold is released.
    pub async fn batch_cancel(&self, actor: &ActorContext, id: Uuid) -> Result<Value> {
        Self::batch_customer(actor)?;
        let person = self.controls.person(actor).await?;
        let state: String = sqlx::query_scalar(
            "SELECT state FROM payments.batch_entries WHERE id=$1 AND person_id=$2",
        )
        .bind(id)
        .bind(person.0)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(crate::ResourceUnavailable)?;
        match state.as_str() {
            "DRAFT" => {}
            "QUEUED" => self.core.release(id).await?,
            "CANCELLED" => return self.batch_detail(actor, id).await,
            _ => anyhow::bail!("Only an unsubmitted entry can be cancelled"),
        }
        sqlx::query("UPDATE payments.batch_entries SET state='CANCELLED',updated_at=clock_timestamp() WHERE id=$1 AND state IN ('DRAFT','QUEUED')")
            .bind(id).execute(&self.pool).await?;
        self.batch_detail(actor, id).await
    }
    /// Seals all queued entries into one outbound file. Called by the worker at the cutoff and by payment operators.
    pub async fn batch_close_window(&self, closed_by: &str) -> Result<Option<Value>> {
        Ok(
            sqlx::query_scalar::<_, Option<Value>>("SELECT payments.batch_close_window($1)")
                .bind(closed_by)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// Worker: closes the window once per business day after the cutoff.
    pub async fn batch_cutoff_due(&self) -> Result<Option<Value>> {
        let due: bool = sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'UTC')::time>=s.cutoff_utc AND extract(isodow FROM (clock_timestamp() AT TIME ZONE 'UTC')::date)<=5 AND NOT EXISTS(SELECT 1 FROM payments.batch_holidays WHERE day=(clock_timestamp() AT TIME ZONE 'UTC')::date) AND EXISTS(SELECT 1 FROM payments.batch_entries WHERE state='QUEUED' AND created_at<(clock_timestamp() AT TIME ZONE 'UTC')::date+s.cutoff_utc) FROM payments.batch_supplement s WHERE s.id=1")
            .fetch_one(&self.pool).await?;
        if due {
            self.batch_close_window("CUTOFF").await
        } else {
            Ok(None)
        }
    }
    pub async fn batch_staff_close(&self, actor: &ActorContext) -> Result<Option<Value>> {
        ensure!(
            self.controls.payment_inquiry_authorized(actor).await?,
            "Current payment operations authority required"
        );
        self.batch_close_window(&format!("STAFF:{}", actor.subject))
            .await
    }
    pub async fn batch_staff_view(&self, actor: &ActorContext) -> Result<Value> {
        ensure!(
            self.controls.payment_inquiry_authorized(actor).await?,
            "Current payment inquiry authority required"
        );
        Ok(sqlx::query_scalar("SELECT jsonb_build_object(
            'supplement',(SELECT jsonb_build_object('rail',rail,'cutoffUtc',cutoff_utc::text,'settlementLagDays',settlement_lag_days,'perEntryMaxMinor',per_entry_max_minor::text,'returnWindowDays',return_window_days) FROM payments.batch_supplement),
            'returnReasons',(SELECT jsonb_agg(jsonb_build_object('code',code,'description',description) ORDER BY code) FROM payments.batch_return_reasons),
            'files',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'entryCount',entry_count,'totalMinor',total_minor::text,'state',state,'closedBy',closed_by,'createdAt',created_at) ORDER BY created_at DESC) FROM (SELECT * FROM payments.batch_files ORDER BY created_at DESC LIMIT 20) f),'[]'::jsonb),
            'entries',coalesce((SELECT jsonb_agg(jsonb_build_object('id',id,'amountMinor',amount_minor::text,'state',state,'settlementDate',settlement_date,'reasonCode',reason_code,'lateReturn',late_return_at IS NOT NULL,'fileId',file_id) ORDER BY created_at DESC) FROM (SELECT * FROM payments.batch_entries ORDER BY created_at DESC LIMIT 50) e),'[]'::jsonb))")
            .fetch_one(&self.pool).await?)
    }
    /// One signed network message: ACKNOWLEDGEMENT (with rejections), SETTLEMENT of a file, or RETURN of an entry.
    pub async fn batch_message(&self, body: &[u8], signature: &str) -> Result<(Vec<u8>, String)> {
        ensure!(verify(body, signature), crate::atm::InvalidSignature);
        let m: NetworkMessage = serde_json::from_slice(body)?;
        ensure!(
            (8..=64).contains(&m.message_id.len())
                && matches!(m.kind.as_str(), "ACKNOWLEDGEMENT" | "SETTLEMENT" | "RETURN"),
            "Invalid batch message"
        );
        let digest = fingerprint(&m)?;
        sqlx::query("INSERT INTO payments.batch_messages(message_id,kind,fingerprint,document) VALUES($1,$2,$3,$4) ON CONFLICT(message_id) DO NOTHING")
            .bind(&m.message_id).bind(&m.kind).bind(&digest).bind(serde_json::to_value(&m)?).execute(&self.pool).await?;
        let stored = sqlx::query(
            "SELECT fingerprint,response FROM payments.batch_messages WHERE message_id=$1",
        )
        .bind(&m.message_id)
        .fetch_one(&self.pool)
        .await?;
        ensure!(
            stored.get::<String, _>("fingerprint") == digest,
            "Batch message identity conflict"
        );
        if let Some(response) = stored.get::<Option<Value>, _>("response") {
            let raw = serde_json::to_vec(&response)?;
            let s = sign(&raw);
            return Ok((raw, s));
        }
        let mut response = match m.kind.as_str() {
            "ACKNOWLEDGEMENT" => self.batch_acknowledge(&m).await?,
            "SETTLEMENT" => self.batch_settle(&m).await?,
            _ => self.batch_return_entry(&m).await?,
        };
        response["messageId"] = json!(m.message_id);
        sqlx::query("UPDATE payments.batch_messages SET response=$2,responded_at=clock_timestamp() WHERE message_id=$1 AND response IS NULL")
            .bind(&m.message_id).bind(&response).execute(&self.pool).await?;
        let response: Value =
            sqlx::query_scalar("SELECT response FROM payments.batch_messages WHERE message_id=$1")
                .bind(&m.message_id)
                .fetch_one(&self.pool)
                .await?;
        let raw = serde_json::to_vec(&response)?;
        let s = sign(&raw);
        Ok((raw, s))
    }
    async fn batch_acknowledge(&self, m: &NetworkMessage) -> Result<Value> {
        let file = m.file_id.ok_or_else(|| anyhow::anyhow!("File required"))?;
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM payments.batch_files WHERE id=$1")
                .bind(file)
                .fetch_optional(&self.pool)
                .await?;
        let Some(state) = state else {
            return Ok(json!({"result": "DECLINED", "reason": "FILE_NOT_FOUND"}));
        };
        if state != "SUBMITTED" {
            return Ok(json!({"result": "ACCEPTED", "reason": "ALREADY_ACKNOWLEDGED"}));
        }
        let mut rejected = 0;
        for r in &m.rejected {
            let ok: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM payments.batch_return_reasons WHERE code=$1)",
            )
            .bind(&r.reason_code)
            .fetch_one(&self.pool)
            .await?;
            ensure!(ok, "Unknown rejection reason");
            let open: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payments.batch_entries WHERE id=$1 AND file_id=$2 AND state='SUBMITTED')").bind(r.entry_id).bind(file).fetch_one(&self.pool).await?;
            if open {
                self.core.release(r.entry_id).await?;
                sqlx::query("UPDATE payments.batch_entries SET state='REJECTED',reason_code=$2,updated_at=clock_timestamp() WHERE id=$1 AND state='SUBMITTED'")
                    .bind(r.entry_id).bind(&r.reason_code).execute(&self.pool).await?;
                rejected += 1;
            }
        }
        sqlx::query("UPDATE payments.batch_files SET state='ACKNOWLEDGED',acknowledged_at=clock_timestamp() WHERE id=$1 AND state='SUBMITTED'").bind(file).execute(&self.pool).await?;
        Ok(json!({"result": "ACKNOWLEDGED", "fileId": file, "rejected": rejected}))
    }
    async fn batch_settle(&self, m: &NetworkMessage) -> Result<Value> {
        let file = m.file_id.ok_or_else(|| anyhow::anyhow!("File required"))?;
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM payments.batch_files WHERE id=$1")
                .bind(file)
                .fetch_optional(&self.pool)
                .await?;
        match state.as_deref() {
            None => return Ok(json!({"result": "DECLINED", "reason": "FILE_NOT_FOUND"})),
            Some("SUBMITTED") => {
                return Ok(json!({"result": "DECLINED", "reason": "FILE_NOT_ACKNOWLEDGED"}));
            }
            _ => {}
        }
        let entries = sqlx::query("SELECT id,account_id,amount_minor FROM payments.batch_entries WHERE file_id=$1 AND state IN ('SUBMITTED','SETTLED','RETURNED') ORDER BY created_at,id")
            .bind(file).fetch_all(&self.pool).await?;
        let mut settled = 0;
        for e in entries {
            let id: Uuid = e.get("id");
            let batch = self
                .core
                .batch_credit(
                    id,
                    AccountId(e.get("account_id")),
                    Money::new(e.get("amount_minor"), Currency::synthetic()),
                )
                .await?;
            settled += sqlx::query("UPDATE payments.batch_entries SET state='SETTLED',settle_batch=$2,updated_at=clock_timestamp() WHERE id=$1 AND state='SUBMITTED'")
                .bind(id).bind(batch).execute(&self.pool).await?.rows_affected();
        }
        sqlx::query("UPDATE payments.batch_files SET state='SETTLED',settled_at=clock_timestamp() WHERE id=$1 AND state='ACKNOWLEDGED'").bind(file).execute(&self.pool).await?;
        Ok(json!({"result": "SETTLED", "fileId": file, "settled": settled}))
    }
    async fn batch_return_entry(&self, m: &NetworkMessage) -> Result<Value> {
        let id = m
            .entry_id
            .ok_or_else(|| anyhow::anyhow!("Entry required"))?;
        let reason = m.reason_code.clone().unwrap_or_default();
        let known: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM payments.batch_return_reasons WHERE code=$1)",
        )
        .bind(&reason)
        .fetch_one(&self.pool)
        .await?;
        if !known {
            return Ok(json!({"result": "DECLINED", "reason": "UNKNOWN_RETURN_REASON"}));
        }
        let Some(e) = sqlx::query("SELECT e.state,e.account_id,e.amount_minor,e.settle_batch,e.return_batch,
              (clock_timestamp() AT TIME ZONE 'UTC')::date<=e.settlement_date+s.return_window_days AS in_window
              FROM payments.batch_entries e,payments.batch_supplement s WHERE e.id=$1 AND s.id=1")
            .bind(id).fetch_optional(&self.pool).await? else {
            return Ok(json!({"result": "DECLINED", "reason": "ENTRY_NOT_FOUND"}));
        };
        match e.get::<String, _>("state").as_str() {
            "RETURNED" => Ok(
                json!({"result": "RETURNED", "entryId": id, "batchId": e.get::<Option<Uuid>, _>("return_batch")}),
            ),
            "SETTLED" if e.get::<bool, _>("in_window") => {
                let batch = self
                    .core
                    .batch_return(
                        id,
                        AccountId(e.get("account_id")),
                        Money::new(e.get("amount_minor"), Currency::synthetic()),
                        e.get("settle_batch"),
                    )
                    .await?;
                sqlx::query("UPDATE payments.batch_entries SET state='RETURNED',return_batch=$2,reason_code=$3,updated_at=clock_timestamp() WHERE id=$1 AND state='SETTLED'")
                    .bind(id).bind(batch).bind(&reason).execute(&self.pool).await?;
                Ok(json!({"result": "RETURNED", "entryId": id, "batchId": batch}))
            }
            "SETTLED" => {
                sqlx::query("UPDATE payments.batch_entries SET late_return_at=coalesce(late_return_at,clock_timestamp()),updated_at=clock_timestamp() WHERE id=$1 AND state='SETTLED'")
                    .bind(id).execute(&self.pool).await?;
                Ok(json!({"result": "REVIEW_REQUIRED", "entryId": id, "reason": "LATE_RETURN"}))
            }
            _ => Ok(json!({"result": "DECLINED", "reason": "ENTRY_NOT_SETTLED"})),
        }
    }
}
fn recipient_json(row: &sqlx::postgres::PgRow) -> Value {
    let account: String = row.get("account_number");
    json!({"id": row.get::<Uuid, _>("id"), "label": row.get::<String, _>("label"), "routingNumber": row.get::<String, _>("routing_number"),
           "accountLast4": &account[account.len() - 4..]})
}
const ENTRY_JSON: &str = "SELECT jsonb_build_object('id',e.id,'accountId',e.account_id,'recipientId',e.recipient_id,'recipientLabel',r.label,'amountMinor',e.amount_minor::text,
  'state',e.state,'challengeId',e.challenge_id,'approvalUrl',CASE WHEN e.state='DRAFT' AND e.challenge_id IS NOT NULL THEN '/auth/customer/step-up/'||e.challenge_id ELSE NULL END,
  'settlementDate',e.settlement_date,'reasonCode',e.reason_code,'lateReturn',e.late_return_at IS NOT NULL,'createdAt',e.created_at,'updatedAt',e.updated_at)
  FROM payments.batch_entries e JOIN payments.batch_recipients r ON r.id=e.recipient_id WHERE e.id=$1 AND e.person_id=$2";
