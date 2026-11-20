//! WP-28 recovery executor. Uses the least-privilege `bank-identity-admin` service client (manage/view users in
//! the customers realm only). Its secret is read from a file on every use, so rotation needs no restart, and it
//! is never logged. The executor revokes every issuer session (browser and native refresh tokens) and, for a
//! credential reset, requires a new password at the next sign-in. Staff never see or set credentials.
use crate::{oidc::Trust, sessions::IdentityService};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Token {
    access_token: String,
}
fn secret() -> Result<String> {
    if let Ok(path) = std::env::var("BANK_IDENTITY_ADMIN_SECRET_FILE") {
        let value =
            std::fs::read_to_string(&path).context("identity admin secret file unreadable")?;
        let value = value.trim().to_owned();
        ensure!(!value.is_empty(), "identity admin secret file is empty");
        return Ok(value);
    }
    Ok(std::env::var("BANK_IDENTITY_ADMIN_SECRET")
        .unwrap_or_else(|_| "synthetic-identity-admin-only".into()))
}
impl IdentityService {
    pub(crate) async fn admin_token(&self) -> Result<String> {
        let issuer = self.verifier.issuer(Trust::Customer);
        let token: Token = self
            .verifier
            .http
            .post(format!("{issuer}/protocol/openid-connect/token"))
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", "bank-identity-admin"),
                ("client_secret", secret()?.as_str()),
            ])
            .send()
            .await?
            .error_for_status()
            .context("identity admin authentication failed")?
            .json()
            .await?;
        Ok(token.access_token)
    }
    /// Revokes all issuer sessions for a customer subject and deletes server-held browser sessions.
    pub async fn revoke_customer(
        &self,
        subject: &str,
        require_new_password: bool,
    ) -> Result<Value> {
        let subject = uuid::Uuid::parse_str(subject).context("issuer subject must be a user id")?;
        let token = self.admin_token().await?;
        let users = format!(
            "{}/admin/realms/customers/users/{subject}",
            self.verifier.issuer_base
        );
        self.verifier
            .http
            .post(format!("{users}/logout"))
            .bearer_auth(&token)
            .send()
            .await?
            .error_for_status()
            .context("issuer logout failed")?;
        if require_new_password {
            let mut user: Value = self
                .verifier
                .http
                .get(&users)
                .bearer_auth(&token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let mut actions: Vec<Value> = user["requiredActions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !actions.iter().any(|a| a == "UPDATE_PASSWORD") {
                actions.push(json!("UPDATE_PASSWORD"));
            }
            user["requiredActions"] = Value::Array(actions);
            self.verifier
                .http
                .put(&users)
                .bearer_auth(&token)
                .json(&user)
                .send()
                .await?
                .error_for_status()
                .context("issuer required action failed")?;
        }
        let issuer = self.verifier.issuer(Trust::Customer);
        let mut tx = self.pool.begin().await?;
        let browser = sqlx::query("DELETE FROM identity.sessions WHERE issuer=$1 AND subject=$2")
            .bind(&issuer)
            .bind(subject.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        sqlx::query("DELETE FROM identity.native_step_ups WHERE issuer=$1 AND subject=$2")
            .bind(&issuer)
            .bind(subject.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO identity.subjects(issuer,subject,trust) VALUES($1,$2,'customer') ON CONFLICT DO NOTHING")
            .bind(&issuer)
            .bind(subject.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO identity.audit(id,issuer,subject,action) VALUES($1,$2,$3,$4)")
            .bind(uuid::Uuid::new_v4())
            .bind(&issuer)
            .bind(subject.to_string())
            .bind(if require_new_password {
                "recovery.credential-reset"
            } else {
                "recovery.sessions-revoked"
            })
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(
            json!({"issuerSessionsRevoked": true, "browserSessionsDeleted": browser, "passwordUpdateRequired": require_new_password}),
        )
    }
}
