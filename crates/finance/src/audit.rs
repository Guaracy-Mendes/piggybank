use crate::Finance;
use anyhow::Result;
use bank_controls::audit::{AuditLog, AuditRecord};
use bank_platform::domain::ActorContext;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
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
    pub async fn relay_audit(&self) -> Result<()> {
        let rows = sqlx::query("SELECT id,record FROM finance.audit_pending WHERE delivered_at IS NULL ORDER BY id LIMIT 50").fetch_all(&self.pool).await?;
        for row in rows {
            let id: Uuid = row.get("id");
            let record: AuditRecord = serde_json::from_value(row.get("record"))?;
            AuditLog::new(self.controls.pool.clone())
                .append(id, "finance", &record)
                .await?;
            sqlx::query("UPDATE finance.audit_pending SET delivered_at=now() WHERE id=$1 AND delivered_at IS NULL").bind(id).execute(&self.pool).await?;
        }
        Ok(())
    }
}
