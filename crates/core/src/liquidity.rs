//! WP-40 approved settlement prefunding. Finance proposes, confirms and approves the action; this command books it once
//! by its execution id after `core.execute_liquidity_action` re-checks the exact details, the independent people, the
//! recent approval and the approved per-action and daily limits. A replay returns the original batch.
use crate::{Core, corrections::CorrectionCommands};
use anyhow::Result;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

impl CorrectionCommands {
    pub async fn execute_liquidity_action(
        &self,
        details: &Value,
        digest: &str,
        maker: Uuid,
        checker: Uuid,
    ) -> Result<(Uuid, bool)> {
        let execution: Uuid = details["executionId"]
            .as_str()
            .unwrap_or_default()
            .parse()?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM core.execute_liquidity_action($1,$2,$3,$4)")
            .bind(details)
            .bind(digest)
            .bind(maker)
            .bind(checker)
            .fetch_one(&mut *tx)
            .await?;
        let batch: Uuid = row.get("batch_id");
        let created: bool = row.get("created");
        if created {
            Core::new(self.pool.clone())
                .posted(&mut tx, batch, execution)
                .await?;
        }
        tx.commit().await?;
        Ok((batch, created))
    }
}
