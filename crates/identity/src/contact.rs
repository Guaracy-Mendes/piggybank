//! UI-03 local template: email ownership by one-time code. The destination is the email the issuer holds for the
//! signed-in customer (read with the least-privilege `bank-identity-admin` client); the code is generated here,
//! stored only as a salted digest and delivered to the simulator's synthetic outbox (`simulator.contact_outbox`).
//! No real email or SMS is sent. A matching code marks the email verified at the issuer and is audited.
use crate::sessions::IdentityService;
use anyhow::{Context, Result, anyhow, ensure};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactVerification {
    pub channel: &'static str,
    pub delivery: &'static str,
    pub destination: String,
    pub verified: bool,
    pub pending: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_left: Option<i64>,
}

/// Errors a client can act on; everything else is a dependency failure.
#[derive(Debug)]
pub enum ContactError {
    AlreadyVerified,
    ResendTooSoon,
    TooManyCodes,
    NoEmail,
}
impl std::fmt::Display for ContactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AlreadyVerified => "Email is already verified",
            Self::ResendTooSoon => "A new code can be sent 45 seconds after the last one",
            Self::TooManyCodes => "Too many codes this hour",
            Self::NoEmail => "No email is held for this sign-in",
        })
    }
}
impl std::error::Error for ContactError {}

/// `a•••@example.invalid`: enough to recognise the address without disclosing it.
pub fn mask(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) if !local.is_empty() => {
            let first: String = local.chars().take(1).collect();
            format!("{first}•••@{domain}")
        }
        _ => "•••".into(),
    }
}

/// Uniform 6-digit code by rejection sampling over OS entropy.
fn code() -> String {
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).expect("operating system entropy unavailable");
        let n = u32::from_le_bytes(bytes);
        // 4_294_000_000 is the largest multiple of 1_000_000 below u32::MAX.
        if n < 4_294_000_000 {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

impl IdentityService {
    async fn issuer_user(&self, subject: &str) -> Result<(String, Value, String)> {
        let id = uuid::Uuid::parse_str(subject).context("issuer subject must be a user id")?;
        let token = self.admin_token().await?;
        let url = format!(
            "{}/admin/realms/customers/users/{id}",
            self.verifier.issuer_base
        );
        let user: Value = self
            .verifier
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await?
            .error_for_status()
            .context("issuer user lookup failed")?
            .json()
            .await?;
        Ok((url, user, token))
    }
    async fn status(
        &self,
        issuer: &str,
        subject: &str,
        user: &Value,
    ) -> Result<ContactVerification> {
        let email = user["email"].as_str().unwrap_or_default();
        let verified = user["emailVerified"].as_bool().unwrap_or(false);
        let pending: Option<Value> = if verified {
            None
        } else {
            sqlx::query_scalar("SELECT identity.contact_code_status($1,$2)")
                .bind(issuer)
                .bind(subject)
                .fetch_one(&self.pool)
                .await?
        };
        Ok(ContactVerification {
            channel: "EMAIL",
            delivery: "SIMULATED_OUTBOX",
            destination: mask(email),
            verified,
            pending,
            result: None,
            attempts_left: None,
        })
    }
    pub async fn contact_verification(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<ContactVerification> {
        let (_, user, _) = self.issuer_user(subject).await?;
        self.status(issuer, subject, &user).await
    }
    pub async fn send_contact_code(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<ContactVerification> {
        let (_, user, _) = self.issuer_user(subject).await?;
        if user["emailVerified"].as_bool().unwrap_or(false) {
            return Err(ContactError::AlreadyVerified.into());
        }
        let email = user["email"]
            .as_str()
            .filter(|e| e.contains('@'))
            .ok_or(ContactError::NoEmail)?;
        let issued = sqlx::query("SELECT identity.contact_code_issue($1,$2,$3,$4)")
            .bind(issuer)
            .bind(subject)
            .bind(email)
            .bind(code())
            .execute(&self.pool)
            .await;
        if let Err(sqlx::Error::Database(e)) = &issued {
            match e.message() {
                "RESEND_TOO_SOON" => return Err(ContactError::ResendTooSoon.into()),
                "TOO_MANY_CODES" => return Err(ContactError::TooManyCodes.into()),
                _ => {}
            }
        }
        issued?;
        self.status(issuer, subject, &user).await
    }
    pub async fn confirm_contact_code(
        &self,
        issuer: &str,
        subject: &str,
        candidate: &str,
    ) -> Result<ContactVerification> {
        ensure!(
            candidate.len() == 6 && candidate.bytes().all(|b| b.is_ascii_digit()),
            "A 6-digit code is required"
        );
        let (url, mut user, token) = self.issuer_user(subject).await?;
        if user["emailVerified"].as_bool().unwrap_or(false) {
            return Err(ContactError::AlreadyVerified.into());
        }
        let checked: Value = sqlx::query_scalar("SELECT identity.contact_code_check($1,$2,$3)")
            .bind(issuer)
            .bind(subject)
            .bind(candidate)
            .fetch_one(&self.pool)
            .await?;
        let result = checked["result"].as_str().unwrap_or("EXPIRED").to_owned();
        if result == "VERIFIED" {
            // The code was sent to this exact address; refuse if the issuer's email changed meanwhile.
            ensure!(
                checked["destination"] == user["email"],
                "Email changed since the code was sent; request a new code"
            );
            user["emailVerified"] = Value::Bool(true);
            self.verifier
                .http
                .put(&url)
                .bearer_auth(&token)
                .json(&user)
                .send()
                .await?
                .error_for_status()
                .map_err(|e| anyhow!("issuer email verification failed: {e}"))?;
            sqlx::query("INSERT INTO identity.audit(id,issuer,subject,action) VALUES($1,$2,$3,'contact.email-verified')")
                .bind(uuid::Uuid::new_v4())
                .bind(issuer)
                .bind(subject)
                .execute(&self.pool)
                .await?;
        }
        let mut status = self.status(issuer, subject, &user).await?;
        status.attempts_left = checked["attemptsLeft"].as_i64();
        status.result = Some(result);
        Ok(status)
    }
}
