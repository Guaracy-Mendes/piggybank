use crate::Finance;
use anyhow::{Result, ensure};
use bank_controls::audit::{AuditLog, AuditRecord};
use bank_platform::domain::{ActorContext, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};
use std::time::{Duration, Instant};
use uuid::Uuid;
#[derive(Debug)]
pub struct AuditDeliveryConflict;
impl std::fmt::Display for AuditDeliveryConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Audit delivery source or receipt integrity conflict")
    }
}
impl std::error::Error for AuditDeliveryConflict {}
impl Finance {
    pub(crate) async fn audit_in(
        tx: &mut Transaction<'_, Postgres>,
        actor: &ActorContext,
        action: &str,
        id: Uuid,
    ) -> Result<()> {
        let record = AuditRecord::new(actor, action, id, id);
        sqlx::query("INSERT INTO finance.audit_pending(id,record) VALUES($1,$2)")
            .bind(Uuid::new_v4())
            .bind(serde_json::to_value(record)?)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn audit_confirmation(&self, id: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT finance.confirm_audit_delivery($1)")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    async fn deliver_one_audit(&self, id: Uuid, raw: &Value) -> &'static str {
        let Ok(record) = serde_json::from_value::<AuditRecord>(raw.clone()) else {
            return "INVALID_RECORD";
        };
        if record.validate().is_err() {
            return "INVALID_RECORD";
        }
        if fingerprint(raw).ok() != fingerprint(&record).ok() {
            return "NON_CANONICAL_RECORD";
        }
        match self.audit_confirmation(id).await {
            Ok(true) => return "NATIVE_RECEIPT_CONFIRMED",
            Ok(false) => {}
            Err(e) => return confirmation_error(&e),
        }
        // Append failures are uncertain. Only independent native receipt inquiry establishes delivery.
        let _ = AuditLog::new(self.controls.pool.clone())
            .append(id, "finance", &record)
            .await;
        match self.audit_confirmation(id).await {
            Ok(true) => "NATIVE_RECEIPT_CONFIRMED",
            Ok(false) => "APPEND_UNCONFIRMED",
            Err(e) => confirmation_error(&e),
        }
    }
    pub async fn relay_audit(&self) -> Result<()> {
        let after: Uuid =
            sqlx::query_scalar("SELECT last_id FROM finance.audit_relay_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let mut rows = sqlx::query("SELECT id,record FROM finance.audit_pending WHERE delivered_at IS NULL AND id>$1 ORDER BY id LIMIT 50")
            .bind(after).fetch_all(&self.pool).await?;
        if rows.is_empty() {
            sqlx::query("UPDATE finance.audit_relay_cursor SET last_id=$1,scanned_at=clock_timestamp() WHERE id")
                .bind(Uuid::nil()).execute(&self.pool).await?;
            // A legacy zero UUID is examined only at wrap. It must neither be
            // skipped forever nor monopolize every normal pass after timeout.
            if let Some(zero) = sqlx::query(
                "SELECT id,record FROM finance.audit_pending WHERE delivered_at IS NULL AND id=$1",
            )
            .bind(Uuid::nil())
            .fetch_optional(&self.pool)
            .await?
            {
                rows.push(zero);
            } else {
                return Ok(());
            }
        }
        let started = Instant::now();
        let mut failed = false;
        for row in rows {
            if started.elapsed() >= Duration::from_secs(2) {
                break;
            }
            let id: Uuid = row.get("id");
            let raw: Value = row.get("record");
            // Commit progress before crossing Controls, one attempted identity at a time.
            // An interrupted pass resumes after this identity and wraps to retry it.
            sqlx::query("UPDATE finance.audit_relay_cursor SET last_id=$1,scanned_at=clock_timestamp() WHERE id")
                .bind(id).execute(&self.pool).await?;
            let outcome =
                tokio::time::timeout(Duration::from_secs(2), self.deliver_one_audit(id, &raw))
                    .await
                    .unwrap_or("CONFIRMATION_UNAVAILABLE");
            sqlx::query("INSERT INTO finance.audit_delivery_attempts(id,audit_id,source_digest,outcome) VALUES($1,$2,$3,$4)")
                .bind(Uuid::new_v4()).bind(id).bind(fingerprint(&raw)?).bind(outcome).execute(&self.pool).await?;
            failed |= outcome != "NATIVE_RECEIPT_CONFIRMED";
        }
        ensure!(
            !failed,
            "One or more Finance audit delivery attempts lacked a confirmed native receipt at observation"
        );
        Ok(())
    }
    pub async fn audit_delivery(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<AuditDeliveryItem> {
        self.authorize_staff(actor, false).await?;
        self.audit_delivery_item(id).await
    }
    async fn audit_delivery_item(&self, id: Uuid) -> Result<AuditDeliveryItem> {
        let row=sqlx::query("SELECT p.record,p.delivered_at,finance.audit_delivery_receipt(p.id) receipt,a.outcome,a.source_digest attempt_digest,a.attempted_at FROM finance.audit_pending p LEFT JOIN LATERAL (SELECT outcome,source_digest,attempted_at FROM finance.audit_delivery_attempts WHERE audit_id=p.id ORDER BY attempted_at DESC,id DESC LIMIT 1) a ON true WHERE p.id=$1")
            .bind(id).fetch_one(&self.pool).await?;
        let raw: Value = row.get("record");
        let receipt = row
            .get::<Option<Value>, _>("receipt")
            .map(serde_json::from_value::<AuditDeliveryReceipt>)
            .transpose()?;
        let digest = fingerprint(&raw)?;
        let attempt = row
            .get::<Option<String>, _>("outcome")
            .map(|outcome| AuditDeliveryAttempt {
                outcome,
                source_digest: row.get("attempt_digest"),
                attempted_at: row.get("attempted_at"),
            });
        if let Some(a) = &attempt {
            ensure!(a.source_digest == digest, AuditDeliveryConflict);
            ensure!(
                a.outcome != "NATIVE_RECEIPT_CONFIRMED" || receipt.is_some(),
                AuditDeliveryConflict
            );
        }
        let record = serde_json::from_value::<AuditRecord>(raw.clone())
            .ok()
            .filter(|r| fingerprint(r).ok() == Some(digest.clone()));
        let resource_id = record.as_ref().and_then(|r| r.resource.parse().ok());
        let action = record
            .as_ref()
            .map(|r| r.action.as_str())
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
            .map(str::to_owned);
        Ok(AuditDeliveryItem {
            id,
            source_digest: digest,
            resource_id,
            action,
            delivered_at: row.get("delivered_at"),
            state: if receipt.is_some() {
                "DELIVERED"
            } else if attempt.is_some() {
                "PENDING_RETRY"
            } else {
                "PENDING"
            }
            .into(),
            receipt,
            last_attempt: attempt,
        })
    }
    pub async fn audit_delivery_status(&self, actor: &ActorContext) -> Result<AuditDeliveryStatus> {
        self.authorize_staff(actor, false).await?;
        let row=sqlx::query("SELECT clock_timestamp() as_of,(SELECT count(*)::text FROM finance.audit_pending WHERE delivered_at IS NULL) pending,(SELECT count(*)::text FROM finance.audit_delivery_receipts) confirmed,last_id,scanned_at FROM finance.audit_relay_cursor WHERE id")
            .fetch_one(&self.pool).await?;
        let ids:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM finance.audit_pending ORDER BY (delivered_at IS NULL) DESC,id LIMIT 100").fetch_all(&self.pool).await?;
        let mut items = vec![];
        for id in ids {
            items.push(self.audit_delivery_item(id).await?);
        }
        Ok(AuditDeliveryStatus {
            as_of: row.get("as_of"),
            pending_count: row.get("pending"),
            confirmed_count: row.get("confirmed"),
            cursor: row.get("last_id"),
            last_scanned_at: row.get("scanned_at"),
            items,
        })
    }
}
fn confirmation_error(e: &anyhow::Error) -> &'static str {
    if matches!(e.downcast_ref::<sqlx::Error>(),Some(sqlx::Error::Database(d)) if d.code().as_deref()==Some("P0001"))
    {
        "AUDIT_IDENTITY_CONFLICT"
    } else {
        "CONFIRMATION_UNAVAILABLE"
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditDeliveryReceipt {
    pub audit_id: Uuid,
    pub controls_digest: String,
    pub controls_sequence: String,
    pub confirmed_at: DateTime<Utc>,
    pub origin: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditDeliveryAttempt {
    pub outcome: String,
    pub source_digest: String,
    pub attempted_at: DateTime<Utc>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditDeliveryItem {
    pub id: Uuid,
    pub source_digest: String,
    pub resource_id: Option<Uuid>,
    pub action: Option<String>,
    pub state: String,
    pub delivered_at: Option<DateTime<Utc>>,
    pub receipt: Option<AuditDeliveryReceipt>,
    pub last_attempt: Option<AuditDeliveryAttempt>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditDeliveryStatus {
    pub as_of: DateTime<Utc>,
    pub pending_count: String,
    pub confirmed_count: String,
    pub cursor: Uuid,
    pub last_scanned_at: Option<DateTime<Utc>>,
    pub items: Vec<AuditDeliveryItem>,
}
