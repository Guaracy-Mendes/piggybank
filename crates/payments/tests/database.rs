use anyhow::Result;
use bank_controls::{
    checks::{CheckResult, Checks, Outcome, Purpose, simulator_key},
    entitlements::Entitlements,
    governance::ChangeKind,
};
use bank_core::{
    Core,
    accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest},
    payments::{ExecutionPermit, InternalInstruction, PaymentPolicy},
    products::ProductVersion,
};
use bank_identity::{IdentityService, oidc::Verifier};
use bank_payments::{PaymentStatus, Payments, Progress};
use bank_platform::{
    authenticity::SignedDocument,
    database::Module,
    domain::{AccountId, ActorContext, Currency, Money, PaymentDetails, TrustContext, fingerprint},
    operations::Operations,
    testing::TestDatabase,
};
use chrono::{Duration, Utc};
use futures_util::future::BoxFuture;
use serde_json::json;
use uuid::Uuid;
fn money(n: i64) -> Money {
    Money::new(n, Currency::synthetic())
}
fn actor(subject: &str, trust: TrustContext) -> ActorContext {
    ActorContext {
        issuer: "urn:payment-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: if trust == TrustContext::Customer {
            "customer-web"
        } else {
            "staff-web"
        }
        .into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
struct OpeningFixture;
impl OpeningPolicy for OpeningFixture {
    fn validate<'a>(&'a self, r: &'a OpeningRequest) -> BoxFuture<'a, Result<ActivationEvidence>> {
        Box::pin(async move {
            Ok(ActivationEvidence {
                application_id: r.application_id,
                party_id: r.party_id,
                product_version_id: r.product_version_id,
                acceptance_id: r.acceptance_id,
                controls_revision: "trusted-financial-test-opening".into(),
                expires_at: Utc::now() + Duration::minutes(10),
            })
        })
    }
}
struct FinancialPermit(chrono::DateTime<Utc>);
impl PaymentPolicy for FinancialPermit {
    fn validate<'a>(
        &'a self,
        r: &'a InternalInstruction,
    ) -> BoxFuture<'a, Result<ExecutionPermit>> {
        Box::pin(async move {
            Ok(ExecutionPermit {
                request_digest: fingerprint(r)?,
                valid_until: self.0,
            })
        })
    }
}
struct Fixture {
    db: TestDatabase,
    p: Payments,
    alice: ActorContext,
    bob: ActorContext,
    from: AccountId,
    to: AccountId,
    product: ProductVersion,
}
impl Fixture {
    async fn new() -> Result<Self> {
        let db = TestDatabase::new().await?;
        let core = Core::new(db.module(Module::Core).await?);
        let controls = Entitlements::new(db.module(Module::Controls).await?);
        let alice = actor("alice", TrustContext::Customer);
        let bob = actor("bob", TrustContext::Customer);
        let maker = actor("maker", TrustContext::Workforce);
        let checker = actor("checker", TrustContext::Workforce);
        for (who, role) in [(&maker, "product_admin"), (&checker, "product_approver")] {
            let person = bank_platform::domain::PartyId::new();
            controls.link_person(who, person).await?;
            sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&db.migrator).await?;
        }
        let product = ProductVersion {
            id: Uuid::new_v4(),
            code: "PAYMENT_FIXTURE".into(),
            version: 1,
            name: "Synthetic test pricing".into(),
            label: "SYNTHETIC".into(),
            effective_from: Utc::now().date_naive() - Duration::days(1),
            effective_until: Utc::now().date_naive() + Duration::days(365),
            minimum_age: 18,
            payment_limit: money(10000),
            daily_limit: money(15000),
            flat_fee: money(7),
            fee_basis_points: 25,
            chart_version: "synthetic-chart-v1".into(),
        };
        let proposal = controls
            .propose_change(
                &maker,
                ChangeKind::Product,
                product.id,
                &product,
                Utc::now() + Duration::minutes(10),
            )
            .await?;
        controls.approve_change(&checker, proposal).await?;
        core.publish_product(&controls, &maker, proposal, &product)
            .await?;
        let mut accounts = vec![];
        for who in [&alice, &bob] {
            let app = Uuid::new_v4();
            let party = core
                .ensure_party(who, app, &fingerprint(&who.subject)?)
                .await?;
            controls.link_person(who, party).await?;
            let request = OpeningRequest {
                application_id: app,
                party_id: party,
                product_version_id: product.id,
                acceptance_id: Uuid::new_v4(),
            };
            let account = core.stage_account(&request).await?;
            core.activate(account, &request, &OpeningFixture).await?;
            accounts.push(account);
            sqlx::query(
                "INSERT INTO identity.subjects(issuer,subject,trust) VALUES($1,$2,'customer')",
            )
            .bind(&who.issuer)
            .bind(&who.subject)
            .execute(&db.migrator)
            .await?;
        }
        let clearing = AccountId::new();
        sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'SETTLEMENT','ACTIVE',false)").bind(clearing.0).execute(&db.migrator).await?;
        core.fund_synthetic(Uuid::new_v4(), accounts[0], clearing, money(100000))
            .await?;
        let identity = IdentityService {
            verifier: Verifier::new()?,
            pool: db.module(Module::Identity).await?,
            public_base: "http://127.0.0.1:4180".into(),
        };
        let p = Payments {
            pool: db.module(Module::Payments).await?,
            core,
            checks: Checks::new(controls.pool.clone())?,
            controls,
            operations: Operations::new(db.module(Module::Platform).await?),
            identity,
            rail_client: bank_rails::RailClient::local()?,
        };
        Ok(Self {
            db,
            p,
            alice,
            bob,
            from: accounts[0],
            to: accounts[1],
            product,
        })
    }
    async fn draft(&self, amount: i64) -> Result<PaymentStatus> {
        let reference = self
            .p
            .core
            .receiving_reference(&self.bob, &self.p.controls, self.to)
            .await?;
        let b = self
            .p
            .add_beneficiary(
                &self.alice,
                Uuid::new_v4(),
                "Synthetic recipient",
                &reference,
            )
            .await?;
        let q = self
            .p
            .quote(
                &self.alice,
                self.from,
                b.id,
                money(amount),
                Utc::now().date_naive(),
            )
            .await?;
        assert_eq!(q.total, q.amount.checked_add(q.fee)?);
        self.p
            .create_intent(&self.alice, Uuid::new_v4(), q.id)
            .await
    }
    async fn proof(&self, status: &PaymentStatus, digest: &str) -> Result<()> {
        sqlx::query("INSERT INTO identity.step_up_proofs(id,challenge_id,issuer,subject,intent_digest,authenticated_at,expires_at) VALUES($1,$2,$3,$4,$5,now(),now()+interval '1 minute')").bind(Uuid::new_v4()).bind(status.challenge_id).bind(&self.alice.issuer).bind(&self.alice.subject).bind(digest).execute(&self.db.migrator).await?;
        Ok(())
    }
    async fn instruction(&self, status: &PaymentStatus) -> Result<InternalInstruction> {
        Ok(serde_json::from_value(
            sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT instruction FROM payments.quotes WHERE id=$1",
            )
            .bind(status.quote_id)
            .fetch_one(&self.p.pool)
            .await?,
        )?)
    }
    async fn allow_controls(&self, status: &PaymentStatus, outcome: Outcome) -> Result<()> {
        let instruction = self.instruction(status).await?;
        for kind in Purpose::Payment.required() {
            let request = self
                .p
                .checks
                .request(&self.alice, status.id, *kind, &fingerprint(&instruction)?)
                .await?;
            let result = CheckResult {
                request,
                outcome,
                evidence_id: Uuid::new_v4(),
                expires_at: Utc::now() + Duration::minutes(5),
                reason_code: "SYNTHETIC_FIXTURE".into(),
            };
            self.p
                .checks
                .record(&SignedDocument::sign(simulator_key().as_bytes(), &result)?)
                .await?;
        }
        Ok(())
    }
    async fn close(self) -> Result<()> {
        self.p.pool.close().await;
        self.db.close().await
    }
}
#[tokio::test]
async fn scoped_approval_controlled_posting_and_lost_ack_recover_one_economic_payment() -> Result<()>
{
    let f = Fixture::new().await?;
    let reference =
        f.p.core
            .receiving_reference(&f.bob, &f.p.controls, f.to)
            .await?;
    assert!(
        f.p.core
            .receiving_reference(&f.alice, &f.p.controls, f.to)
            .await
            .is_err()
    );
    let key = Uuid::new_v4();
    let b =
        f.p.add_beneficiary(&f.alice, key, "Synthetic recipient", &reference)
            .await?;
    assert_eq!(
        b.id,
        f.p.add_beneficiary(&f.alice, key, "Synthetic recipient", &reference)
            .await?
            .id
    );
    assert!(
        f.p.add_beneficiary(&f.alice, key, "Changed", &reference)
            .await
            .is_err()
    );
    assert!(
        f.p.quote(&f.bob, f.from, b.id, money(10), Utc::now().date_naive())
            .await
            .is_err()
    );
    assert!(
        f.p.quote(&f.alice, f.from, b.id, money(-1), Utc::now().date_naive())
            .await
            .is_err()
    );
    let q =
        f.p.quote(
            &f.alice,
            f.from,
            b.id,
            money(10000),
            Utc::now().date_naive(),
        )
        .await?;
    assert_eq!(q.fee, money(32));
    assert_eq!(q.total, money(10032));
    let key = Uuid::new_v4();
    let (a, b) = tokio::join!(
        f.p.create_intent(&f.alice, key, q.id),
        f.p.create_intent(&f.alice, key, q.id)
    );
    let draft = a?;
    let concurrent = b?;
    assert_eq!(draft.id, concurrent.id);
    assert!(draft.challenge_id.is_some());
    assert_eq!(draft.challenge_id, concurrent.challenge_id);
    assert!(f.p.detail(&f.bob, draft.id).await.is_err());
    assert!(f.p.confirm(&f.alice, draft.id).await.is_err());
    assert!(f.p.process(draft.id).await.is_err());
    f.proof(&draft, &fingerprint(&draft.details)?).await?;
    assert!(f.p.detail(&f.alice, draft.id).await?.approval_ready);
    let accepted = f.p.confirm(&f.alice, draft.id).await?;
    assert_eq!(accepted.state, "ACCEPTED");
    assert_eq!(
        accepted.operation_id,
        f.p.confirm(&f.alice, draft.id).await?.operation_id
    );
    assert!(matches!(f.p.process(draft.id).await?, Progress::Pending));
    assert_eq!(f.p.core.balance(f.from).await?.booked, money(100000));
    f.allow_controls(&draft, Outcome::Allow).await?;
    assert!(f.p.process_with_ack_fault(draft.id, true).await.is_err());
    assert_eq!(f.p.core.balance(f.from).await?.booked, money(89968));
    assert_ne!(f.p.detail(&f.alice, draft.id).await?.state, "SETTLED");
    let (first, second) = tokio::join!(f.p.process(draft.id), f.p.process(draft.id));
    first?;
    second?;
    let done = f.p.detail(&f.alice, draft.id).await?;
    assert_eq!(done.state, "SETTLED");
    assert!(done.batch_id.is_some());
    assert_eq!(f.p.core.balance(f.to).await?.booked, money(10000));
    assert_eq!(
        f.p.core
            .balance(AccountId(Uuid::parse_str(
                "4f1a780b-3e31-48ef-a294-000000000004"
            )?))
            .await?
            .booked,
        money(32)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.payment_executions")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT sum(booked)::bigint FROM core.ledger_accounts")
            .fetch_one(&f.db.migrator)
            .await?,
        0
    );
    assert!(
        f.p.create_intent(&f.alice, Uuid::new_v4(), q.id)
            .await
            .is_err()
    );
    f.p.relay_audit().await?;
    f.p.relay_audit().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM payments.audit_pending WHERE delivered_at IS NULL"
        )
        .fetch_one(&f.p.pool)
        .await?,
        0
    );
    for statement in [
        "UPDATE payments.quotes SET total_minor=1",
        "TRUNCATE payments.intents",
        "UPDATE core.payment_executions SET fee=0",
        "UPDATE payments.beneficiaries SET label='changed'",
    ] {
        assert!(
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&f.db.migrator)
                .await
                .is_err()
        );
    }
    assert!(
        sqlx::query("UPDATE core.ledger_accounts SET booked=0")
            .execute(&f.p.pool)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn current_beneficiary_and_control_denial_block_confirmed_instructions() -> Result<()> {
    let f = Fixture::new().await?;
    let first = f.draft(1000).await?;
    f.proof(&first, &fingerprint(&first.details)?).await?;
    f.p.confirm(&f.alice, first.id).await?;
    f.p.revoke_beneficiary(&f.alice, Uuid::parse_str(&first.details.beneficiary_id)?)
        .await?;
    f.p.process(first.id).await?;
    assert_eq!(f.p.detail(&f.alice, first.id).await?.state, "BLOCKED");
    let second = f.draft(1000).await?;
    f.proof(&second, &fingerprint(&second.details)?).await?;
    f.p.confirm(&f.alice, second.id).await?;
    f.allow_controls(&second, Outcome::Deny).await?;
    f.p.process(second.id).await?;
    assert_eq!(f.p.detail(&f.alice, second.id).await?.state, "DECLINED");
    let third = f.draft(1000).await?;
    f.proof(&third, &fingerprint(&json!({"changed":"instruction"}))?)
        .await?;
    assert!(f.p.confirm(&f.alice, third.id).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.payment_executions")
            .fetch_one(&f.db.migrator)
            .await?,
        0
    );
    // A writable orchestration record cannot fabricate authoritative financial completion.
    sqlx::query("UPDATE payments.intents SET state='SETTLED',batch_id=$2 WHERE id=$1")
        .bind(third.id)
        .bind(Uuid::new_v4())
        .execute(&f.p.pool)
        .await?;
    assert!(f.p.detail(&f.alice, third.id).await.is_err());
    f.close().await
}
#[tokio::test]
async fn core_serializes_daily_limits_fees_and_authority_expiring_during_lock_wait() -> Result<()> {
    let f = Fixture::new().await?;
    let instruction = |id: Uuid| InternalInstruction {
        operation_id: id,
        details: PaymentDetails {
            payer_account_id: f.from,
            beneficiary_id: Uuid::new_v4().to_string(),
            amount: money(10000),
            fee: money(32),
            requested_date: Utc::now().date_naive(),
            rail: "INTERNAL".into(),
            quote_id: Uuid::new_v4().to_string(),
        },
        payee_account_id: f.to,
        product_version_id: f.product.id,
        beneficiary_version: 1,
        rail_policy_version_id: None,
    };
    let first = instruction(Uuid::new_v4());
    let second = instruction(Uuid::new_v4());
    let permit = FinancialPermit(Utc::now() + Duration::minutes(1));
    let (a, b) = tokio::join!(
        f.p.core.execute_internal_payment(&first, &permit),
        f.p.core.execute_internal_payment(&second, &permit)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let (committed, batch) = if let Ok(batch) = a {
        (&first, batch)
    } else {
        (&second, b?)
    };
    assert_eq!(
        f.p.core
            .execute_internal_payment(
                committed,
                &FinancialPermit(Utc::now() - Duration::seconds(1))
            )
            .await?,
        batch
    );
    let mut altered = committed.clone();
    altered.details.fee = money(33);
    assert!(
        f.p.core
            .execute_internal_payment(&altered, &permit)
            .await
            .is_err()
    );
    assert_eq!(f.p.core.balance(f.from).await?.booked, money(89968));
    let mut waiting = instruction(Uuid::new_v4());
    waiting.details.amount = money(1);
    waiting.details.fee = money(7);
    let mut lock = f.db.migrator.begin().await?;
    sqlx::query("SELECT id FROM core.ledger_accounts WHERE id=$1 FOR UPDATE")
        .bind(f.from.0)
        .fetch_one(&mut *lock)
        .await?;
    let core = f.p.core.clone();
    let expired = FinancialPermit(Utc::now() + Duration::milliseconds(200));
    let job = tokio::spawn(async move { core.execute_internal_payment(&waiting, &expired).await });
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    assert!(!job.is_finished());
    lock.commit().await?;
    assert!(job.await?.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.payment_executions")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    f.close().await
}

struct RailProcess(std::process::Child);
impl Drop for RailProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn rail_server(f: &Fixture, address: std::net::SocketAddr) -> Result<RailProcess> {
    Ok(RailProcess(
        std::process::Command::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/provider-simulator"),
        )
        .env("BANK_BIND", address.to_string())
        .env(
            "BANK_SIMULATOR_DATABASE_URL",
            f.db.module_url(Module::Simulator),
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?,
    ))
}
async fn wait_rail(base: &str) -> Result<()> {
    let http = reqwest::Client::new();
    for _ in 0..100 {
        if http.get(format!("{base}/health/live")).send().await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    anyhow::bail!("Isolated provider did not start")
}
async fn rail_policy(f: &Fixture) -> Result<(ActorContext, ActorContext)> {
    let maker = actor("maker", TrustContext::Workforce);
    let checker = actor("checker", TrustContext::Workforce);
    for (who, role) in [(&maker, "payment_operator"), (&checker, "payment_approver")] {
        let person = f.p.controls.person(who).await?;
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&f.db.migrator).await?;
    }
    let policy = bank_payments::policy::RailPolicyVersion {
        id: Uuid::new_v4(),
        rail: "SIMULATED".into(),
        version: 1,
        label: "SYNTHETIC".into(),
        enabled: true,
        effective_from: Utc::now().date_naive() - Duration::days(1),
        effective_until: Utc::now().date_naive() + Duration::days(365),
        payment_limit: money(10000),
        calendar: "SYNTHETIC_UTC_24X7".into(),
    };
    let change = f.p.propose_rail_policy(&maker, &policy).await?;
    assert!(f.p.apply_rail_change(&maker, change).await.is_err());
    f.p.approve_rail_change(&checker, change).await?;
    f.p.apply_rail_change(&maker, change).await?;
    f.p.apply_rail_change(&maker, change).await?;
    assert!(!f.p.rail_policy().await?.paused);
    Ok((maker, checker))
}
async fn rail_draft(f: &Fixture, amount: i64) -> Result<PaymentStatus> {
    let b =
        f.p.add_rail_beneficiary(
            &f.alice,
            Uuid::new_v4(),
            "Simulated rail recipient",
            "SIM00000000000000000000000000000001",
            "SIMULATED",
        )
        .await?;
    assert_eq!(b.rail, "SIMULATED");
    let q =
        f.p.quote(
            &f.alice,
            f.from,
            b.id,
            money(amount),
            Utc::now().date_naive(),
        )
        .await?;
    assert_eq!(q.rail, "SIMULATED");
    let draft = f.p.create_intent(&f.alice, Uuid::new_v4(), q.id).await?;
    f.proof(&draft, &fingerprint(&draft.details)?).await?;
    f.p.confirm(&f.alice, draft.id).await?;
    f.allow_controls(&draft, Outcome::Allow).await?;
    Ok(draft)
}
#[tokio::test]
async fn external_lost_response_restart_pause_and_independent_resume_keep_one_reserved_instruction()
-> Result<()> {
    let mut f = Fixture::new().await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base = format!("http://{address}");
    let mut server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    f.p.rail_client = bank_rails::RailClient::new(&base)?;
    let (maker, checker) = rail_policy(&f).await?;
    let payment = rail_draft(&f, 1000).await?;
    let other = rail_draft(&f, 2000).await?;
    reqwest::Client::new()
        .post(format!("{base}/admin/rail-scenarios"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&json!({"instructionId":payment.id,"mode":"LOST_RESPONSE"}))
        .send()
        .await?
        .error_for_status()?;
    let before = f.p.core.balance(f.from).await?;
    let (a, b) = tokio::join!(f.p.process(payment.id), f.p.process(payment.id));
    a?;
    b?;
    assert_eq!(
        f.p.detail(&f.alice, payment.id).await?.state,
        "OUTCOME_UNKNOWN"
    );
    let held =
        f.p.core
            .external_reservation(&f.instruction(&payment).await?)
            .await?
            .unwrap();
    assert!(held.submitted);
    assert_eq!(held.state, "ACTIVE");
    assert_eq!(held.amount, payment.total);
    let after = f.p.core.balance(f.from).await?;
    assert_eq!(after.booked, before.booked);
    assert_eq!(
        after.available,
        money(before.available.minor_units() - payment.total.minor_units())
    );
    assert!(
        f.p.core
            .release_external_unsubmitted(&f.instruction(&payment).await?)
            .await
            .is_err()
    );
    f.p.pause_rail(&maker).await?;
    f.p.process(other.id).await?;
    assert_eq!(f.p.detail(&f.alice, other.id).await?.state, "RAIL_PAUSED");
    assert!(
        f.p.core
            .external_reservation(&f.instruction(&other).await?)
            .await?
            .is_none()
    );
    server.0.kill()?;
    server.0.wait()?;
    server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    // Pending inquiry and original instruction survive the provider process restart and pause.
    f.p.process(payment.id).await?;
    assert_eq!(f.p.detail(&f.alice, payment.id).await?.state, "SUBMITTED");
    assert!(f.p.detail(&f.alice, payment.id).await?.batch_id.is_none());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM simulator.rail_instructions WHERE id=$1")
            .bind(payment.id)
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(count, 1);
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM payments.rail_submissions WHERE payment_id=$1")
            .bind(payment.id)
            .fetch_one(&f.p.pool)
            .await?;
    assert_eq!(attempts, 1);
    let resume = f.p.propose_rail_resume(&maker).await?;
    let alias = actor("same-human-alias", TrustContext::Workforce);
    let person = f.p.controls.person(&maker).await?;
    f.p.controls.link_person(&alias, person).await?;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'payment_approver',now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).execute(&f.db.migrator).await?;
    assert!(f.p.approve_rail_change(&alias, resume).await.is_err());
    f.p.approve_rail_change(&checker, resume).await?;
    let checker_person = f.p.controls.person(&checker).await?;
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='payment_approver'").bind(checker_person.0).execute(&f.db.migrator).await?;
    assert!(f.p.apply_rail_change(&maker, resume).await.is_err());
    sqlx::query("UPDATE controls.role_grants SET revoked_at=NULL WHERE person_id=$1 AND role='payment_approver'").bind(checker_person.0).execute(&f.db.migrator).await?;
    f.p.apply_rail_change(&maker, resume).await?;
    f.p.apply_rail_change(&maker, resume).await?;
    assert!(!f.p.rail_policy().await?.paused);
    f.p.process(other.id).await?;
    assert_eq!(f.p.detail(&f.alice, other.id).await?.state, "SUBMITTED");
    assert_eq!(f.p.core.balance(f.from).await?.booked, before.booked);
    // Crash after Core submission marking, before Payments work commits: inquiry cannot become a blind resend.
    let crash = rail_draft(&f, 3000).await?;
    let instruction = f.instruction(&crash).await?;
    let reserved =
        f.p.core
            .reserve_external_payment(
                &instruction,
                &FinancialPermit(Utc::now() + Duration::minutes(1)),
            )
            .await?;
    f.p.core
        .mark_external_submitted(&instruction, reserved.authority_until)
        .await?;
    f.p.process(crash.id).await?;
    assert_eq!(
        f.p.detail(&f.alice, crash.id).await?.state,
        "OUTCOME_UNKNOWN"
    );
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM payments.rail_submissions WHERE payment_id=$1")
            .bind(crash.id)
            .fetch_one(&f.p.pool)
            .await?;
    assert_eq!(attempts, 0);
    assert_eq!(
        f.p.core
            .external_reservation(&instruction)
            .await?
            .unwrap()
            .state,
        "ACTIVE"
    );
    drop(server);
    f.close().await
}
#[tokio::test]
async fn active_external_reservations_share_atomic_daily_budget_and_cannot_be_released_after_submission()
-> Result<()> {
    let f = Fixture::new().await?;
    let original = f.draft(8000).await?;
    let internal = f.instruction(&original).await?;
    let mut external = internal.clone();
    external.operation_id = Uuid::new_v4();
    external.details.rail = "SIMULATED".into();
    external.rail_policy_version_id = Some(Uuid::new_v4());
    external.payee_account_id = AccountId(bank_payments::policy::SIMULATED_CLEARING);
    let permit = FinancialPermit(Utc::now() + Duration::minutes(2));
    let r =
        f.p.core
            .reserve_external_payment(&external, &permit)
            .await?;
    assert!(!r.submitted);
    let positions = bank_core::finance::FinanceRecords::positions(&f.p.core).await?;
    assert_eq!(positions.reserved_outgoing_principal, "8000");
    assert_eq!(positions.submitted_outgoing_principal, "0");
    assert_eq!(
        positions.customer_held,
        (external.details.amount.minor_units() + external.details.fee.minor_units()).to_string()
    );
    assert!(
        f.p.core
            .execute_internal_payment(&internal, &permit)
            .await
            .is_err()
    );
    f.p.core.release_external_unsubmitted(&external).await?;
    f.p.core.release_external_unsubmitted(&external).await?;
    assert_eq!(
        bank_core::finance::FinanceRecords::positions(&f.p.core)
            .await?
            .reserved_outgoing_principal,
        "0"
    );
    assert!(
        f.p.core
            .reserve_external_payment(&external, &permit)
            .await
            .is_err()
    );
    f.p.core
        .execute_internal_payment(&internal, &permit)
        .await?;
    external.operation_id = Uuid::new_v4();
    external.details.amount = money(7000);
    external.details.fee = f.product.fee(money(7000))?;
    f.p.core
        .reserve_external_payment(&external, &permit)
        .await?;
    assert!(
        f.p.core
            .mark_external_submitted(&external, Utc::now() - Duration::seconds(1))
            .await
            .is_err()
    );
    f.p.core
        .mark_external_submitted(
            &external,
            f.p.core
                .external_reservation(&external)
                .await?
                .unwrap()
                .authority_until,
        )
        .await?;
    assert!(
        f.p.core
            .release_external_unsubmitted(&external)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT core.release_hold($1,$2)")
            .bind(external.operation_id)
            .bind(Uuid::new_v4())
            .execute(&f.p.core.pool)
            .await
            .is_err()
    );
    assert!(f.p.core.release(external.operation_id).await.is_err());
    assert!(
        f.p.core
            .mark_submitted(external.operation_id)
            .await
            .is_err()
    );
    assert!(
        f.p.core
            .transfer(
                Uuid::new_v4(),
                f.from,
                f.to,
                external.details.amount.checked_add(external.details.fee)?,
                Some(external.operation_id)
            )
            .await
            .is_err()
    );
    let changed = InternalInstruction {
        operation_id: Uuid::new_v4(),
        ..internal
    };
    assert!(
        f.p.core
            .execute_internal_payment(&changed, &permit)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE core.external_reservations SET authority_until=now()+interval '1 day'")
            .execute(&f.p.core.pool)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn provider_deduplicates_rejects_conflicts_and_cancellation_tombstone_blocks_delayed_submission()
-> Result<()> {
    let mut f = Fixture::new().await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base = format!("http://{address}");
    let server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    f.p.rail_client = bank_rails::RailClient::new(&base)?;
    let id = Uuid::new_v4();
    let instruction = bank_rails::RailInstruction {
        instruction_id: id,
        payment_id: id,
        beneficiary_reference: "SIM00000000000000000000000000000001".into(),
        amount: money(100),
        requested_date: Utc::now().date_naive(),
        input_digest: "a".repeat(64),
        rail: "SIMULATED".into(),
        authority_until: Utc::now() + Duration::minutes(1),
    };
    let a = f.p.rail_client.submit(&instruction).await?;
    let b = f.p.rail_client.submit(&instruction).await?;
    assert_eq!(a.digest, b.digest);
    let mut altered = instruction.clone();
    altered.amount = money(101);
    assert!(f.p.rail_client.submit(&altered).await.is_err());
    let mut cancelled = instruction.clone();
    cancelled.payment_id = Uuid::new_v4();
    cancelled.instruction_id = cancelled.payment_id;
    assert_eq!(
        f.p.rail_client.inquire(&cancelled).await?.evidence.status,
        bank_rails::RailStatus::NotFound
    );
    let tombstone = f.p.rail_client.cancel(&cancelled).await?;
    assert_eq!(tombstone.evidence.status, bank_rails::RailStatus::Cancelled);
    assert_eq!(
        f.p.rail_client.submit(&cancelled).await?.digest,
        tombstone.digest
    );
    reqwest::Client::new()
        .post(format!("{base}/admin/rail-scenarios"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&json!({"instructionId":instruction.instruction_id,"mode":"CONFLICT"}))
        .send()
        .await?
        .error_for_status()?;
    assert!(f.p.rail_client.submit(&instruction).await.is_err());
    assert_eq!(
        f.p.rail_client.inquire(&instruction).await?.digest,
        a.digest
    );
    let mut forged = a.document.clone();
    forged.body["status"] = json!("SETTLED");
    assert!(
        bank_rails::VerifiedEvidence::verify(forged, &bank_rails::signing_key(), &instruction)
            .is_err()
    );
    let wrong =
        bank_platform::authenticity::SignedDocument::sign(simulator_key().as_bytes(), &a.evidence)?;
    assert!(
        bank_rails::VerifiedEvidence::verify(wrong, &bank_rails::signing_key(), &instruction)
            .is_err()
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn generic_hold_commands_recheck_external_binding_after_waiting_for_its_commit() -> Result<()>
{
    let f = Fixture::new().await?;
    let original = f.draft(1000).await?;
    let mut instruction = f.instruction(&original).await?;
    instruction.operation_id = Uuid::new_v4();
    instruction.details.rail = "SIMULATED".into();
    instruction.rail_policy_version_id = Some(Uuid::new_v4());
    instruction.payee_account_id = AccountId(bank_payments::policy::SIMULATED_CLEARING);
    let mut tx = f.db.migrator.begin().await?;
    sqlx::query("SELECT core.reserve_external_payment($1,$2,$3)")
        .bind(serde_json::to_value(&instruction)?)
        .bind(fingerprint(&instruction)?)
        .bind(Utc::now() + Duration::minutes(1))
        .execute(&mut *tx)
        .await?;
    let core = f.p.core.clone();
    let id = instruction.operation_id;
    let mark = tokio::spawn(async move { core.mark_submitted(id).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!mark.is_finished());
    tx.commit().await?;
    assert!(mark.await?.is_err());
    assert!(
        !f.p.core
            .external_reservation(&instruction)
            .await?
            .unwrap()
            .submitted
    );
    f.p.core.release_external_unsubmitted(&instruction).await?;
    f.close().await
}

#[tokio::test]
async fn authenticated_finality_books_once_after_freeze_recovers_lost_ack_and_releases_only_definitive_failure()
-> Result<()> {
    let mut f = Fixture::new().await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base = format!("http://{address}");
    let server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    f.p.rail_client = bank_rails::RailClient::new(&base)?;
    let (maker, _) = rail_policy(&f).await?;
    let settled = rail_draft(&f, 1000).await?;
    let rejected = rail_draft(&f, 2000).await?;
    let cancelled = rail_draft(&f, 3000).await?;
    f.p.process(settled.id).await?;
    f.p.process(rejected.id).await?;
    f.p.process(cancelled.id).await?;
    let instruction = f.instruction(&settled).await?;
    let work: bank_rails::RailInstruction = serde_json::from_value(
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT document FROM payments.rail_submissions WHERE payment_id=$1",
        )
        .bind(settled.id)
        .fetch_one(&f.p.pool)
        .await?,
    )?;
    let ack = f.p.rail_client.inquire(&work).await?;
    assert!(
        f.p.core
            .finalize_external(&instruction, &ack.document)
            .await
            .is_err()
    );
    f.p.core
        .restrict(&f.alice, &f.p.controls, f.from, true, false)
        .await?;
    f.p.pause_rail(&maker).await?;
    let response = reqwest::Client::new()
        .post(format!("{base}/admin/rail-events"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&json!({"instructionId":settled.id,"status":"SETTLED"}))
        .send()
        .await?
        .error_for_status()?;
    let packet: SignedDocument = response.json().await?;
    let mut forged = packet.clone();
    forged.body["amount"]["minorUnits"] = json!("999");
    assert!(f.p.accept_rail_evidence(&forged).await.is_err());
    let wrong_scope = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &forged.body)?;
    assert!(f.p.accept_rail_evidence(&wrong_scope).await.is_err());
    assert_eq!(f.p.core.balance(f.from).await?.booked, money(100000));
    f.p.accept_rail_evidence(&packet).await?;
    f.p.accept_rail_evidence(&packet).await?;
    assert!(
        f.p.reconcile_external_with_ack_fault(settled.id, true)
            .await
            .is_err()
    );
    assert_eq!(f.p.detail(&f.alice, settled.id).await?.state, "RECONCILING");
    let booked = f.p.core.payment_receipt(&instruction).await?.unwrap();
    assert!(matches!(f.p.process(settled.id).await?,Progress::Done{state} if state=="SETTLED"));
    assert_eq!(
        f.p.detail(&f.alice, settled.id).await?.batch_id,
        Some(booked)
    );
    assert!(
        sqlx::query("SELECT * FROM core.reverse_batch($1,$2,$3,$4,$5)")
            .bind(Uuid::new_v4())
            .bind(booked)
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .bind(Uuid::new_v4())
            .execute(&f.p.core.pool)
            .await
            .is_err()
    );
    let balance = f.p.core.balance(f.from).await?;
    assert_eq!(balance.booked, money(100000 - settled.total.minor_units()));
    assert_eq!(
        balance.held,
        money(rejected.total.minor_units() + cancelled.total.minor_units())
    );
    // A delayed acknowledgment does not downgrade the immutable settled financial result.
    f.p.accept_rail_evidence(&ack.document).await?;
    assert_eq!(f.p.detail(&f.alice, settled.id).await?.state, "SETTLED");
    let mut contradiction: bank_rails::RailEvidence = serde_json::from_value(packet.body.clone())?;
    contradiction.status = bank_rails::RailStatus::Cancelled;
    contradiction.settlement_date = None;
    contradiction.version += 1;
    contradiction.evidence_id = Uuid::new_v4();
    let contradiction = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &contradiction)?;
    assert!(f.p.accept_rail_evidence(&contradiction).await.is_err());
    assert!(
        f.p.core
            .finalize_external(&instruction, &contradiction)
            .await
            .is_err()
    );
    assert_eq!(f.p.core.balance(f.from).await?.booked, balance.booked);
    let response = reqwest::Client::new()
        .post(format!("{base}/admin/rail-events"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&json!({"instructionId":rejected.id,"status":"REJECTED"}))
        .send()
        .await?
        .error_for_status()?;
    let failure: SignedDocument = response.json().await?;
    f.p.accept_rail_evidence(&failure).await?;
    f.p.process(rejected.id).await?;
    f.p.process(rejected.id).await?;
    assert_eq!(f.p.detail(&f.alice, rejected.id).await?.state, "DECLINED");
    assert_eq!(f.p.core.balance(f.from).await?.held, cancelled.total);
    let cancel_work: bank_rails::RailInstruction = serde_json::from_value(
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT document FROM payments.rail_submissions WHERE payment_id=$1",
        )
        .bind(cancelled.id)
        .fetch_one(&f.p.pool)
        .await?,
    )?;
    let cancellation = f.p.rail_client.cancel(&cancel_work).await?;
    f.p.accept_rail_evidence(&cancellation.document).await?;
    f.p.process(cancelled.id).await?;
    assert_eq!(f.p.detail(&f.alice, cancelled.id).await?.state, "CANCELLED");
    assert_eq!(f.p.core.balance(f.from).await?.held, money(0));
    assert_eq!(f.p.core.balance(f.from).await?.booked, balance.booked);
    // A provider cancellation attempted after final settlement returns that original
    // settlement. It cannot release a consumed hold or undo the journal.
    let too_late = f.p.rail_client.cancel(&work).await?;
    assert_eq!(too_late.evidence.status, bank_rails::RailStatus::Settled);
    f.p.accept_rail_evidence(&too_late.document).await?;
    f.p.process(settled.id).await?;
    let unchanged = f.p.detail(&f.alice, settled.id).await?;
    assert_eq!(unchanged.state, "SETTLED");
    assert_eq!(unchanged.batch_id, Some(booked));
    let total: String =
        sqlx::query_scalar("SELECT sum(delta::numeric)::text FROM core.journal_entries")
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(total, "0");
    let batches: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core.journal_batches WHERE operation_id=$1")
            .bind(settled.id)
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(batches, 1);
    drop(server);
    f.close().await
}

struct CreditPermit;
impl bank_core::credit::CreditPolicy for CreditPermit {
    fn validate<'a>(
        &'a self,
        e: &'a bank_rails::CreditEvidence,
    ) -> BoxFuture<'a, Result<ExecutionPermit>> {
        Box::pin(async move {
            Ok(ExecutionPermit {
                request_digest: fingerprint(e)?,
                valid_until: Utc::now() + Duration::minutes(1),
            })
        })
    }
}
fn incoming(reference: String, amount: i64) -> bank_rails::CreditEvidence {
    bank_rails::CreditEvidence {
        event_type: bank_rails::CreditKind::InboundSettled,
        movement_id: Uuid::new_v4(),
        evidence_id: Uuid::new_v4(),
        provider: bank_rails::PROVIDER.into(),
        rail: bank_rails::RAIL.into(),
        original_payment_id: None,
        receiving_reference: Some(reference),
        amount: money(amount),
        business_date: Utc::now().date_naive(),
        occurred_at: Utc::now(),
        version: 1,
        reason_code: "SYNTHETIC_INBOUND".into(),
    }
}
#[tokio::test]
async fn incoming_and_partial_returns_use_suspense_exact_credit_and_atomic_original_principal_caps()
-> Result<()> {
    let f = Fixture::new().await?;
    let reference =
        f.p.core
            .receiving_reference(&f.bob, &f.p.controls, f.to)
            .await?;
    let inbound = incoming(reference, 500);
    let packet = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &inbound)?;
    let saved = f.p.core.record_rail_credit(&packet).await?;
    assert!(saved.allocation_batch_id.is_none());
    assert_eq!(f.p.core.balance(f.to).await?.available, money(0));
    f.p.core.record_rail_credit(&packet).await?;
    let credited =
        f.p.core
            .allocate_rail_credit(&inbound, &CreditPermit)
            .await?;
    assert!(credited.allocation_batch_id.is_some());
    f.p.core
        .allocate_rail_credit(&inbound, &CreditPermit)
        .await?;
    assert_eq!(f.p.core.balance(f.to).await?.available, money(500));
    let unknown = incoming("PB00000000000000000000000000000000".into(), 200);
    let unknown_packet = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &unknown)?;
    assert!(
        f.p.core
            .record_rail_credit(&unknown_packet)
            .await?
            .target_account_id
            .is_none()
    );
    assert!(
        f.p.core
            .allocate_rail_credit(&unknown, &CreditPermit)
            .await
            .is_err()
    );
    // A bank-side credit restriction is fixture setup, separate from the customer's debit-freeze action.
    sqlx::query("SELECT core.restrict_account($1,false,true,false)")
        .bind(f.to.0)
        .execute(&f.db.migrator)
        .await?;
    let restricted = incoming(inbound.receiving_reference.clone().unwrap(), 100);
    f.p.core
        .record_rail_credit(&SignedDocument::sign(
            bank_rails::signing_key().as_bytes(),
            &restricted,
        )?)
        .await?;
    assert!(
        f.p.core
            .allocate_rail_credit(&restricted, &CreditPermit)
            .await
            .is_err()
    );
    assert_eq!(f.p.core.balance(f.to).await?.booked, money(500));
    sqlx::query("SELECT core.restrict_account($1,false,false,false)")
        .bind(f.to.0)
        .execute(&f.db.migrator)
        .await?;
    // Establish one authenticated outbound settlement, then receive multiple original-linked returns.
    let draft = f.draft(1000).await?;
    let mut instruction = f.instruction(&draft).await?;
    instruction.details.rail = "SIMULATED".into();
    instruction.payee_account_id = AccountId(bank_payments::policy::SIMULATED_CLEARING);
    instruction.rail_policy_version_id = Some(Uuid::new_v4());
    let reserved =
        f.p.core
            .reserve_external_payment(
                &instruction,
                &FinancialPermit(Utc::now() + Duration::minutes(1)),
            )
            .await?;
    f.p.core
        .mark_external_submitted(&instruction, reserved.authority_until)
        .await?;
    let finality = bank_rails::RailEvidence {
        evidence_id: Uuid::new_v4(),
        provider: bank_rails::PROVIDER.into(),
        instruction_id: instruction.operation_id,
        payment_id: instruction.operation_id,
        input_digest: fingerprint(&instruction)?,
        amount: money(1000),
        status: bank_rails::RailStatus::Settled,
        version: 2,
        occurred_at: Utc::now(),
        settlement_date: Some(Utc::now().date_naive()),
        reason_code: "SYNTHETIC_SETTLED".into(),
    };
    let original =
        f.p.core
            .finalize_external(
                &instruction,
                &SignedDocument::sign(bank_rails::signing_key().as_bytes(), &finality)?,
            )
            .await?;
    let before = f.p.core.balance(f.from).await?.booked;
    let mut returned = incoming("PB00000000000000000000000000000000".into(), 600);
    returned.event_type = bank_rails::CreditKind::ReturnSettled;
    returned.receiving_reference = None;
    returned.original_payment_id = Some(instruction.operation_id);
    let first = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &returned)?;
    let r = f.p.core.record_rail_credit(&first).await?;
    assert_eq!(r.original_payment_id, Some(instruction.operation_id));
    let mut second = returned.clone();
    second.movement_id = Uuid::new_v4();
    second.evidence_id = Uuid::new_v4();
    second.amount = money(500);
    let excessive = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &second)?;
    assert!(f.p.core.record_rail_credit(&excessive).await.is_err());
    second.amount = money(400);
    let second_packet = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &second)?;
    f.p.core.record_rail_credit(&second_packet).await?;
    f.p.core
        .allocate_rail_credit(&returned, &CreditPermit)
        .await?;
    f.p.core
        .allocate_rail_credit(&second, &CreditPermit)
        .await?;
    f.p.core.record_rail_credit(&first).await?;
    assert_eq!(
        f.p.core.balance(f.from).await?.booked,
        money(before.minor_units() + 1000)
    );
    assert_eq!(
        f.p.core.payment_receipt(&instruction).await?,
        original.batch_id
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core.credit_movements WHERE original_payment_id=$1",
    )
    .bind(instruction.operation_id)
    .fetch_one(&f.db.migrator)
    .await?;
    assert_eq!(count, 2);
    let mut cross = incoming(inbound.receiving_reference.unwrap(), 1);
    cross.evidence_id = finality.evidence_id;
    assert!(
        f.p.core
            .record_rail_credit(&SignedDocument::sign(
                bank_rails::signing_key().as_bytes(),
                &cross
            )?)
            .await
            .is_err()
    );
    let budget: String =
        sqlx::query_scalar("SELECT state FROM core.payment_limit_usage WHERE operation_id=$1")
            .bind(instruction.operation_id)
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(budget, "POSTED");
    assert_eq!(
        f.p.core
            .balance(AccountId(Uuid::parse_str(
                "4f1a780b-3e31-48ef-a294-000000000004"
            )?))
            .await?
            .booked,
        money(10)
    );
    // Competing partial returns serialize against the original principal even before either allocation.
    let mut second_original = instruction.clone();
    second_original.operation_id = Uuid::new_v4();
    let reserved =
        f.p.core
            .reserve_external_payment(
                &second_original,
                &FinancialPermit(Utc::now() + Duration::minutes(1)),
            )
            .await?;
    f.p.core
        .mark_external_submitted(&second_original, reserved.authority_until)
        .await?;
    let mut settled_again = finality.clone();
    settled_again.evidence_id = Uuid::new_v4();
    settled_again.instruction_id = second_original.operation_id;
    settled_again.payment_id = second_original.operation_id;
    settled_again.input_digest = fingerprint(&second_original)?;
    f.p.core
        .finalize_external(
            &second_original,
            &SignedDocument::sign(bank_rails::signing_key().as_bytes(), &settled_again)?,
        )
        .await?;
    let mut a = returned.clone();
    a.movement_id = Uuid::new_v4();
    a.evidence_id = Uuid::new_v4();
    a.original_payment_id = Some(second_original.operation_id);
    let mut b = a.clone();
    b.movement_id = Uuid::new_v4();
    b.evidence_id = Uuid::new_v4();
    let pa = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &a)?;
    let pb = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &b)?;
    let (a, b) = tokio::join!(
        f.p.core.record_rail_credit(&pa),
        f.p.core.record_rail_credit(&pb)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let total: String =
        sqlx::query_scalar("SELECT sum(delta::numeric)::text FROM core.journal_entries")
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(total, "0");
    f.close().await
}

