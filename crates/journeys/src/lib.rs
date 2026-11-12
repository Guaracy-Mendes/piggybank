//! Durable synthetic onboarding coordinator. Every authority retains its own transaction boundary.
pub mod profile;
use anyhow::{Result, ensure};
use bank_controls::{
    checks::{Checks, Outcome, Purpose},
    entitlements::{Action, Entitlements},
    evidence::{BlobStore, EvidenceService},
};
use bank_core::{
    Core,
    accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest},
};
use bank_documents::Terms;
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, EventEnvelope, EventId, TrustContext, fingerprint},
    events::EventStore,
    operations::{JobSpec, OperationStatus, Operations},
};
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicantProfile {
    pub full_name: String,
    pub email: String,
    pub date_of_birth: NaiveDate,
    pub residency: String,
    pub consent_version: String,
    pub marketing: bool,
}
impl ApplicantProfile {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.full_name.trim().is_empty()
                && self.full_name.len() <= 100
                && self.email.len() <= 128
                && self.email.ends_with("@example.invalid")
                && self.email.bytes().filter(|b| *b == b'@').count() == 1
                && !self.email.starts_with('@')
                && self.residency == "SYNTHETIC"
                && self.consent_version == "synthetic-consent-v1"
                && self.age().is_some_and(|a| a <= 130),
            "Complete synthetic identity, contact and consent required"
        );
        Ok(())
    }
    pub fn age(&self) -> Option<u8> {
        Utc::now()
            .date_naive()
            .years_since(self.date_of_birth)
            .and_then(|a| a.try_into().ok())
    }
    pub fn decision_digest(&self) -> Result<String> {
        let mut profile = self.clone();
        profile.marketing = false;
        Ok(fingerprint(&profile)?)
    }
    pub fn identity_digest(&self) -> Result<String> {
        Ok(fingerprint(
            &json!({"fullName":self.full_name,"dateOfBirth":self.date_of_birth,"residency":self.residency}),
        )?)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub id: Uuid,
    pub product_version_id: Uuid,
    pub terms_id: Uuid,
    pub acceptance_id: Option<Uuid>,
    pub profile: Option<ApplicantProfile>,
    pub evidence_id: Option<Uuid>,
    pub state: String,
    pub next_action: String,
    pub version: i64,
    pub generation: i32,
    pub operation_id: Option<Uuid>,
    pub account_id: Option<AccountId>,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    product_version_id: Uuid,
    terms_id: Uuid,
    acceptance_id: Uuid,
    profile: ApplicantProfile,
    evidence_id: Uuid,
    input_digest: String,
}
#[derive(Serialize)]
pub enum Progress {
    Done {
        state: String,
        account_id: Option<AccountId>,
    },
    Pending,
    Review {
        code: &'static str,
    },
}
#[derive(Clone)]
pub struct Journeys<T: BlobStore> {
    pub pool: PgPool,
    pub core: Core,
    pub checks: Checks,
    pub controls: Entitlements,
    pub terms: Terms,
    pub operations: Operations,
    pub evidence: Arc<EvidenceService<T>>,
}
impl<T: BlobStore> Journeys<T> {
    pub async fn create(
        &self,
        actor: &ActorContext,
        key: &str,
        product: Uuid,
        marketing: bool,
        attribution: Option<String>,
    ) -> Result<Application> {
        ensure!(
            actor.trust == TrustContext::Customer && identifier(key),
            "Customer context and valid key required"
        );
        ensure!(
            attribution.as_ref().is_none_or(|s| identifier(s))
                && (marketing || attribution.is_none()),
            "Attribution requires explicit marketing purpose"
        );
        let current = self.core.product(product).await?;
        ensure!(
            current.effective_from <= Utc::now().date_naive()
                && current.effective_until > Utc::now().date_naive(),
            "Offer is not current"
        );
        let terms = self.terms.current(product, Utc::now().date_naive()).await?;
        let digest = fingerprint(&(product, marketing, &attribution))?;
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4();
        let created=sqlx::query("INSERT INTO journeys.applications(id,issuer,subject,create_key,create_digest,product_version_id,terms_id,marketing,attribution) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(issuer,subject,create_key) DO NOTHING RETURNING id").bind(id).bind(&actor.issuer).bind(&actor.subject).bind(key).bind(&digest).bind(product).bind(terms.id).bind(marketing).bind(attribution).fetch_optional(&mut *tx).await?;
        let row=sqlx::query("SELECT id,create_digest FROM journeys.applications WHERE issuer=$1 AND subject=$2 AND create_key=$3").bind(&actor.issuer).bind(&actor.subject).bind(key).fetch_one(&mut *tx).await?;
        ensure!(
            row.get::<String, _>("create_digest") == digest,
            "Application idempotency conflict"
        );
        let id = row.get("id");
        if created.is_some() {
            self.fact(&mut tx, id, "APPLICATION_CREATED").await?;
        }
        tx.commit().await?;
        self.get(actor, id).await
    }
    pub async fn get(&self, actor: &ActorContext, id: Uuid) -> Result<Application> {
        let row=sqlx::query("SELECT a.*,s.operation_id FROM journeys.applications a LEFT JOIN journeys.submissions s ON s.application_id=a.id AND s.generation=a.generation WHERE a.id=$1 AND a.issuer=$2 AND a.subject=$3").bind(id).bind(&actor.issuer).bind(&actor.subject).fetch_one(&self.pool).await?;
        application(&row)
    }
    pub async fn list(&self, actor: &ActorContext) -> Result<Vec<Application>> {
        ensure!(
            actor.trust == TrustContext::Customer,
            "Customer context required"
        );
        let rows=sqlx::query("SELECT a.*,s.operation_id FROM journeys.applications a LEFT JOIN journeys.submissions s ON s.application_id=a.id AND s.generation=a.generation WHERE a.issuer=$1 AND a.subject=$2 ORDER BY a.created_at DESC,a.id LIMIT 50").bind(&actor.issuer).bind(&actor.subject).fetch_all(&self.pool).await?;
        rows.iter().map(application).collect()
    }
    pub async fn save(
        &self,
        actor: &ActorContext,
        id: Uuid,
        version: i64,
        profile: ApplicantProfile,
    ) -> Result<Application> {
        profile.validate()?;
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE journeys.applications SET profile=$4,consent_version=$5,marketing=$6,attribution=CASE WHEN $6 THEN attribution ELSE NULL END,version=version+1,updated_at=now() WHERE id=$1 AND issuer=$2 AND subject=$3 AND version=$7 AND state IN ('DRAFT','INFORMATION_REQUIRED') AND expires_at>now()")
    .bind(id).bind(&actor.issuer).bind(&actor.subject).bind(serde_json::to_value(&profile)?).bind(&profile.consent_version).bind(profile.marketing).bind(version).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "Application changed or cannot be edited"
        );
        self.fact(&mut tx, id, "PROFILE_SAVED").await?;
        tx.commit().await?;
        self.get(actor, id).await
    }
    pub async fn attach_evidence(
        &self,
        actor: &ActorContext,
        id: Uuid,
        content: Vec<u8>,
    ) -> Result<Application> {
        let app = self.get(actor, id).await?;
        ensure!(
            ["DRAFT", "INFORMATION_REQUIRED"].contains(&app.state.as_str())
                && !content.is_empty()
                && content.len() <= 32768,
            "Application is not awaiting evidence"
        );
        let evidence = self.evidence.put(actor, "IDENTITY", content, id).await?;
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE journeys.applications SET evidence_id=$2,version=version+1,updated_at=now() WHERE id=$1 AND version=$3 AND state IN ('DRAFT','INFORMATION_REQUIRED')").bind(id).bind(evidence.id).bind(app.version).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "Application changed during evidence upload"
        );
        self.fact(&mut tx, id, "EVIDENCE_ATTACHED").await?;
        tx.commit().await?;
        self.get(actor, id).await
    }
    pub async fn accept_terms(
        &self,
        actor: &ActorContext,
        id: Uuid,
        terms_id: Uuid,
    ) -> Result<Application> {
        let app = self.get(actor, id).await?;
        ensure!(
            ["DRAFT", "INFORMATION_REQUIRED"].contains(&app.state.as_str()),
            "Application cannot change terms"
        );
        let current = self
            .terms
            .current(app.product_version_id, Utc::now().date_naive())
            .await?;
        ensure!(current.id == terms_id, "Current displayed terms required");
        if app.acceptance_id.is_some() && app.terms_id == terms_id {
            return Ok(app);
        }
        let acceptance = self.terms.accept(actor, terms_id, id).await?;
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE journeys.applications SET terms_id=$2,acceptance_id=$3,version=version+1,updated_at=now() WHERE id=$1 AND version=$4 AND state IN ('DRAFT','INFORMATION_REQUIRED')").bind(id).bind(terms_id).bind(acceptance.id).bind(app.version).execute(&mut *tx).await?;
        ensure!(
            changed.rows_affected() == 1,
            "Application changed during acceptance"
        );
        self.fact(&mut tx, id, "TERMS_ACCEPTED").await?;
        tx.commit().await?;
        self.get(actor, id).await
    }
    pub async fn withdraw_marketing(&self, actor: &ActorContext, id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE journeys.applications SET marketing=false,attribution=NULL,profile=CASE WHEN profile IS NULL THEN NULL ELSE jsonb_set(profile,'{marketing}','false') END,version=version+1,updated_at=now() WHERE id=$1 AND issuer=$2 AND subject=$3 AND marketing").bind(id).bind(&actor.issuer).bind(&actor.subject).execute(&mut *tx).await?;
        if changed.rows_affected() == 1 {
            self.fact(&mut tx, id, "MARKETING_WITHDRAWN").await?;
        }
        tx.commit().await?;
        self.get(actor, id).await?;
        Ok(())
    }
    pub async fn submit(
        &self,
        actor: &ActorContext,
        id: Uuid,
        key: &str,
    ) -> Result<OperationStatus> {
        ensure!(identifier(key), "Invalid submission key");
        let app = self.get(actor, id).await?;
        let mut profile = app
            .profile
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Profile required"))?;
        profile.validate()?;
        let acceptance = app
            .acceptance_id
            .ok_or_else(|| anyhow::anyhow!("Current terms acceptance required"))?;
        self.terms
            .validate_acceptance(actor, acceptance, id, app.product_version_id)
            .await?;
        let evidence = app
            .evidence_id
            .ok_or_else(|| anyhow::anyhow!("Identity evidence required"))?;
        let data = self.evidence.get(actor, evidence, id).await?;
        let input_digest =
            fingerprint(&(profile.decision_digest()?, evidence, fingerprint(&data)?))?;
        profile.marketing = false; // Optional purpose withdrawal cannot alter accepted legal work.
        let snapshot = Snapshot {
            product_version_id: app.product_version_id,
            terms_id: app.terms_id,
            acceptance_id: acceptance,
            profile,
            evidence_id: evidence,
            input_digest,
        };
        let digest = fingerprint(&snapshot)?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT state,version,generation FROM journeys.applications WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        let existing = sqlx::query(
            "SELECT generation,digest FROM journeys.submissions WHERE application_id=$1 AND key=$2",
        )
        .bind(id)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?;
        let generation = if let Some(existing) = existing {
            ensure!(
                existing.get::<String, _>("digest") == digest,
                "Submission key changed details"
            );
            existing.get("generation")
        } else {
            ensure!(
                row.get::<i64, _>("version") == app.version
                    && ["DRAFT", "INFORMATION_REQUIRED"]
                        .contains(&row.get::<String, _>("state").as_str())
                    && app.expires_at > Utc::now(),
                "Application is not ready for a new submission"
            );
            let generation = row.get::<i32, _>("generation") + 1;
            sqlx::query("INSERT INTO journeys.submissions(application_id,generation,key,digest,actor_context,snapshot) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(generation).bind(key).bind(digest).bind(serde_json::to_value(actor)?).bind(serde_json::to_value(snapshot)?).execute(&mut *tx).await?;
            sqlx::query("UPDATE journeys.applications SET generation=$2,state='SUBMITTING',next_action='AWAIT_CHECKS',version=version+1,updated_at=now() WHERE id=$1").bind(id).bind(generation).execute(&mut *tx).await?;
            self.fact(&mut tx, id, "SUBMISSION_ACCEPTED").await?;
            generation
        };
        tx.commit().await?;
        self.schedule(id, generation).await
    }
    pub async fn schedule(&self, id: Uuid, generation: i32) -> Result<OperationStatus> {
        let row = sqlx::query(
            "SELECT * FROM journeys.submissions WHERE application_id=$1 AND generation=$2",
        )
        .bind(id)
        .bind(generation)
        .fetch_one(&self.pool)
        .await?;
        let actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
        let key = format!("onboard-{id}-{generation}");
        let command = json!({"applicationId":id,"generation":generation,"snapshotDigest":row.get::<String,_>("digest")});
        let operation = self
            .operations
            .accept(
                &actor,
                "journey.onboard",
                &key,
                &command,
                JobSpec {
                    kind: "journey.onboard".into(),
                    step_key: "initial".into(),
                    payload: json!({"applicationId":id,"generation":generation}),
                    delay_seconds: 0,
                },
            )
            .await?;
        sqlx::query("UPDATE journeys.submissions SET operation_id=$3 WHERE application_id=$1 AND generation=$2 AND (operation_id IS NULL OR operation_id=$3)").bind(id).bind(generation).bind(operation.id).execute(&self.pool).await?;
        Ok(operation)
    }
    pub async fn recover_submissions(&self) -> Result<()> {
        let rows=sqlx::query("SELECT application_id,generation FROM journeys.submissions WHERE operation_id IS NULL ORDER BY created_at LIMIT 20").fetch_all(&self.pool).await?;
        for row in rows {
            self.schedule(row.get("application_id"), row.get("generation"))
                .await?;
        }
        Ok(())
    }
    pub async fn resume_ready(&self) -> Result<()> {
        let rows=sqlx::query("SELECT s.*,a.state AS application_state FROM journeys.submissions s JOIN journeys.applications a ON a.id=s.application_id AND a.generation=s.generation WHERE a.state IN ('MANUAL_REVIEW','CONTROLS_PENDING','INFORMATION_REQUIRED') AND s.operation_id IS NOT NULL ORDER BY s.created_at LIMIT 50").fetch_all(&self.pool).await?;
        for row in rows {
            let actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
            let id: Uuid = row.get("application_id");
            let operation: Uuid = row.get("operation_id");
            let snapshot: Snapshot = serde_json::from_value(row.get("snapshot"))?;
            if self.operations.status(&actor, operation).await?.state != "REVIEW_REQUIRED" {
                continue;
            }
            self.own_exception(id, row.get("generation"), operation)
                .await?;
            if row.get::<String, _>("application_state") == "INFORMATION_REQUIRED" {
                continue;
            }
            let gate = self
                .checks
                .evaluate(id, Purpose::Onboarding, &snapshot.input_digest)
                .await?;
            if matches!(gate.outcome, Outcome::Allow | Outcome::Deny) {
                if gate.outcome == Outcome::Allow {
                    self.terms
                        .validate_acceptance(
                            &actor,
                            snapshot.acceptance_id,
                            id,
                            snapshot.product_version_id,
                        )
                        .await?;
                }
                self.operations.resume_review(&actor,operation,JobSpec{kind:"journey.onboard".into(),step_key:format!("recheck-{}",gate.revision),payload:json!({"applicationId":id,"generation":row.get::<i32,_>("generation")}),delay_seconds:0}).await?;
            }
        }
        Ok(())
    }
    pub async fn own_exception(&self, id: Uuid, generation: i32, operation: Uuid) -> Result<()> {
        let actor:serde_json::Value=sqlx::query_scalar("SELECT actor_context FROM journeys.submissions WHERE application_id=$1 AND generation=$2").bind(id).bind(generation).fetch_one(&self.pool).await?;
        let initiator: ActorContext = serde_json::from_value(actor)?;
        let worker = ActorContext {
            issuer: "urn:piggybank:worker".into(),
            subject: "onboarding".into(),
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
    pub async fn process(&self, id: Uuid, generation: i32) -> Result<Progress> {
        let row=sqlx::query("SELECT actor_context,snapshot FROM journeys.submissions WHERE application_id=$1 AND generation=$2").bind(id).bind(generation).fetch_one(&self.pool).await?;
        let actor: ActorContext = serde_json::from_value(row.get("actor_context"))?;
        let snapshot: Snapshot = serde_json::from_value(row.get("snapshot"))?;
        let app = self.get(&actor, id).await?;
        if app.generation != generation {
            return Ok(Progress::Review {
                code: "APPLICATION_REVISED",
            });
        }
        if app.state == "ACTIVE" {
            return Ok(Progress::Done {
                state: "ACTIVE".into(),
                account_id: app.account_id,
            });
        }
        if app
            .profile
            .as_ref()
            .map(ApplicantProfile::decision_digest)
            .transpose()?
            != Some(snapshot.profile.decision_digest()?)
            || app.evidence_id != Some(snapshot.evidence_id)
            || app.acceptance_id != Some(snapshot.acceptance_id)
        {
            return Ok(Progress::Review {
                code: "APPLICATION_DETAILS_CHANGED",
            });
        }
        if app.expires_at <= Utc::now() {
            self.state(id, generation, "EXPIRED", "START_NEW_APPLICATION")
                .await?;
            return Ok(Progress::Review {
                code: "APPLICATION_EXPIRED",
            });
        }
        let product = self.core.product(snapshot.product_version_id).await?;
        if !product.eligible(snapshot.profile.age().unwrap_or(0), Utc::now().date_naive()) {
            self.state(id, generation, "DECLINED", "OFFER_NOT_ELIGIBLE")
                .await?;
            return Ok(Progress::Done {
                state: "DECLINED".into(),
                account_id: None,
            });
        }
        if self
            .checks
            .evaluate(id, Purpose::Onboarding, &snapshot.input_digest)
            .await?
            .outcome
            == Outcome::Deny
        {
            self.state(id, generation, "DECLINED", "CONTACT_SUPPORT")
                .await?;
            return Ok(Progress::Done {
                state: "DECLINED".into(),
                account_id: None,
            });
        }
        if let Err(error) = self
            .terms
            .validate_acceptance(
                &actor,
                snapshot.acceptance_id,
                id,
                snapshot.product_version_id,
            )
            .await
        {
            if let Some(sql_error) = error.downcast_ref::<sqlx::Error>()
                && !matches!(sql_error, sqlx::Error::RowNotFound)
            {
                return Err(error);
            }
            self.state(
                id,
                generation,
                "INFORMATION_REQUIRED",
                "ACCEPT_CURRENT_TERMS",
            )
            .await?;
            return Ok(Progress::Review {
                code: "CURRENT_TERMS_REQUIRED",
            });
        }
        self.evidence.get(&actor, snapshot.evidence_id, id).await?;
        for kind in Purpose::Onboarding.required() {
            self.checks
                .request(&actor, id, *kind, &snapshot.input_digest)
                .await?;
        }
        let gates = self
            .checks
            .evaluate(id, Purpose::Onboarding, &snapshot.input_digest)
            .await?;
        match gates.outcome {
            Outcome::Pending => {
                self.state(id, generation, "CONTROLS_PENDING", "AWAIT_CHECKS")
                    .await?;
                return Ok(Progress::Pending);
            }
            Outcome::Review => {
                self.state(id, generation, "MANUAL_REVIEW", "AWAIT_REVIEW")
                    .await?;
                return Ok(Progress::Review {
                    code: "CONTROL_REVIEW",
                });
            }
            Outcome::Deny => {
                self.state(id, generation, "DECLINED", "CONTACT_SUPPORT")
                    .await?;
                return Ok(Progress::Done {
                    state: "DECLINED".into(),
                    account_id: None,
                });
            }
            Outcome::Allow => {}
        }
        let linked = match self.controls.person(&actor).await {
            Ok(person) => Some(person),
            Err(error) if error.downcast_ref::<sqlx::Error>().is_none() => None,
            Err(error) => return Err(error),
        };
        let party = self
            .core
            .ensure_party_linked(&actor, id, &snapshot.profile.identity_digest()?, linked)
            .await?;
        self.controls.link_person(&actor, party).await?;
        let request = OpeningRequest {
            application_id: id,
            party_id: party,
            product_version_id: snapshot.product_version_id,
            acceptance_id: snapshot.acceptance_id,
        };
        let policy = OpeningGates {
            journeys: self,
            actor: &actor,
            snapshot: &snapshot,
            generation,
        };
        let account = self.core.prepare_opening(&request, &policy).await?;
        let authority = self.core.authority(account, Action::ServiceAccount).await?;
        ensure!(
            self.controls
                .authorize(&actor, Action::ServiceAccount, Some(&authority), id)
                .await?,
            "Current opening ownership denied"
        );
        self.core.activate(account, &request, &policy).await?;
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE journeys.applications SET account_id=$3,party_id=$4,state='ACTIVE',next_action='VIEW_ACCOUNT',version=version+1,updated_at=now() WHERE id=$1 AND generation=$2 AND state<>'ACTIVE'").bind(id).bind(generation).bind(account.0).bind(party.0).execute(&mut *tx).await?;
        if changed.rows_affected() == 1 {
            self.fact(&mut tx, id, "ACCOUNT_ACTIVATED").await?;
        }
        tx.commit().await?;
        Ok(Progress::Done {
            state: "ACTIVE".into(),
            account_id: Some(account),
        })
    }
    async fn state(&self, id: Uuid, generation: i32, state: &str, next: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let changed=sqlx::query("UPDATE journeys.applications SET state=$3,next_action=$4,version=version+1,updated_at=now() WHERE id=$1 AND generation=$2 AND state<>'ACTIVE' AND (state<>$3 OR next_action<>$4)").bind(id).bind(generation).bind(state).bind(next).execute(&mut *tx).await?;
        if changed.rows_affected() == 1 {
            self.fact(&mut tx, id, next).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn fact(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        reason: &str,
    ) -> Result<()> {
        let row = sqlx::query("SELECT state,version FROM journeys.applications WHERE id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
        let version: i64 = row.get("version");
        let state: String = row.get("state");
        sqlx::query("INSERT INTO journeys.history(id,application_id,version,state,reason_code) VALUES($1,$2,$3,$4,$5)").bind(Uuid::new_v4()).bind(id).bind(version).bind(&state).bind(reason).execute(&mut **tx).await?;
        EventStore::new(self.pool.clone(), Module::Journeys)
            .stage(
                tx,
                &EventEnvelope {
                    event_id: EventId::new(),
                    event_type: "application.changed".into(),
                    schema_version: 1,
                    producer: "journeys".into(),
                    aggregate_id: id,
                    aggregate_version: version,
                    occurred_at: Utc::now(),
                    correlation_id: id,
                    causation_id: id.to_string(),
                    data: json!({"applicationId":id,"state":state}),
                },
            )
            .await
    }
}
struct OpeningGates<'a, T: BlobStore> {
    journeys: &'a Journeys<T>,
    actor: &'a ActorContext,
    snapshot: &'a Snapshot,
    generation: i32,
}
impl<T: BlobStore> OpeningPolicy for OpeningGates<'_, T> {
    fn validate<'a>(
        &'a self,
        request: &'a OpeningRequest,
    ) -> BoxFuture<'a, Result<ActivationEvidence>> {
        Box::pin(async move {
            let app = self
                .journeys
                .get(self.actor, request.application_id)
                .await?;
            ensure!(
                app.generation == self.generation,
                "Application generation changed"
            );
            ensure!(
                ["SUBMITTING", "CONTROLS_PENDING", "MANUAL_REVIEW"].contains(&app.state.as_str())
                    && app
                        .profile
                        .as_ref()
                        .map(ApplicantProfile::decision_digest)
                        .transpose()?
                        == Some(self.snapshot.profile.decision_digest()?)
                    && app.acceptance_id == Some(self.snapshot.acceptance_id)
                    && app.evidence_id == Some(self.snapshot.evidence_id),
                "Opening material state changed"
            );
            self.journeys
                .terms
                .validate_acceptance(
                    self.actor,
                    request.acceptance_id,
                    request.application_id,
                    request.product_version_id,
                )
                .await?;
            let gates = self
                .journeys
                .checks
                .evaluate(
                    request.application_id,
                    Purpose::Onboarding,
                    &self.snapshot.input_digest,
                )
                .await?;
            ensure!(
                gates.outcome == Outcome::Allow && gates.valid_until > Utc::now(),
                "Current required controls have not cleared"
            );
            Ok(ActivationEvidence {
                application_id: request.application_id,
                party_id: request.party_id,
                product_version_id: request.product_version_id,
                acceptance_id: request.acceptance_id,
                controls_revision: gates.revision,
                expires_at: gates.valid_until,
            })
        })
    }
}
fn application(row: &sqlx::postgres::PgRow) -> Result<Application> {
    Ok(Application {
        id: row.get("id"),
        product_version_id: row.get("product_version_id"),
        terms_id: row.get("terms_id"),
        acceptance_id: row.get("acceptance_id"),
        profile: row
            .get::<Option<serde_json::Value>, _>("profile")
            .map(serde_json::from_value)
            .transpose()?,
        evidence_id: row.get("evidence_id"),
        state: row.get("state"),
        next_action: row.get("next_action"),
        version: row.get("version"),
        generation: row.get("generation"),
        operation_id: row.get("operation_id"),
        account_id: row.get::<Option<Uuid>, _>("account_id").map(AccountId),
        expires_at: row.get("expires_at"),
    })
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub type LocalJourneys = Journeys<bank_adapters::object_store::FilesystemStore>;
pub async fn local() -> Result<Arc<LocalJourneys>> {
    let controls_pool = Module::Controls.connect().await?;
    let store = bank_adapters::object_store::FilesystemStore::new(
        std::env::var("BANK_EVIDENCE_DIR").unwrap_or_else(|_| ".local/evidence".into()),
    )?;
    Ok(Arc::new(Journeys {
        pool: Module::Journeys.connect().await?,
        core: Core::new(Module::Core.connect().await?),
        checks: Checks::new(controls_pool.clone())?,
        controls: Entitlements::new(controls_pool.clone()),
        terms: Terms::new(Module::Documents.connect().await?),
        operations: Operations::new(Module::Platform.connect().await?),
        evidence: Arc::new(EvidenceService::new(controls_pool, Arc::new(store))),
    }))
}
