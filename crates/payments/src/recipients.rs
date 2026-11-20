use crate::{Payments, types::Beneficiary};
use anyhow::{Result, ensure};
use bank_platform::domain::{ActorContext, TrustContext, fingerprint};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;
impl Payments {
    pub async fn add_beneficiary(
        &self,
        actor: &ActorContext,
        key: Uuid,
        label: &str,
        reference: &str,
    ) -> Result<Beneficiary> {
        self.add_rail_beneficiary(actor, key, label, reference, "INTERNAL")
            .await
    }
    pub async fn add_rail_beneficiary(
        &self,
        actor: &ActorContext,
        key: Uuid,
        label: &str,
        reference: &str,
        rail: &str,
    ) -> Result<Beneficiary> {
        ensure!(matches!(rail, "INTERNAL" | "SIMULATED"), "Unsupported rail");
        ensure!(
            actor.trust == TrustContext::Customer
                && !label.trim().is_empty()
                && label == label.trim()
                && label.len() <= 80
                && !label.chars().any(char::is_control),
            "Invalid synthetic beneficiary"
        );
        self.controls.person(actor).await?;
        let digest = if rail == "INTERNAL" {
            fingerprint(&(label, reference))?
        } else {
            fingerprint(&(label, reference, rail))?
        };
        if let Some(row)=sqlx::query("SELECT * FROM payments.beneficiaries WHERE issuer=$1 AND subject=$2 AND request_key=$3").bind(&actor.issuer).bind(&actor.subject).bind(key).fetch_optional(&self.pool).await? {
   ensure!(row.get::<String,_>("fingerprint")==digest,"Beneficiary request identity conflict");return Ok(record(&row));
  }
        let payee = if rail == "INTERNAL" {
            self.core.resolve_receiver(reference).await?
        } else {
            self.rail_client.verify_recipient(reference).await?;
            bank_platform::domain::AccountId(crate::policy::SIMULATED_CLEARING)
        };
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        let created=sqlx::query("INSERT INTO payments.beneficiaries(id,issuer,subject,request_key,label,receiving_reference,payee_account_id,state,fingerprint,rail) VALUES($1,$2,$3,$4,$5,$6,$7,'VERIFIED',$8,$9) ON CONFLICT(issuer,subject,request_key) DO NOTHING RETURNING id")
   .bind(id).bind(&actor.issuer).bind(&actor.subject).bind(key).bind(label).bind(reference).bind(payee.0).bind(&digest).bind(rail).fetch_optional(&mut *tx).await?;
        if created.is_some() {
            Self::audit_in(&mut tx, actor, "beneficiary.verified", id).await?;
        }
        let row=sqlx::query("SELECT * FROM payments.beneficiaries WHERE issuer=$1 AND subject=$2 AND request_key=$3").bind(&actor.issuer).bind(&actor.subject).bind(key).fetch_one(&mut *tx).await?;
        ensure!(
            row.get::<String, _>("fingerprint") == digest,
            "Beneficiary request identity conflict"
        );
        tx.commit().await?;
        Ok(record(&row))
    }
    pub async fn beneficiaries(&self, actor: &ActorContext) -> Result<Vec<Beneficiary>> {
        // A consented partner (payments:initiate) may list, never add or revoke, the customer's verified recipients.
        ensure!(
            matches!(actor.trust, TrustContext::Customer | TrustContext::Partner),
            "Customer required"
        );
        Ok(sqlx::query("SELECT * FROM payments.beneficiaries WHERE issuer=$1 AND subject=$2 ORDER BY verified_at DESC,id LIMIT 100").bind(&actor.issuer).bind(&actor.subject).fetch_all(&self.pool).await?.iter().map(record).collect())
    }
    pub async fn revoke_beneficiary(&self, actor: &ActorContext, id: Uuid) -> Result<()> {
        ensure!(actor.trust == TrustContext::Customer, "Customer required");
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("SELECT state FROM payments.beneficiaries WHERE id=$1 AND issuer=$2 AND subject=$3 FOR UPDATE").bind(id).bind(&actor.issuer).bind(&actor.subject).fetch_one(&mut *tx).await?;
        if row.get::<String, _>("state") != "REVOKED" {
            sqlx::query(
                "UPDATE payments.beneficiaries SET state='REVOKED',version=version+1 WHERE id=$1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            Self::audit_in(&mut tx, actor, "beneficiary.revoked", id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn beneficiary(
        &self,
        actor: &ActorContext,
        id: Uuid,
    ) -> Result<(Beneficiary, bank_platform::domain::AccountId)> {
        let row = sqlx::query(
            "SELECT * FROM payments.beneficiaries WHERE id=$1 AND issuer=$2 AND subject=$3",
        )
        .bind(id)
        .bind(&actor.issuer)
        .bind(&actor.subject)
        .fetch_one(&self.pool)
        .await?;
        Ok((
            record(&row),
            bank_platform::domain::AccountId(row.get("payee_account_id")),
        ))
    }
}
pub(crate) fn record(row: &PgRow) -> Beneficiary {
    Beneficiary {
        id: row.get("id"),
        label: row.get("label"),
        state: row.get("state"),
        version: row.get("version"),
        receiving_reference: row.get("receiving_reference"),
        rail: row.get("rail"),
        verified_at: row.get("verified_at"),
    }
}
