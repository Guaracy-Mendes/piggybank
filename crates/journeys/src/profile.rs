use crate::*;
use bank_core::profile::ContactInput;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactStatus {
    pub id: Uuid,
    pub state: String,
    pub operation_id: Option<Uuid>,
}
impl<T: BlobStore> Journeys<T> {
    pub async fn propose_contact(
        &self,
        actor: &ActorContext,
        key: &str,
        email: String,
    ) -> Result<ContactStatus> {
        ensure!(identifier(key), "Valid contact request key required");
        let profile = self.core.profile(actor, &self.controls).await?;
        let input = ContactInput {
            party_id: profile.party_id,
            email: email.clone(),
            expected_version: profile.version,
        };
        let digest = input.digest()?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO journeys.contact_changes(id,issuer,subject,key,actor_context,email,expected_version,input_digest) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(issuer,subject,key) DO NOTHING").bind(id).bind(&actor.issuer).bind(&actor.subject).bind(key).bind(serde_json::to_value(actor)?).bind(email).bind(input.expected_version).bind(digest).execute(&self.pool).await?;
        let row=sqlx::query("SELECT id,email FROM journeys.contact_changes WHERE issuer=$1 AND subject=$2 AND key=$3").bind(&actor.issuer).bind(&actor.subject).bind(key).fetch_one(&self.pool).await?;
        ensure!(
            row.get::<String, _>("email") == input.email,
            "Contact request key changed details"
        );
        let id = row.get("id");
        self.schedule_contact(id).await?;
        self.contact_status(actor, id).await
    }
    pub async fn contact_status(&self, actor: &ActorContext, id: Uuid) -> Result<ContactStatus> {
        let row=sqlx::query("SELECT id,state,operation_id FROM journeys.contact_changes WHERE id=$1 AND issuer=$2 AND subject=$3").bind(id).bind(&actor.issuer).bind(&actor.subject).fetch_one(&self.pool).await?;
        Ok(ContactStatus {
            id,
            state: row.get("state"),
            operation_id: row.get("operation_id"),
        })
    }
    pub async fn schedule_contact(&self, id: Uuid) -> Result<()> {
        let row = sqlx::query("SELECT * FROM journeys.contact_changes WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
        let operation = self
            .operations
            .accept(
                &actor,
                "profile.contact",
                &format!("contact-{id}"),
                &json!({"changeId":id,"digest":row.get::<String,_>("input_digest")}),
                JobSpec {
                    kind: "profile.contact".into(),
                    step_key: "initial".into(),
                    payload: json!({"changeId":id}),
                    delay_seconds: 0,
                },
            )
            .await?;
        sqlx::query("UPDATE journeys.contact_changes SET operation_id=$2 WHERE id=$1 AND (operation_id IS NULL OR operation_id=$2)").bind(id).bind(operation.id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn recover_contacts(&self) -> Result<()> {
        let rows=sqlx::query("SELECT id,operation_id,actor_context,input_digest,state,expected_version FROM journeys.contact_changes WHERE state IN ('PENDING','REVIEW') ORDER BY created_at LIMIT 50").fetch_all(&self.pool).await?;
        for row in rows {
            let id: Uuid = row.get("id");
            if let Some(operation) = row.get::<Option<Uuid>, _>("operation_id") {
                let actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
                if self.operations.status(&actor, operation).await?.state == "REVIEW_REQUIRED" {
                    self.own_contact_exception(id, operation).await?;
                    let gate = self
                        .checks
                        .evaluate(
                            id,
                            Purpose::Onboarding,
                            &row.get::<String, _>("input_digest"),
                        )
                        .await?;
                    if gate.outcome == Outcome::Allow
                        && self.core.profile(&actor, &self.controls).await?.version
                            != row.get::<i64, _>("expected_version")
                    {
                        continue;
                    }
                    if matches!(gate.outcome, Outcome::Allow | Outcome::Deny) {
                        self.operations
                            .resume_review(
                                &actor,
                                operation,
                                JobSpec {
                                    kind: "profile.contact".into(),
                                    step_key: format!("verified-{}", gate.revision),
                                    payload: json!({"changeId":id}),
                                    delay_seconds: 0,
                                },
                            )
                            .await?;
                    }
                }
            } else {
                self.schedule_contact(id).await?;
            }
        }
        Ok(())
    }
    pub async fn own_contact_exception(&self, id: Uuid, operation: Uuid) -> Result<()> {
        let actor: serde_json::Value =
            sqlx::query_scalar("SELECT actor_context FROM journeys.contact_changes WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let initiator: ActorContext = serde_json::from_value(actor)?;
        let worker = ActorContext {
            issuer: "urn:piggybank:worker".into(),
            subject: "contact-verification".into(),
            trust: TrustContext::Service,
            client_id: "bank-worker".into(),
            person_id: None,
            represented_party: None,
            roles: vec![],
        };
        bank_controls::cases::Cases::new(self.controls.pool.clone())
            .ensure(
                &worker,
                bank_controls::cases::NewCase {
                    kind: bank_controls::cases::CaseKind::Support,
                    origin_id: operation,
                    resource_id: id,
                    evidence_refs: vec![],
                    customer: Some((initiator.issuer, initiator.subject)),
                },
            )
            .await?;
        Ok(())
    }
    pub async fn process_contact(&self, id: Uuid) -> Result<Progress> {
        let row = sqlx::query("SELECT * FROM journeys.contact_changes WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        let actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
        let profile = self.core.profile(&actor, &self.controls).await?;
        if self
            .core
            .contact_result(&actor, &self.controls, id)
            .await?
            .is_some()
        {
            self.contact_state(id, "VERIFIED").await?;
            return Ok(Progress::Done {
                state: "CONTACT_VERIFIED".into(),
                account_id: None,
            });
        }
        let input = ContactInput {
            party_id: profile.party_id,
            email: row.get("email"),
            expected_version: row.get("expected_version"),
        };
        let digest = input.digest()?;
        ensure!(
            digest == row.get::<String, _>("input_digest"),
            "Contact intent changed"
        );
        if self
            .checks
            .evaluate(id, Purpose::Onboarding, &digest)
            .await?
            .outcome
            == Outcome::Deny
        {
            self.contact_state(id, "DENIED").await?;
            return Ok(Progress::Done {
                state: "CONTACT_DENIED".into(),
                account_id: None,
            });
        }
        if profile.version != input.expected_version {
            self.contact_state(id, "REVIEW").await?;
            return Ok(Progress::Review {
                code: "PROFILE_VERSION_CHANGED",
            });
        }
        for kind in Purpose::Onboarding.required() {
            self.checks.request(&actor, id, *kind, &digest).await?;
        }
        let gates = self
            .checks
            .evaluate(id, Purpose::Onboarding, &digest)
            .await?;
        match gates.outcome {
            Outcome::Pending => Ok(Progress::Pending),
            Outcome::Review => {
                self.contact_state(id, "REVIEW").await?;
                Ok(Progress::Review {
                    code: "CONTACT_REVIEW",
                })
            }
            Outcome::Deny => {
                self.contact_state(id, "DENIED").await?;
                Ok(Progress::Done {
                    state: "CONTACT_DENIED".into(),
                    account_id: None,
                })
            }
            Outcome::Allow => {
                self.core
                    .apply_contact(&actor, &self.controls, &self.checks, id, &input)
                    .await?;
                self.contact_state(id, "VERIFIED").await?;
                Ok(Progress::Done {
                    state: "CONTACT_VERIFIED".into(),
                    account_id: None,
                })
            }
        }
    }
    async fn contact_state(&self, id: Uuid, state: &str) -> Result<()> {
        sqlx::query("UPDATE journeys.contact_changes SET state=$2 WHERE id=$1")
            .bind(id)
            .bind(state)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