async fn allow_credit(
    f: &Fixture,
    customer: &ActorContext,
    e: &bank_rails::CreditEvidence,
) -> Result<()> {
    for kind in Purpose::Payment.required() {
        let request =
            f.p.checks
                .request(customer, e.movement_id, *kind, &fingerprint(e)?)
                .await?;
        let result = CheckResult {
            request,
            outcome: Outcome::Allow,
            evidence_id: Uuid::new_v4(),
            expires_at: Utc::now() + Duration::minutes(5),
            reason_code: "SYNTHETIC_FIXTURE".into(),
        };
        f.p.checks
            .record(&SignedDocument::sign(simulator_key().as_bytes(), &result)?)
            .await?;
    }
    Ok(())
}
#[tokio::test]
async fn credit_ingress_controls_owned_suspense_closed_destinations_and_lost_allocation_ack_are_recoverable()
-> Result<()> {
    let f = Fixture::new().await?;
    let reference =
        f.p.core
            .receiving_reference(&f.alice, &f.p.controls, f.from)
            .await?;
    let e = incoming(reference, 500);
    let packet = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &e)?;
    f.p.accept_rail_evidence(&packet).await?;
    f.p.accept_rail_evidence(&packet).await?;
    assert!(matches!(
        f.p.process_credit(e.movement_id).await?,
        Progress::Pending
    ));
    assert_eq!(f.p.core.balance(f.from).await?.available, money(100000));
    assert!(
        f.p.core
            .credit_record(e.movement_id)
            .await?
            .unwrap()
            .allocation_batch_id
            .is_none()
    );
    allow_credit(&f, &f.alice, &e).await?;
    assert!(
        f.p.process_credit_with_ack_fault(e.movement_id, true)
            .await
            .is_err()
    );
    assert_eq!(f.p.core.balance(f.from).await?.available, money(100500));
    assert!(
        matches!(f.p.process_credit(e.movement_id).await?,Progress::Done{state} if state=="CREDITED")
    );
    let status = f.p.credit_status(e.movement_id).await?;
    assert_eq!(status.state, "CREDITED");
    assert_eq!(f.p.customer_credits(&f.alice).await?.len(), 1);
    assert!(f.p.customer_credits(&f.bob).await?.is_empty());
    let mut changed = e.clone();
    changed.amount = money(501);
    assert!(
        f.p.accept_rail_evidence(&SignedDocument::sign(
            bank_rails::signing_key().as_bytes(),
            &changed
        )?)
        .await
        .is_err()
    );
    assert_eq!(f.p.core.balance(f.from).await?.available, money(100500));
    let bob_reference =
        f.p.core
            .receiving_reference(&f.bob, &f.p.controls, f.to)
            .await?;
    f.p.core.close_account(&f.bob, &f.p.controls, f.to).await?;
    let closed = incoming(bob_reference, 200);
    let packet = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &closed)?;
    f.p.accept_rail_evidence(&packet).await?;
    assert!(matches!(
        f.p.process_credit(closed.movement_id).await?,
        Progress::Review { .. }
    ));
    assert_eq!(
        f.p.credit_status(closed.movement_id).await?.state,
        "DESTINATION_RESTRICTED"
    );
    assert_eq!(f.p.core.balance(f.to).await?.available, money(0));
    let case_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM controls.cases WHERE origin_id=$1 AND state<>'CLOSED'",
    )
    .bind(closed.movement_id)
    .fetch_one(&f.db.migrator)
    .await?;
    assert_eq!(case_count, 1);
    let unknown = incoming("PB00000000000000000000000000000000".into(), 300);
    f.p.accept_rail_evidence(&SignedDocument::sign(
        bank_rails::signing_key().as_bytes(),
        &unknown,
    )?)
    .await?;
    f.p.process_credit(unknown.movement_id).await?;
    assert_eq!(
        f.p.credit_status(unknown.movement_id).await?.state,
        "DESTINATION_RESTRICTED"
    );
    assert!(
        f.p.core
            .credit_record(unknown.movement_id)
            .await?
            .unwrap()
            .target_account_id
            .is_none()
    );
    let total: String =
        sqlx::query_scalar("SELECT sum(delta::numeric)::text FROM core.journal_entries")
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(total, "0");
    f.close().await
}

