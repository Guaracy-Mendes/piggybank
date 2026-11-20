use crate::{
    audit::{AuditLog, AuditRecord},
    checks::Checks,
    entitlements::{Action, Entitlements, ResourceAuthority},
};
use anyhow::{Result, ensure};
use bank_platform::{
    database::Module,
    domain::{ActorContext, EventEnvelope, EventId, TrustContext},
    events::EventStore,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaseKind {
    Control,
    Support,
    Payment,
    Data,
    Finance,
}
impl CaseKind {
    fn code(self) -> &'static str {
        match self {
            Self::Control => "CONTROL",
            Self::Support => "SUPPORT",
            Self::Payment => "PAYMENT",
            Self::Data => "DATA",
            Self::Finance => "FINANCE",
        }
    }
    fn queue(self) -> &'static str {
        match self {
            Self::Control => "controls-operations",
            Self::Support => "support",
            Self::Payment => "payments-operations",
            Self::Data => "data-operations",
            Self::Finance => "finance-operations",
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Disposition {
    ClearControl,
    Decline,
    ResolveService,
    Escalate,
}
impl Disposition {
    fn code(self) -> &'static str {
        match self {
            Self::ClearControl => "CLEAR_CONTROL",
            Self::Decline => "DECLINE",
            Self::ResolveService => "RESOLVE_SERVICE",
            Self::Escalate => "ESCALATE",
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseRecord {
    pub id: Uuid,
    pub kind: String,
    pub resource_id: Uuid,
    pub queue: String,
    pub owner_person: Option<Uuid>,
    pub severity: String,
    pub state: String,
    pub next_action: String,
    pub evidence_refs: Vec<Uuid>,
    pub version: i64,
    pub deadline: DateTime<Utc>,
    pub escalated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
pub struct NewCase {
    pub kind: CaseKind,
    pub origin_id: Uuid,
    pub resource_id: Uuid,
    pub evidence_refs: Vec<Uuid>,
    pub customer: Option<(String, String)>,
}
#[derive(Clone)]
pub struct Cases {
    pub pool: PgPool,
    pub entitlements: Entitlements,
}
impl Cases {
    pub fn new(pool: PgPool) -> Self {
        Self {
            entitlements: Entitlements::new(pool.clone()),
            pool,
        }
    }
    pub async fn ensure(&self, actor: &ActorContext, source: NewCase) -> Result<Uuid> {
        let NewCase {
            kind,
            origin_id: origin,
            resource_id: resource,
            evidence_refs: evidence,
            customer,
        } = source;
        ensure!(
            evidence.len() <= 32,
            "Case evidence reference bound exceeded"
        );
        if actor.trust == TrustContext::Customer {
            ensure!(
                customer.as_ref() == Some(&(actor.issuer.clone(), actor.subject.clone())),
                "Customer case scope mismatch"
            );
        }
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        let created=sqlx::query("INSERT INTO controls.cases(id,kind,origin_id,resource_id,customer_issuer,customer_subject,queue,evidence_refs,deadline) VALUES($1,$2,$3,$4,$5,$6,$7,$8,now()+interval '4 hours') ON CONFLICT(kind,origin_id) DO NOTHING RETURNING id")
            .bind(id).bind(kind.code()).bind(origin).bind(resource).bind(customer.as_ref().map(|c|&c.0)).bind(customer.as_ref().map(|c|&c.1)).bind(kind.queue()).bind(serde_json::to_value(&evidence)?).fetch_optional(&mut *tx).await?;
        let row =
            sqlx::query("SELECT id,resource_id,customer_issuer,customer_subject FROM controls.cases WHERE kind=$1 AND origin_id=$2")
                .bind(kind.code())
                .bind(origin)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            row.get::<Uuid, _>("resource_id") == resource
                && row.get::<Option<String>, _>("customer_issuer")
                    == customer.as_ref().map(|c| c.0.clone())
                && row.get::<Option<String>, _>("customer_subject")
                    == customer.as_ref().map(|c| c.1.clone()),
            "Case origin scope conflict"
        );
        let id = row.get("id");
        if created.is_some() {
            self.fact(&mut tx, id, actor, "case.created").await?;
        }
        tx.commit().await?;
        Ok(id)
    }
    async fn authorized(&self, actor: &ActorContext, case: Uuid, action: Action) -> Result<Uuid> {
        ensure!(
            actor.trust == TrustContext::Workforce,
            "Workforce case authority required"
        );
        let person = self.entitlements.person(actor).await?;
        let capability = if matches!(action, Action::ApproveCase) {
            "APPROVE"
        } else {
            "REVIEW"
        };
        let related:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.cases c JOIN controls.case_queue_members m ON m.queue=c.queue WHERE c.id=$1 AND m.person_id=$2 AND m.capability=$3 AND m.revoked_at IS NULL)").bind(case).bind(person.0).bind(capability).fetch_one(&self.pool).await?;
        let authority = ResourceAuthority {
            delegates: vec![],
            resource_id: case,
            owners: vec![],
            permitted_staff: if related { vec![person] } else { vec![] },
            action_restricted: false,
        };
        ensure!(
            self.entitlements
                .authorize(actor, action, Some(&authority), case)
                .await?,
            "Case action denied"
        );
        Ok(person.0)
    }
    pub async fn list(&self, actor: &ActorContext, limit: i64) -> Result<Vec<CaseRecord>> {
        ensure!((1..=100).contains(&limit), "Invalid case page size");
        let person = self.entitlements.person(actor).await?;
        ensure!(
            actor.trust == TrustContext::Workforce
                && (self
                    .entitlements
                    .authorize(actor, Action::ReviewCase, None, Uuid::new_v4())
                    .await?
                    || self
                        .entitlements
                        .authorize(actor, Action::ApproveCase, None, Uuid::new_v4())
                        .await?),
            "Case queue denied"
        );
        let rows=sqlx::query("SELECT c.* FROM controls.cases c WHERE EXISTS(SELECT 1 FROM controls.case_queue_members m WHERE m.queue=c.queue AND m.person_id=$1 AND m.revoked_at IS NULL) AND c.state<>'CLOSED' ORDER BY c.deadline,c.id LIMIT $2").bind(person.0).bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(record).collect()
    }
    pub async fn detail(&self, actor: &ActorContext, id: Uuid) -> Result<CaseRecord> {
        if self
            .authorized(actor, id, Action::ReviewCase)
            .await
            .is_err()
        {
            self.authorized(actor, id, Action::ApproveCase).await?;
        }
        record(
            &sqlx::query("SELECT * FROM controls.cases WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn claim(&self, actor: &ActorContext, id: Uuid) -> Result<()> {
        let person = self.authorized(actor, id, Action::ReviewCase).await?;
        let mut tx = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT owner_person,state FROM controls.cases WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            row.get::<String, _>("state") != "CLOSED"
                && row
                    .get::<Option<Uuid>, _>("owner_person")
                    .is_none_or(|owner| owner == person),
            "Case already owned or closed"
        );
        if row.get::<Option<Uuid>, _>("owner_person").is_none() {
            sqlx::query("UPDATE controls.cases SET owner_person=$2,next_action='REVIEW',version=version+1,updated_at=now() WHERE id=$1").bind(id).bind(person).execute(&mut *tx).await?;
            self.fact(&mut tx, id, actor, "case.assigned").await?;
        }
        tx.commit().await?;
        Ok(())
    }
    /// Read context rechecks current queue authority; provider evidence is not an object download.
    pub async fn context(&self, actor: &ActorContext, id: Uuid) -> Result<serde_json::Value> {
        let case = self.detail(actor, id).await?;
        let proposals = sqlx::query("SELECT id,disposition,maker_person,checker_person,state,expires_at FROM controls.case_proposals WHERE case_id=$1 ORDER BY expires_at DESC,id LIMIT 100").bind(id).fetch_all(&self.pool).await?;
        let objects = sqlx::query("SELECT e.id,e.digest,e.size,e.state FROM controls.evidence_objects e JOIN controls.cases c ON c.id=$1 WHERE c.evidence_refs @> to_jsonb(e.id::text)").bind(id).fetch_all(&self.pool).await?;
        let checks = sqlx::query("SELECT r.id,r.kind,r.state,r.policy_version,r.input_digest,e.outcome,e.expires_at,e.document->>'reasonCode' AS reason_code,e.evidence_id FROM controls.cases c JOIN controls.check_requests r ON r.id=c.origin_id LEFT JOIN controls.check_results e ON e.request_id=r.id WHERE c.id=$1 AND c.kind='CONTROL'").bind(id).fetch_all(&self.pool).await?;
        Ok(
            serde_json::json!({"case":case,"proposals":proposals.into_iter().map(|r|serde_json::json!({"id":r.get::<Uuid,_>("id"),"disposition":r.get::<String,_>("disposition"),"makerPerson":r.get::<Uuid,_>("maker_person"),"checkerPerson":r.get::<Option<Uuid>,_>("checker_person"),"state":r.get::<String,_>("state"),"expiresAt":r.get::<DateTime<Utc>,_>("expires_at")})).collect::<Vec<_>>(),"objects":objects.into_iter().map(|r|serde_json::json!({"id":r.get::<Uuid,_>("id"),"digest":r.get::<String,_>("digest"),"size":r.get::<i64,_>("size"),"state":r.get::<String,_>("state")})).collect::<Vec<_>>(),"checks":checks.into_iter().map(|r|serde_json::json!({"id":r.get::<Uuid,_>("id"),"kind":r.get::<String,_>("kind"),"state":r.get::<String,_>("state"),"policyVersion":r.get::<String,_>("policy_version"),"inputDigest":r.get::<String,_>("input_digest"),"originalOutcome":r.get::<Option<String>,_>("outcome"),"expiresAt":r.get::<Option<DateTime<Utc>>,_>("expires_at"),"reasonCode":r.get::<Option<String>,_>("reason_code"),"providerEvidenceId":r.get::<Option<Uuid>,_>("evidence_id")})).collect::<Vec<_>>()}),
        )
    }
    pub async fn request_information(
        &self,
        actor: &ActorContext,
        id: Uuid,
        code: &str,
    ) -> Result<()> {
        ensure!(
            [
                "IDENTITY_EVIDENCE_REQUIRED",
                "CONTACT_VERIFICATION_REQUIRED",
                "PAYMENT_EVIDENCE_REQUIRED"
            ]
            .contains(&code),
            "Unsupported information request"
        );
        let person = self.authorized(actor, id, Action::ReviewCase).await?;
        let mut tx = self.pool.begin().await?;
        let updated=sqlx::query("UPDATE controls.cases SET state='INFORMATION_REQUIRED',next_action='AWAIT_CUSTOMER_INFORMATION',deadline=now()+interval '24 hours',version=version+1,updated_at=now() WHERE id=$1 AND owner_person=$2 AND state='OPEN'").bind(id).bind(person).execute(&mut *tx).await?;
        ensure!(updated.rows_affected() == 1, "Owned open case required");
        sqlx::query("INSERT INTO controls.case_actions VALUES($1,$2,$3,$4,'INFORMATION_REQUESTED',$5,now())").bind(Uuid::new_v4()).bind(id).bind(&actor.issuer).bind(&actor.subject).bind(code).execute(&mut *tx).await?;
        self.fact(&mut tx, id, actor, "case.information-requested")
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn propose(
        &self,
        actor: &ActorContext,
        id: Uuid,
        disposition: Disposition,
    ) -> Result<Uuid> {
        let person = self.authorized(actor, id, Action::ReviewCase).await?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM controls.cases WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        ensure!(
            row.get::<Option<Uuid>, _>("owner_person") == Some(person)
                && row.get::<String, _>("state") == "OPEN",
            "Owned open case required"
        );
        let kind: String = row.get("kind");
        ensure!(
            match disposition {
                Disposition::ClearControl | Disposition::Decline => kind == "CONTROL",
                Disposition::ResolveService => kind == "SUPPORT",
                Disposition::Escalate => true,
            },
            "Disposition cannot override this domain"
        );
        let proposal = Uuid::new_v4();
        let version = row.get::<i64, _>("version") + 1;
        sqlx::query("INSERT INTO controls.case_proposals(id,case_id,case_version,disposition,maker_person,expires_at) VALUES($1,$2,$3,$4,$5,now()+interval '15 minutes')").bind(proposal).bind(id).bind(version).bind(disposition.code()).bind(person).execute(&mut *tx).await?;
        sqlx::query("UPDATE controls.cases SET state='PROPOSED',next_action='INDEPENDENT_APPROVAL',version=$2,updated_at=now() WHERE id=$1").bind(id).bind(version).execute(&mut *tx).await?;
        self.fact(&mut tx, id, actor, "case.disposition-proposed")
            .await?;
        tx.commit().await?;
        Ok(proposal)
    }
    pub async fn approve(&self, actor: &ActorContext, case: Uuid, proposal: Uuid) -> Result<()> {
        let person = self.authorized(actor, case, Action::ApproveCase).await?;
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("SELECT c.*,p.id AS proposal_id,p.case_version,p.disposition,p.maker_person,p.state AS proposal_state,p.expires_at FROM controls.cases c JOIN controls.case_proposals p ON p.case_id=c.id WHERE c.id=$1 AND p.id=$2 FOR UPDATE OF c,p").bind(case).bind(proposal).fetch_one(&mut *tx).await?;
        ensure!(
            row.get::<Uuid, _>("maker_person") != person,
            "Maker cannot approve case through another login"
        );
        if row.get::<String, _>("proposal_state") == "APPROVED" {
            tx.commit().await?;
            return Ok(());
        }
        ensure!(
            row.get::<String, _>("proposal_state") == "PENDING"
                && row.get::<i64, _>("version") == row.get::<i64, _>("case_version")
                && row.get::<String, _>("state") == "PROPOSED"
                && row.get::<DateTime<Utc>, _>("expires_at") > Utc::now(),
            "Case approval changed or expired"
        );
        let disposition: String = row.get("disposition");
        let escalate = disposition == "ESCALATE";
        if disposition == "CLEAR_CONTROL" || disposition == "DECLINE" {
            let request: Uuid = row.get("origin_id");
            let result=sqlx::query("SELECT e.*,r.state AS request_state FROM controls.check_results e JOIN controls.check_requests r ON r.id=e.request_id JOIN controls.check_heads h ON h.request_id=r.id WHERE e.request_id=$1 FOR UPDATE OF r").bind(request).fetch_one(&mut *tx).await?;
            ensure!(
                result.get::<String, _>("outcome") == "REVIEW"
                    && result.get::<String, _>("request_state") == "REVIEW"
                    && result.get::<DateTime<Utc>, _>("expires_at") > Utc::now(),
                "Only current reviewable evidence permits manual control disposition"
            );
            let decision = if disposition == "CLEAR_CONTROL" {
                "ALLOW"
            } else {
                "DENY"
            };
            sqlx::query("INSERT INTO controls.check_reviews VALUES($1,$2,$3,$4,$5,$6,$7)")
                .bind(request)
                .bind(case)
                .bind(result.get::<String, _>("fingerprint"))
                .bind(row.get::<Uuid, _>("maker_person"))
                .bind(person)
                .bind(decision)
                .bind(result.get::<DateTime<Utc>, _>("expires_at"))
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE controls.check_requests SET state=$2,revision=revision+1 WHERE id=$1",
            )
            .bind(request)
            .bind(if decision == "ALLOW" {
                "MANUAL_ALLOW"
            } else {
                "MANUAL_DENY"
            })
            .execute(&mut *tx)
            .await?;
            Checks::new(self.pool.clone())?
                .fact(&mut tx, request, actor)
                .await?;
        }
        sqlx::query(
            "UPDATE controls.case_proposals SET state='APPROVED',checker_person=$2 WHERE id=$1",
        )
        .bind(proposal)
        .bind(person)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE controls.cases SET state=$2,next_action=$3,severity=CASE WHEN $4 THEN 'CRITICAL' ELSE severity END,version=version+1,updated_at=now() WHERE id=$1").bind(case).bind(if escalate{"OPEN"}else{"CLOSED"}).bind(if escalate{"ESCALATED_REVIEW"}else{"COMPLETE"}).bind(escalate).execute(&mut *tx).await?;
        self.fact(&mut tx, case, actor, "case.disposition-approved")
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn withdraw(&self, actor: &ActorContext, id: Uuid) -> Result<()> {
        let person = self.authorized(actor, id, Action::ReviewCase).await?;
        let mut tx = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT owner_person,state FROM controls.cases WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(
            row.get::<Option<Uuid>, _>("owner_person") == Some(person)
                && row.get::<String, _>("state") == "PROPOSED",
            "Owned proposed case required"
        );
        sqlx::query("UPDATE controls.case_proposals SET state='WITHDRAWN' WHERE case_id=$1 AND state='PENDING'").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE controls.cases SET state='OPEN',next_action='REVIEW',version=version+1,updated_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await?;
        self.fact(&mut tx, id, actor, "case.proposal-withdrawn")
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn provide_information<T: crate::evidence::BlobStore>(
        &self,
        actor: &ActorContext,
        id: Uuid,
        key: Uuid,
        data: Vec<u8>,
        evidence: &crate::evidence::EvidenceService<T>,
    ) -> Result<Uuid> {
        ensure!(
            actor.trust == TrustContext::Customer && !data.is_empty() && data.len() <= 32768,
            "Invalid customer evidence"
        );
        let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM controls.cases WHERE id=$1 AND customer_issuer=$2 AND customer_subject=$3)").bind(id).bind(&actor.issuer).bind(&actor.subject).fetch_one(&self.pool).await?;
        ensure!(owned, "Customer case unavailable");
        let digest = bank_platform::domain::fingerprint(&data)?;
        let receipt=sqlx::query("SELECT digest,evidence_id FROM controls.case_information WHERE case_id=$1 AND request_id=$2").bind(id).bind(key).fetch_optional(&self.pool).await?;
        if let Some(receipt) = receipt {
            ensure!(
                receipt.get::<String, _>("digest") == digest,
                "Information idempotency conflict"
            );
            return Ok(receipt.get("evidence_id"));
        }
        let state: String = sqlx::query_scalar("SELECT state FROM controls.cases WHERE id=$1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        ensure!(
            state == "INFORMATION_REQUIRED",
            "Case is not awaiting information"
        );
        let object = evidence.put(actor, "CASE", data, id).await?;
        let mut tx = self.pool.begin().await?;
        let row =
            sqlx::query("SELECT state,evidence_refs FROM controls.cases WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        let receipt=sqlx::query("SELECT digest,evidence_id FROM controls.case_information WHERE case_id=$1 AND request_id=$2").bind(id).bind(key).fetch_optional(&mut *tx).await?;
        if let Some(receipt) = receipt {
            ensure!(
                receipt.get::<String, _>("digest") == digest,
                "Information idempotency conflict"
            );
            let saved = receipt.get("evidence_id");
            tx.commit().await?;
            return Ok(saved);
        }
        ensure!(
            row.get::<String, _>("state") == "INFORMATION_REQUIRED",
            "Case information state changed"
        );
        let mut refs: Vec<Uuid> = serde_json::from_value(row.get("evidence_refs"))?;
        ensure!(refs.len() < 32, "Case reference limit exceeded");
        refs.push(object.id);
        sqlx::query("INSERT INTO controls.case_information VALUES($1,$2,$3,$4)")
            .bind(id)
            .bind(key)
            .bind(digest)
            .bind(object.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE controls.cases SET evidence_refs=$2,state='OPEN',next_action='REVIEW_INFORMATION',deadline=now()+interval '4 hours',version=version+1,updated_at=now() WHERE id=$1").bind(id).bind(serde_json::to_value(refs)?).execute(&mut *tx).await?;
        self.fact(&mut tx, id, actor, "case.information-provided")
            .await?;
        tx.commit().await?;
        Ok(object.id)
    }
    pub async fn customer_cases(&self, actor: &ActorContext) -> Result<serde_json::Value> {
        ensure!(
            actor.trust == TrustContext::Customer,
            "Customer context required"
        );
        let rows=sqlx::query("SELECT id,state,created_at FROM controls.cases WHERE customer_issuer=$1 AND customer_subject=$2 ORDER BY created_at DESC,id LIMIT 100").bind(&actor.issuer).bind(&actor.subject).fetch_all(&self.pool).await?;
        Ok(
            serde_json::json!({"items":rows.into_iter().map(|r|serde_json::json!({"id":r.get::<Uuid,_>("id"),"state":r.get::<String,_>("state"),"createdAt":r.get::<DateTime<Utc>,_>("created_at"),"message":if r.get::<String,_>("state")=="INFORMATION_REQUIRED"{"Additional information is required."}else if r.get::<String,_>("state")=="CLOSED"{"The review of this request is complete. Refer to your account or application for its current status."}else{"Your request is being handled by our team."}})).collect::<Vec<_>>()}),
        )
    }
    pub async fn ensure_control_exceptions(&self) -> Result<()> {
        let rows=sqlx::query("SELECT r.id,r.resource_id,r.customer_issuer,r.customer_subject,e.evidence_id FROM controls.check_requests r JOIN controls.check_heads h ON h.request_id=r.id LEFT JOIN controls.check_results e ON e.request_id=r.id WHERE (r.state IN ('REVIEW','DENY') OR r.next_action='MANUAL_REVIEW') AND NOT EXISTS(SELECT 1 FROM controls.cases c WHERE c.kind='CONTROL' AND c.origin_id=r.id) ORDER BY r.created_at LIMIT 20").fetch_all(&self.pool).await?;
        for row in rows {
            let customer = row
                .get::<Option<String>, _>("customer_issuer")
                .zip(row.get::<Option<String>, _>("customer_subject"));
            self.ensure(
                &system_actor(),
                NewCase {
                    kind: CaseKind::Control,
                    origin_id: row.get("id"),
                    resource_id: row.get("resource_id"),
                    evidence_refs: row
                        .get::<Option<Uuid>, _>("evidence_id")
                        .into_iter()
                        .collect(),
                    customer,
                },
            )
            .await?;
        }
        Ok(())
    }
    pub async fn escalate_overdue(&self) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let rows=sqlx::query("UPDATE controls.cases SET severity='CRITICAL',next_action='ESCALATED_REVIEW',escalated_at=now(),version=version+1,updated_at=now() WHERE id IN(SELECT id FROM controls.cases WHERE state<>'CLOSED' AND escalated_at IS NULL AND deadline<now() ORDER BY deadline LIMIT 20 FOR UPDATE SKIP LOCKED) RETURNING id").fetch_all(&mut *tx).await?;
        for row in rows {
            self.fact(&mut tx, row.get("id"), &system_actor(), "case.escalated")
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn fact(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        actor: &ActorContext,
        action: &str,
    ) -> Result<()> {
        let row =
            sqlx::query("SELECT version,state,severity,queue FROM controls.cases WHERE id=$1")
                .bind(id)
                .fetch_one(&mut **tx)
                .await?;
        EventStore::new(self.pool.clone(),Module::Controls).stage(tx,&EventEnvelope{event_id:EventId::new(),event_type:"case.changed".into(),schema_version:1,producer:"controls".into(),aggregate_id:id,aggregate_version:row.get("version"),occurred_at:Utc::now(),correlation_id:id,causation_id:id.to_string(),data:serde_json::json!({"caseId":id,"state":row.get::<String,_>("state"),"severity":row.get::<String,_>("severity"),"queue":row.get::<String,_>("queue")})}).await?;
        AuditLog::append_in(
            tx,
            Uuid::new_v4(),
            "cases",
            &AuditRecord::new(actor, action, id, id),
        )
        .await?;
        Ok(())
    }
}
fn record(row: &sqlx::postgres::PgRow) -> Result<CaseRecord> {
    Ok(CaseRecord {
        id: row.get("id"),
        kind: row.get("kind"),
        resource_id: row.get("resource_id"),
        queue: row.get("queue"),
        owner_person: row.get("owner_person"),
        severity: row.get("severity"),
        state: row.get("state"),
        next_action: row.get("next_action"),
        evidence_refs: serde_json::from_value(row.get("evidence_refs"))?,
        version: row.get("version"),
        deadline: row.get("deadline"),
        escalated_at: row.get("escalated_at"),
        created_at: row.get("created_at"),
    })
}
fn system_actor() -> ActorContext {
    ActorContext {
        issuer: "urn:piggybank:worker".into(),
        subject: "case-monitor".into(),
        trust: TrustContext::Service,
        client_id: "bank-worker".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
