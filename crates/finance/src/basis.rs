//! Core-observed periodic basis and durable audit recovery. No financial command exists here.
use crate::Finance;
use anyhow::{Result, ensure};
use bank_controls::audit::AuditRecord;
use bank_core::accruals::basis::{
    BasisArchive, BasisDetail, BasisInput, BasisInputError, BasisPage,
};
use bank_platform::domain::ActorContext;
use uuid::Uuid;
impl Finance {
    async fn basis_audit(&self, archive: &BasisArchive) -> Result<()> {
        let mut record = AuditRecord::new(
            &archive.actor,
            "finance.periodic-basis.captured",
            archive.detail.basis.id,
            archive.detail.basis.id,
        );
        record.occurred_at = archive.detail.basis.core_as_of.parse()?;
        sqlx::query(
            "INSERT INTO finance.audit_pending(id,record) VALUES($1,$2) ON CONFLICT(id) DO NOTHING",
        )
        .bind(archive.detail.basis.id)
        .bind(serde_json::to_value(record)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    pub async fn capture_periodic_basis(
        &self,
        actor: &ActorContext,
        key: Uuid,
        input: &BasisInput,
    ) -> Result<BasisDetail> {
        self.authorize_staff(actor, true).await?;
        let archive = self
            .bases
            .capture(&self.controls, actor, key, input)
            .await?;
        self.basis_audit(&archive).await?;
        self.authorize_staff(actor, false).await?;
        Ok(archive.detail)
    }
    pub async fn periodic_basis(&self, actor: &ActorContext, id: Uuid) -> Result<BasisDetail> {
        self.authorize_staff(actor, false).await?;
        Ok(self.bases.get(id).await?.detail)
    }
    pub async fn periodic_bases(
        &self,
        actor: &ActorContext,
        policy: Option<Uuid>,
        account: Option<Uuid>,
    ) -> Result<Vec<BasisDetail>> {
        self.authorize_staff(actor, false).await?;
        let mut out = vec![];
        for id in self.bases.ids(policy, account).await? {
            out.push(self.bases.get(id).await?.detail);
        }
        Ok(out)
    }
    pub async fn periodic_basis_items(
        &self,
        actor: &ActorContext,
        id: Uuid,
        after: u32,
        limit: u32,
    ) -> Result<BasisPage> {
        self.authorize_staff(actor, false).await?;
        ensure!((1..=100).contains(&limit), BasisInputError);
        let archive = self.bases.get(id).await?;
        ensure!(after <= archive.detail.basis.source_count, BasisInputError);
        let items = archive
            .items
            .into_iter()
            .skip(after as usize)
            .take(limit as usize)
            .collect::<Vec<_>>();
        let last = items.last().map(|i| i.ordinal).unwrap_or(after);
        Ok(BasisPage {
            basis_id: id,
            digest: archive.detail.digest,
            items,
            next_after: (last < archive.detail.basis.source_count).then_some(last),
        })
    }
    pub async fn recover_basis_audit(&self) -> Result<()> {
        let after: Uuid =
            sqlx::query_scalar("SELECT last_id FROM finance.basis_audit_cursor WHERE id")
                .fetch_one(&self.pool)
                .await?;
        let ids = self.bases.audit_ids(after).await?;
        sqlx::query("UPDATE finance.basis_audit_cursor SET last_id=$1 WHERE id")
            .bind(ids.last().copied().unwrap_or(Uuid::nil()))
            .execute(&self.pool)
            .await?;
        // Advance even past damaged/unavailable old artifacts; the cursor wraps to retry them.
        for id in ids {
            if let Ok(archive) = self.bases.get(id).await {
                let _ = self.basis_audit(&archive).await;
            }
        }
        Ok(())
    }
}