#[tokio::test]
async fn only_owned_unconfirmed_drafts_can_be_cancelled_without_financial_effect() -> Result<()> {
    let f = Fixture::new().await?;
    let draft = f.draft(1000).await?;
    assert!(f.p.cancel_draft(&f.bob, draft.id).await.is_err());
    let cancelled = f.p.cancel_draft(&f.alice, draft.id).await?;
    assert_eq!(cancelled.state, "CANCELLED");
    assert!(cancelled.allowed_actions.is_empty());
    assert_eq!(
        f.p.cancel_draft(&f.alice, draft.id).await?.state,
        "CANCELLED"
    );
    assert!(matches!(f.p.process(draft.id).await?,Progress::Done{state} if state=="CANCELLED"));
    assert_eq!(f.p.core.balance(f.from).await?.available, money(100000));
    assert!(
        f.p.core
            .payment_receipt(&f.instruction(&draft).await?)
            .await?
            .is_none()
    );
    f.close().await
}

async fn recovery_operator(f: &Fixture, maker: &ActorContext) -> Result<()> {
    let person = f.p.controls.person(maker).await?;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'payment_inquiry',now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('payments-operations',$1,'REVIEW')").bind(person.0).execute(&f.db.migrator).await?;
    Ok(())
}
async fn rail_work(f: &Fixture, id: Uuid) -> Result<bank_rails::RailInstruction> {
    Ok(serde_json::from_value(
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT document FROM payments.rail_submissions WHERE payment_id=$1",
        )
        .bind(id)
        .fetch_one(&f.p.pool)
        .await?,
    )?)
}
#[tokio::test]
async fn durable_cancellation_covers_unsubmitted_release_loss_and_unknown_provider_tombstone()
-> Result<()> {
    let mut f = Fixture::new().await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base = format!("http://{address}");
    let server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    f.p.rail_client = bank_rails::RailClient::new(&base)?;
    let (maker, _) = rail_policy(&f).await?;
    let before = rail_draft(&f, 1000).await?;
    let original = f.instruction(&before).await?;
    f.p.core
        .reserve_external_payment(
            &original,
            &FinancialPermit(Utc::now() + Duration::minutes(1)),
        )
        .await?;
    let key = Uuid::new_v4();
    assert!(
        f.p.customer_cancellation(&f.bob, before.id, key)
            .await
            .is_err()
    );
    let command = f.p.customer_cancellation(&f.alice, before.id, key).await?;
    assert_eq!(
        f.p.customer_cancellation(&f.alice, before.id, key)
            .await?
            .id,
        command.id
    );
    // Core release committed, then the workflow acknowledgment was lost.
    f.p.core.release_external_unsubmitted(&original).await?;
    assert!(
        matches!(f.p.process_recovery(command.id).await?,Progress::Done{state} if state=="CANCELLED")
    );
    assert_eq!(
        f.p.detail(&f.alice, before.id)
            .await?
            .cancellation
            .unwrap()
            .reason_code,
        "CANCELLATION_CONFIRMED"
    );
    assert_eq!(f.p.core.balance(f.from).await?.held, money(0));
    let provider_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM simulator.rail_instructions")
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(provider_count, 0);
    let pending = rail_draft(&f, 2000).await?;
    reqwest::Client::new()
        .post(format!("{base}/admin/rail-scenarios"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&json!({"instructionId":pending.id,"mode":"BEFORE_ACCEPT_TIMEOUT"}))
        .send()
        .await?
        .error_for_status()?;
    let sender = f.p.clone();
    let pending_id = pending.id;
    let attempt = tokio::spawn(async move { sender.process(pending_id).await });
    let mut attempted = false;
    for _ in 0..100 {
        attempted=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM payments.rail_submissions WHERE payment_id=$1 AND attempted_at IS NOT NULL)").bind(pending.id).fetch_one(&f.p.pool).await?;
        if attempted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(attempted);
    let work = rail_work(&f, pending.id).await?;
    let cancel =
        f.p.customer_cancellation(&f.alice, pending.id, Uuid::new_v4())
            .await?;
    assert_ne!(f.p.detail(&f.alice, pending.id).await?.state, "CANCELLED");
    assert_eq!(f.p.core.balance(f.from).await?.held, pending.total);
    assert!(matches!(
        f.p.process_recovery(cancel.id).await?,
        Progress::Pending
    ));
    attempt.await??;
    f.p.pause_rail(&maker).await?;
    assert!(matches!(
        f.p.process_recovery(cancel.id).await?,
        Progress::Pending
    ));
    assert_eq!(f.p.core.balance(f.from).await?.held, pending.total);
    assert!(
        f.p.reconcile_external_with_ack_fault(pending.id, true)
            .await
            .is_err()
    );
    assert_eq!(f.p.core.balance(f.from).await?.held, money(0));
    assert!(
        matches!(f.p.process_recovery(cancel.id).await?,Progress::Done{state} if state=="CANCELLED")
    );
    assert_eq!(
        {
            reqwest::Client::new()
                .post(format!("{base}/admin/rail-scenarios"))
                .header("X-Rail-Key", bank_rails::signing_key())
                .json(&json!({"instructionId":pending.id,"mode":"ACCEPT"}))
                .send()
                .await?
                .error_for_status()?;
            f.p.rail_client.submit(&work).await?.evidence.status
        },
        bank_rails::RailStatus::Cancelled
    );
    assert_eq!(f.p.core.balance(f.from).await?.booked, money(100000));
    assert!(
        f.p.core
            .payment_receipt(&f.instruction(&pending).await?)
            .await?
            .is_none()
    );
    assert!(
        sqlx::query("UPDATE payments.recovery_requests SET action='INQUIRE' WHERE id=$1")
            .bind(cancel.id)
            .execute(&f.p.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM payments.recovery_history WHERE request_id=$1")
            .bind(cancel.id)
            .execute(&f.p.pool)
            .await
            .is_err()
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn operator_inquiry_repairs_aged_unknown_false_failure_and_lost_booking_ack_without_resubmission()
-> Result<()> {
    let mut f = Fixture::new().await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base = format!("http://{address}");
    let mut server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    f.p.rail_client = bank_rails::RailClient::new(&base)?;
    let (maker, _) = rail_policy(&f).await?;
    recovery_operator(&f, &maker).await?;
    let payment = rail_draft(&f, 1000).await?;
    reqwest::Client::new()
        .post(format!("{base}/admin/rail-scenarios"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&json!({"instructionId":payment.id,"mode":"LOST_RESPONSE"}))
        .send()
        .await?
        .error_for_status()?;
    f.p.process(payment.id).await?;
    // Only the synthetic migration principal can seed clock/state corruption.
    sqlx::query(
        "UPDATE core.holds SET created_at=now()-interval '10 minutes' WHERE operation_id=$1",
    )
    .bind(payment.id)
    .execute(&f.db.migrator)
    .await?;
    sqlx::query("UPDATE payments.intents SET state='DECLINED' WHERE id=$1")
        .bind(payment.id)
        .execute(&f.db.migrator)
        .await?;
    assert!(f.p.detail(&f.alice, payment.id).await.is_err());
    f.p.reconcile_positions().await?;
    let breaks = f.p.staff_reconciliation(&maker).await?;
    assert!(
        breaks
            .iter()
            .any(|b| b.code == "SUBMITTED_OUTCOME_OVERDUE" && b.case_id.is_some())
    );
    assert!(
        breaks
            .iter()
            .any(|b| b.code == "SUBMITTED_HOLD_WITHOUT_FINALITY" && b.case_id.is_some())
    );
    assert_eq!(f.p.core.balance(f.from).await?.held, payment.total);
    assert!(
        f.p.staff_recovery(
            &f.alice,
            payment.id,
            Uuid::new_v4(),
            bank_payments::RecoveryAction::Inquire,
            "Synthetic recovery"
        )
        .await
        .is_err()
    );
    let key = Uuid::new_v4();
    let command =
        f.p.staff_recovery(
            &maker,
            payment.id,
            key,
            bank_payments::RecoveryAction::Inquire,
            "Investigate original unknown outcome",
        )
        .await?;
    assert_eq!(
        f.p.staff_recovery(
            &maker,
            payment.id,
            key,
            bank_payments::RecoveryAction::Inquire,
            "Investigate original unknown outcome"
        )
        .await?
        .id,
        command.id
    );
    assert!(
        f.p.staff_recovery(
            &maker,
            payment.id,
            key,
            bank_payments::RecoveryAction::Inquire,
            "Changed operator reason"
        )
        .await
        .is_err()
    );
    assert!(
        f.p.staff_recovery(
            &maker,
            payment.id,
            Uuid::new_v4(),
            bank_payments::RecoveryAction::Inquire,
            "Duplicate budget without cooldown"
        )
        .await
        .is_err()
    );
    f.p.pause_rail(&maker).await?;
    drop(server);
    server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    reqwest::Client::new()
        .post(format!("{base}/admin/rail-events"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&json!({"instructionId":payment.id,"status":"SETTLED"}))
        .send()
        .await?
        .error_for_status()?;
    f.p.process_recovery(command.id).await?;
    assert!(
        f.p.reconcile_external_with_ack_fault(payment.id, true)
            .await
            .is_err()
    );
    let booked =
        f.p.core
            .payment_receipt(&f.instruction(&payment).await?)
            .await?
            .unwrap();
    assert!(
        matches!(f.p.process_recovery(command.id).await?,Progress::Done{state} if state=="SETTLED")
    );
    let recovered = f.p.detail(&f.alice, payment.id).await?;
    assert_eq!(recovered.batch_id, Some(booked));
    assert_eq!(recovered.state, "SETTLED");
    f.p.reconcile_positions().await?;
    f.p.reconcile_positions().await?;
    let matched = f.p.staff_reconciliation(&maker).await?;
    assert!(matched.iter().all(|b| b.state == "MATCHED"));
    for b in matched {
        let state: String = sqlx::query_scalar("SELECT state FROM controls.cases WHERE id=$1")
            .bind(b.case_id)
            .fetch_one(&f.db.migrator)
            .await?;
        assert_ne!(state, "CLOSED");
    }
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM payments.rail_submissions WHERE payment_id=$1")
            .bind(payment.id)
            .fetch_one(&f.p.pool)
            .await?;
    assert_eq!(attempts, 1);
    let too_late =
        f.p.staff_recovery(
            &maker,
            payment.id,
            Uuid::new_v4(),
            bank_payments::RecoveryAction::Cancel,
            "Cancellation after confirmed settlement",
        )
        .await?;
    f.p.process_recovery(too_late.id).await?;
    assert_eq!(f.p.detail(&f.alice, payment.id).await?.state, "SETTLED");
    assert_eq!(
        f.p.detail(&f.alice, payment.id).await?.batch_id,
        Some(booked)
    );
    assert_eq!(
        f.p.recovery_history(&maker, payment.id).await?[0].reason_code,
        "CANCELLATION_TOO_LATE_OR_NOT_REQUIRED"
    );
    let person = f.p.controls.person(&maker).await?;
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='payment_operator'").bind(person.0).execute(&f.db.migrator).await?;
    assert!(
        f.p.staff_recovery(
            &maker,
            payment.id,
            Uuid::new_v4(),
            bank_payments::RecoveryAction::Cancel,
            "Revoked operator action"
        )
        .await
        .is_err()
    );
    drop(server);
    f.close().await
}

#[tokio::test]
async fn exhausted_recovery_budget_owns_the_uncertain_hold_and_requires_another_audited_action()
-> Result<()> {
    let mut f = Fixture::new().await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base = format!("http://{address}");
    let server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    f.p.rail_client = bank_rails::RailClient::new(&base)?;
    let (maker, _) = rail_policy(&f).await?;
    recovery_operator(&f, &maker).await?;
    let payment = rail_draft(&f, 1000).await?;
    f.p.process(payment.id).await?;
    let command =
        f.p.staff_recovery(
            &maker,
            payment.id,
            Uuid::new_v4(),
            bank_payments::RecoveryAction::Inquire,
            "Provider unavailable; inquire original",
        )
        .await?;
    // Keep the deterministic harness focused on this command's actual lease/budget.
    sqlx::query("UPDATE platform.jobs SET state='DONE' WHERE operation_id<>$1")
        .bind(command.operation_id)
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE platform.jobs SET max_attempts=1 WHERE operation_id=$1")
        .bind(command.operation_id)
        .execute(&f.db.migrator)
        .await?;
    let lease = f.p.operations.claim(Uuid::new_v4(), 30).await?.unwrap();
    assert_eq!(lease.operation_id, command.operation_id.unwrap());
    drop(server);
    assert!(matches!(
        f.p.process_recovery(command.id).await?,
        Progress::Pending
    ));
    f.p.operations
        .retry(&lease, "PROVIDER_UNAVAILABLE", 0)
        .await?;
    f.p.recover().await?;
    assert_eq!(
        f.p.recovery_history(&maker, payment.id).await?[0].state,
        "REVIEW_REQUIRED"
    );
    assert_eq!(f.p.core.balance(f.from).await?.held, payment.total);
    let cases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM controls.cases WHERE origin_id=$1 AND state<>'CLOSED'",
    )
    .bind(payment.id)
    .fetch_one(&f.db.migrator)
    .await?;
    assert_eq!(cases, 1);
    let server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    let cancel =
        f.p.staff_recovery(
            &maker,
            payment.id,
            Uuid::new_v4(),
            bank_payments::RecoveryAction::Cancel,
            "Request definitive provider cancellation",
        )
        .await?;
    f.p.process_recovery(cancel.id).await?;
    f.p.process_recovery(cancel.id).await?;
    assert_eq!(f.p.detail(&f.alice, payment.id).await?.state, "CANCELLED");
    assert_eq!(f.p.core.balance(f.from).await?.held, money(0));
    drop(server);
    f.close().await
}

#[tokio::test]
async fn recovery_pages_do_not_starve_later_receipts_behind_fifty_unresolved_movements()
-> Result<()> {
    let f = Fixture::new().await?;
    for n in 1..=51 {
        let mut e = incoming("PB00000000000000000000000000000000".into(), 1);
        e.movement_id = Uuid::from_u128(n);
        f.p.accept_rail_evidence(&SignedDocument::sign(
            bank_rails::signing_key().as_bytes(),
            &e,
        )?)
        .await?;
    }
    // Simulate lost scheduling acknowledgments after Platform accepted each original job.
    sqlx::query("UPDATE payments.credit_intents SET operation_id=NULL")
        .execute(&f.db.migrator)
        .await?;
    f.p.recover().await?;
    let repaired: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.credit_intents WHERE operation_id IS NOT NULL",
    )
    .fetch_one(&f.p.pool)
    .await?;
    assert_eq!(repaired, 50);
    f.p.recover().await?;
    let repaired: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payments.credit_intents WHERE operation_id IS NOT NULL",
    )
    .fetch_one(&f.p.pool)
    .await?;
    assert_eq!(repaired, 51);
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM platform.jobs WHERE kind='credit.allocate'")
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(jobs, 51);
    f.close().await
}

#[tokio::test]
async fn reconciliation_owns_orphaned_submitted_commitments_and_seeded_hold_mismatches_without_releasing()
-> Result<()> {
    let f = Fixture::new().await?;
    let (maker, _) = rail_policy(&f).await?;
    recovery_operator(&f, &maker).await?;
    let draft = f.draft(1000).await?;
    let mut instruction = f.instruction(&draft).await?;
    instruction.operation_id = Uuid::new_v4();
    instruction.details.rail = "SIMULATED".into();
    instruction.payee_account_id = AccountId(bank_payments::policy::SIMULATED_CLEARING);
    instruction.rail_policy_version_id = Some(Uuid::new_v4());
    let reservation =
        f.p.core
            .reserve_external_payment(
                &instruction,
                &FinancialPermit(Utc::now() + Duration::minutes(1)),
            )
            .await?;
    f.p.core
        .mark_external_submitted(&instruction, reservation.authority_until)
        .await?;
    sqlx::query(
        "UPDATE core.holds SET created_at=now()-interval '10 minutes' WHERE operation_id=$1",
    )
    .bind(instruction.operation_id)
    .execute(&f.db.migrator)
    .await?;
    sqlx::query("UPDATE core.ledger_accounts SET held=held+1 WHERE id=$1")
        .bind(f.from.0)
        .execute(&f.db.migrator)
        .await?;
    f.p.reconcile_positions().await?;
    f.p.reconcile_positions().await?;
    f.p.reconcile_positions().await?;
    let breaks = f.p.staff_reconciliation(&maker).await?;
    for code in [
        "CORE_RESERVATION_WITHOUT_INTENT",
        "ACCOUNT_HOLD_TOTAL_MISMATCH",
        "SUBMITTED_OUTCOME_OVERDUE",
    ] {
        assert!(
            breaks.iter().any(|b| b.code == code
                && b.case_id.is_some()
                && b.state == "OPEN"
                && b.version == 1)
        );
    }
    assert!(f.p.staff_reconciliation(&f.alice).await.is_err());
    assert_eq!(
        f.p.core.balance(f.from).await?.held,
        money(reservation.amount.minor_units() + 1)
    );
    assert!(
        f.p.core
            .release_external_unsubmitted(&instruction)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM payments.reconciliation_history")
            .execute(&f.p.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM payments.reconciliation_breaks")
            .execute(&f.p.pool)
            .await
            .is_err()
    );
    f.close().await
}

#[tokio::test]
async fn payment_create_replay_repairs_committed_intent_before_challenge_ack_without_bypassing_step_up()
-> Result<()> {
    let f = Fixture::new().await?;
    let reference =
        f.p.core
            .receiving_reference(&f.bob, &f.p.controls, f.to)
            .await?;
    let recipient =
        f.p.add_beneficiary(
            &f.alice,
            Uuid::new_v4(),
            "Synthetic replay recipient",
            &reference,
        )
        .await?;
    let quote =
        f.p.quote(
            &f.alice,
            f.from,
            recipient.id,
            money(1000),
            Utc::now().date_naive(),
        )
        .await?;
    let key = Uuid::new_v4();
    // The durable Payments commit exists; Controls challenge creation/link acknowledgment was lost.
    sqlx::query("INSERT INTO payments.intents(id,issuer,subject,client_id,request_key,quote_id,actor_context,state) VALUES($1,$2,$3,$4,$5,$6,$7,'DRAFT')").bind(quote.payment_id).bind(&f.alice.issuer).bind(&f.alice.subject).bind(&f.alice.client_id).bind(key).bind(quote.id).bind(serde_json::to_value(&f.alice)?).execute(&f.db.migrator).await?;
    let (first, second) = tokio::join!(
        f.p.create_intent(&f.alice, key, quote.id),
        f.p.create_intent(&f.alice, key, quote.id)
    );
    let first = first?;
    let second = second?;
    assert_eq!(first.id, quote.payment_id);
    assert!(first.challenge_id.is_some());
    assert_eq!(first.challenge_id, second.challenge_id);
    assert!(!first.approval_ready);
    assert!(f.p.confirm(&f.alice, first.id).await.is_err());
    assert!(f.p.create_intent(&f.bob, key, quote.id).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM controls.approval_challenges WHERE operation_id=$1"
        )
        .bind(first.id)
        .fetch_one(&f.db.migrator)
        .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.payment_executions")
            .fetch_one(&f.db.migrator)
            .await?,
        0
    );
    f.close().await
}

async fn rail_admin(base: &str, path: &str, body: serde_json::Value) -> Result<reqwest::Response> {
    Ok(reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&body)
        .send()
        .await?
        .error_for_status()?)
}
fn forged_evidence(
    provider: &str,
    payment: &PaymentStatus,
    instruction: &bank_core::payments::InternalInstruction,
    status: bank_rails::RailStatus,
    version: i64,
    reason: &str,
) -> Result<bank_platform::authenticity::SignedDocument> {
    let settled = status == bank_rails::RailStatus::Settled;
    let evidence = bank_rails::RailEvidence {
        evidence_id: Uuid::new_v4(),
        provider: provider.into(),
        instruction_id: payment.id,
        payment_id: payment.id,
        input_digest: fingerprint(instruction)?,
        amount: instruction.details.amount,
        status,
        version,
        occurred_at: Utc::now(),
        settlement_date: settled.then(|| Utc::now().date_naive()),
        reason_code: reason.into(),
    };
    bank_platform::authenticity::SignedDocument::sign(
        bank_rails::provider_key(provider).unwrap().as_bytes(),
        &evidence,
    )
}
async fn count(f: &Fixture, sql: &'static str, id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(&f.db.migrator)
        .await?)
}
#[tokio::test]
async fn routing_falls_back_only_on_proven_nonexecution_books_once_and_policies_need_independent_shadowed_approval()
-> Result<()> {
    let mut f = Fixture::new().await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base = format!("http://{address}");
    let server = rail_server(&f, address)?;
    wait_rail(&base).await?;
    f.p.rail_client = bank_rails::RailClient::new(&base)?;
    let (maker, checker) = rail_policy(&f).await?;
    const AT_A: &str = "SELECT count(*) FROM simulator.rail_instructions WHERE id=$1";
    const AT_B: &str = "SELECT count(*) FROM simulator.rail_b_instructions WHERE id=$1";
    const ROUTES: &str = "SELECT count(*) FROM core.external_routes WHERE operation_id=$1";
    const BOOKED: &str = "SELECT count(*) FROM core.journal_batches WHERE operation_id=$1 AND posting_kind='EXTERNAL_PAYMENT'";

    // 1. Signed capacity non-execution at A: same instruction re-routed to B, settled there, booked once.
    let p1 = rail_draft(&f, 1000).await?;
    let i1 = f.instruction(&p1).await?;
    rail_admin(
        &base,
        "/admin/rail-scenarios",
        json!({"instructionId":p1.id,"mode":"REJECT_CAPACITY"}),
    )
    .await?;
    let before = f.p.core.balance(f.from).await?;
    f.p.process(p1.id).await?;
    assert_eq!(f.p.detail(&f.alice, p1.id).await?.state, "SUBMITTING");
    assert_eq!(count(&f, ROUTES, p1.id).await?, 2);
    assert_eq!(
        f.p.core.external_reservation(&i1).await?.unwrap().state,
        "ACTIVE",
        "hold kept across the re-route"
    );
    f.p.process(p1.id).await?;
    assert_eq!(f.p.detail(&f.alice, p1.id).await?.state, "SUBMITTED");
    assert_eq!(
        (count(&f, AT_A, p1.id).await?, count(&f, AT_B, p1.id).await?),
        (1, 1)
    );
    let b_sent: serde_json::Value =
        sqlx::query_scalar("SELECT instruction FROM simulator.rail_b_instructions WHERE id=$1")
            .bind(p1.id)
            .fetch_one(&f.db.migrator)
            .await?;
    let a_sent: serde_json::Value =
        sqlx::query_scalar("SELECT instruction FROM simulator.rail_instructions WHERE id=$1")
            .bind(p1.id)
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(
        a_sent, b_sent,
        "the same economic instruction goes to each provider"
    );
    let settled: bank_platform::authenticity::SignedDocument = rail_admin(
        &base,
        "/admin/rail-b-events",
        json!({"instructionId":p1.id,"status":"SETTLED"}),
    )
    .await?
    .json()
    .await?;
    f.p.accept_rail_evidence(&settled).await?;
    f.p.process(p1.id).await?;
    assert_eq!(f.p.detail(&f.alice, p1.id).await?.state, "SETTLED");
    assert_eq!(count(&f, BOOKED, p1.id).await?, 1);
    assert_eq!(
        f.p.core.balance(f.from).await?.booked,
        money(before.booked.minor_units() - p1.total.minor_units())
    );
    // 2. Provider A contradicting its own non-execution is quarantined; Core never books it.
    let lie = forged_evidence(
        bank_rails::PROVIDER,
        &p1,
        &i1,
        bank_rails::RailStatus::Settled,
        2,
        "SIMULATED_RAIL_RESULT",
    )?;
    assert!(f.p.accept_rail_evidence(&lie).await.is_err());
    assert_eq!(count(&f, "SELECT count(*) FROM payments.rail_quarantine WHERE payment_id=$1 AND reason_code='IMMUTABLE_PROVIDER_CONFLICT'", p1.id).await?, 1);
    assert!(f.p.core.finalize_external(&i1, &lie).await.is_err());
    assert_eq!(count(&f, BOOKED, p1.id).await?, 1);
    // 3. Unknown outcome at A is not proof of non-execution: no fallback; inquiry resolves at A.
    let p2 = rail_draft(&f, 1100).await?;
    rail_admin(
        &base,
        "/admin/rail-scenarios",
        json!({"instructionId":p2.id,"mode":"LOST_RESPONSE"}),
    )
    .await?;
    f.p.process(p2.id).await?;
    assert_eq!(f.p.detail(&f.alice, p2.id).await?.state, "OUTCOME_UNKNOWN");
    f.p.process(p2.id).await?;
    assert_eq!(f.p.detail(&f.alice, p2.id).await?.state, "SUBMITTED");
    assert_eq!(
        (
            count(&f, AT_B, p2.id).await?,
            count(&f, ROUTES, p2.id).await?
        ),
        (0, 1)
    );
    // 4. An ordinary rejection is final: no fallback, hold released; Core refuses a re-route on that evidence.
    let p3 = rail_draft(&f, 1200).await?;
    let i3 = f.instruction(&p3).await?;
    rail_admin(
        &base,
        "/admin/rail-scenarios",
        json!({"instructionId":p3.id,"mode":"REJECT"}),
    )
    .await?;
    f.p.process(p3.id).await?;
    let ordinary = forged_evidence(
        bank_rails::PROVIDER,
        &p3,
        &i3,
        bank_rails::RailStatus::Rejected,
        1,
        "SIMULATED_RAIL_RESULT",
    )?;
    assert!(
        f.p.core
            .route_external(&i3, bank_rails::PROVIDER_B, Some(&ordinary))
            .await
            .is_err()
    );
    f.p.process(p3.id).await?;
    assert_eq!(f.p.detail(&f.alice, p3.id).await?.state, "DECLINED");
    assert_eq!(
        (
            count(&f, AT_B, p3.id).await?,
            count(&f, BOOKED, p3.id).await?
        ),
        (0, 0)
    );

    // 5. Policy governance: independent approver, shadow evidence before activation, rollback without shadow.
    let alias = actor("maker-alias", TrustContext::Workforce);
    let maker_person = f.p.controls.person(&maker).await?;
    f.p.controls.link_person(&alias, maker_person).await?;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'payment_approver',now()+interval '1 day')").bind(Uuid::new_v4()).bind(maker_person.0).execute(&f.db.migrator).await?;
    let weighted = json!({"strategy":"HEALTH_WEIGHTED","providers":["SYNTHETIC_RAIL_B","SYNTHETIC_RAIL"],"fallbackReasons":[],"maxAttempts":1,
        "health":{"windowMinutes":60,"minSamples":1,"maxUnknownRate":0.5,"maxNonexecutionRate":0.5},"feesMinor":{"SYNTHETIC_RAIL":3,"SYNTHETIC_RAIL_B":2},"promotionMinDecisions":2});
    let mut bad = weighted.clone();
    bad["fallbackReasons"] = json!(["SIMULATED_RAIL_RESULT"]);
    assert!(
        f.p.propose_route_change(&maker, Some(&bad), None, "SHADOW")
            .await
            .is_err(),
        "fallback only on Core non-execution reasons"
    );
    let mut extra = weighted.clone();
    extra["skipStepUp"] = json!(true);
    assert!(
        f.p.propose_route_change(&maker, Some(&extra), None, "SHADOW")
            .await
            .is_err(),
        "policies cannot touch approvals"
    );
    assert!(
        f.p.propose_route_change(&checker, Some(&weighted), None, "SHADOW")
            .await
            .is_err(),
        "proposer needs the operator role"
    );
    let early =
        f.p.propose_route_change(&maker, Some(&weighted), None, "ACTIVE")
            .await?;
    let v2 = early["version"].as_i64().unwrap() as i32;
    let early_id: Uuid = serde_json::from_value(early["changeId"].clone())?;
    assert!(
        f.p.decide_route_change(&alias, early_id, true)
            .await
            .is_err(),
        "the proposer cannot approve"
    );
    assert!(
        format!(
            "{:#}",
            f.p.decide_route_change(&checker, early_id, true)
                .await
                .unwrap_err()
        )
        .contains("Shadow evaluation required")
    );
    let shadow =
        f.p.propose_route_change(&maker, None, Some(v2), "SHADOW")
            .await?;
    f.p.decide_route_change(
        &checker,
        serde_json::from_value(shadow["changeId"].clone())?,
        true,
    )
    .await?;
    for amount in [1300, 1400] {
        let p = rail_draft(&f, amount).await?;
        f.p.process(p.id).await?;
        let shadowed: (i32, Option<i32>, Option<String>) = sqlx::query_as("SELECT policy_version,shadow_version,shadow_provider FROM payments.route_decisions WHERE payment_id=$1 AND attempt=1")
            .bind(p.id).fetch_one(&f.db.migrator).await?;
        assert_eq!((shadowed.0, shadowed.1), (1, Some(v2)));
        assert!(shadowed.2.is_some());
    }
    let view = f.p.routing_view(&checker).await?;
    assert_eq!(view["head"]["shadowVersion"], v2);
    assert_eq!(view["shadow"]["decisions"], 2);
    assert!(
        view["shadow"]["baseline"]
            .as_str()
            .unwrap()
            .starts_with("LOCAL_SIMULATOR_ONLY")
    );
    let promote =
        f.p.propose_route_change(&maker, None, Some(v2), "ACTIVE")
            .await?;
    let applied =
        f.p.decide_route_change(
            &checker,
            serde_json::from_value(promote["changeId"].clone())?,
            true,
        )
        .await?;
    assert_eq!(applied["head"]["active_version"], v2);
    assert!(applied["head"]["shadow_version"].is_null());
    // 6. An instruction decided under v2 keeps v2's rules (no fallback) after a rollback to v1.
    let p4 = rail_draft(&f, 1500).await?;
    let i4 = f.instruction(&p4).await?;
    f.p.process(p4.id).await?;
    let decided: (i32, String) = sqlx::query_as("SELECT policy_version,strategy FROM payments.route_decisions WHERE payment_id=$1 AND attempt=1").bind(p4.id).fetch_one(&f.db.migrator).await?;
    assert_eq!(decided, (v2, "HEALTH_WEIGHTED".into()));
    let rollback =
        f.p.propose_route_change(&maker, None, Some(1), "ACTIVE")
            .await?;
    f.p.decide_route_change(
        &checker,
        serde_json::from_value(rollback["changeId"].clone())?,
        true,
    )
    .await?;
    assert_eq!(
        f.p.routing_view(&checker).await?["head"]["activeVersion"],
        1
    );
    let provider: String =
        sqlx::query_scalar("SELECT provider FROM payments.rail_submissions WHERE payment_id=$1")
            .bind(p4.id)
            .fetch_one(&f.db.migrator)
            .await?;
    let capacity = forged_evidence(
        &provider,
        &p4,
        &i4,
        bank_rails::RailStatus::Rejected,
        2,
        "PROVIDER_CAPACITY_NOT_EXECUTED",
    )?;
    f.p.accept_rail_evidence(&capacity).await?;
    f.p.process(p4.id).await?;
    assert_eq!(f.p.detail(&f.alice, p4.id).await?.state, "DECLINED");
    assert_eq!(
        count(&f, ROUTES, p4.id).await?,
        1,
        "v2 allows no fallback, even after rollback to v1"
    );
    drop(server);
    f.close().await
}
