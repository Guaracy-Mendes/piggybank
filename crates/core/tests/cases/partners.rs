//! WP-33: FR-ACC-08 delegated authority and FR-IAM-05/FR-EXT-02 partner clients, consents, quotas and callbacks.
use anyhow::Result;
use bank_controls::entitlements::{Action, Entitlements};
use bank_core::Core;
use bank_identity::consents::Partners;
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, PartyId, TrustContext},
    testing::TestDatabase,
};
use serde_json::{Value, json};
use uuid::Uuid;

struct P {
    db: TestDatabase,
    core: Core,
    c: Entitlements,
    partners: Partners,
    alice: ActorContext,
    bob: ActorContext,
    carol: ActorContext,
    admin: ActorContext,
    admin2: ActorContext,
    alice_account: Uuid,
    bob_account: Uuid,
}
fn actor(subject: &str, trust: TrustContext) -> ActorContext {
    ActorContext {
        issuer: "urn:partner-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "fixture".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    }
}
fn uuid(v: &Value) -> Uuid {
    serde_json::from_value(v.clone()).unwrap()
}
impl P {
    async fn new() -> Result<Self> {
        let db = TestDatabase::new().await?;
        let core = Core::new(db.module(Module::Core).await?);
        let c = Entitlements::new(db.module(Module::Controls).await?);
        let partners = Partners::new(db.module(Module::Identity).await?);
        let alice = actor("alice", TrustContext::Customer);
        let bob = actor("bob", TrustContext::Customer);
        let carol = actor("carol", TrustContext::Customer);
        let admin = actor("admin", TrustContext::Workforce);
        let admin2 = actor("admin2", TrustContext::Workforce);
        for (who, role) in [
            (&alice, None),
            (&bob, None),
            (&carol, None),
            (&admin, Some("security_admin")),
            (&admin2, Some("security_admin")),
        ] {
            let person = Uuid::new_v4();
            sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
                .bind(&who.issuer)
                .bind(&who.subject)
                .bind(person)
                .execute(&db.migrator)
                .await?;
            if let Some(role) = role {
                sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
            }
        }
        let product = Uuid::new_v4();
        sqlx::query("INSERT INTO core.product_versions VALUES($1,'UI_FIXTURE',1,'2026-01-01','2030-01-01','{\"label\":\"SYNTHETIC\",\"chartVersion\":\"synthetic-chart-v1\"}',$2,$3,$4,now())").bind(product).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(Uuid::new_v4()).execute(&db.migrator).await?;
        let mut accounts = vec![];
        for who in [&alice, &bob] {
            let person = c.person(who).await?.0;
            let (party, account) = (Uuid::new_v4(), Uuid::new_v4());
            sqlx::query("INSERT INTO core.parties(id,issuer,subject,source_application,identity_digest) VALUES($1,$2,$3,$4,'fixture')").bind(party).bind(&who.issuer).bind(&who.subject).bind(Uuid::new_v4()).execute(&db.migrator).await?;
            sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'CUSTOMER','ACTIVE',false)").bind(account).execute(&db.migrator).await?;
            sqlx::query("INSERT INTO core.account_registry(id,application_id,party_id,product_version_id,acceptance_id,opening_digest) VALUES($1,$2,$3,$4,$5,'fixture')").bind(account).bind(Uuid::new_v4()).bind(party).bind(product).bind(Uuid::new_v4()).execute(&db.migrator).await?;
            sqlx::query("INSERT INTO core.account_relationships VALUES($1,$2,'OWNER',NULL)")
                .bind(account)
                .bind(person)
                .execute(&db.migrator)
                .await?;
            accounts.push(account);
        }
        Ok(Self {
            db,
            core,
            c,
            partners,
            alice,
            bob,
            carol,
            admin,
            admin2,
            alice_account: accounts[0],
            bob_account: accounts[1],
        })
    }
    async fn person(&self, a: &ActorContext) -> Result<PartyId> {
        self.c.person(a).await
    }
    async fn can(&self, a: &ActorContext, action: Action, account: Uuid) -> Result<bool> {
        let authority = self.core.authority(AccountId(account), action).await?;
        self.c.authorize(a, action, Some(&authority), account).await
    }
    async fn active_partner(
        &self,
        client: &str,
        scopes: Value,
        callback: Option<&str>,
    ) -> Result<Uuid> {
        let mut data = json!({"clientId": client, "displayName": "Synthetic Budget App", "allowedScopes": scopes, "quotaPerMinute": 3});
        if let Some(url) = callback {
            data["callbackUrl"] = json!(url);
        }
        let admin = self.person(&self.admin).await?;
        let (row, secret) = self
            .partners
            .propose(&self.admin, admin, Uuid::new_v4(), &data)
            .await?;
        assert_eq!(row["state"], "PENDING_APPROVAL");
        assert_eq!(
            secret.is_some(),
            callback.is_some(),
            "a signing secret is issued once, only with a callback"
        );
        let id = uuid(&row["id"]);
        // Maker/checker: the proposer cannot approve its own registration.
        assert!(
            self.partners
                .decide(&self.admin, admin, id, "APPROVE", "self approval")
                .await
                .is_err()
        );
        let admin2 = self.person(&self.admin2).await?;
        let row = self
            .partners
            .decide(
                &self.admin2,
                admin2,
                id,
                "APPROVE",
                "contract fixture reviewed",
            )
            .await?;
        assert_eq!(row["state"], "ACTIVE");
        Ok(id)
    }
}

#[tokio::test]
async fn delegation_grants_bounded_authority_and_revocation_is_prospective() -> Result<()> {
    let p = P::new().await?;
    let (alice, bob, carol) = (
        p.person(&p.alice).await?,
        p.person(&p.bob).await?,
        p.person(&p.carol).await?,
    );
    let reference = p.core.delegate_reference(bob).await?;
    let code = reference["reference"].as_str().unwrap().to_owned();
    assert!(code.starts_with("DLG-") && code.len() == 12);
    assert_eq!(
        p.core.delegate_reference(bob).await?["reference"],
        code,
        "an unexpired reference is reused"
    );
    // Before delegation Bob has no authority on Alice's account.
    assert!(!p.can(&p.bob, Action::ReadAccount, p.alice_account).await?);
    let key = Uuid::new_v4();
    let grant = json!({"accountId": p.alice_account, "reference": code, "authority": "PAY", "paymentLimitMinor": 5000, "durationDays": 30});
    let d = p.core.grant_delegation(alice, key, &grant).await?;
    assert_eq!(
        (d["authority"].as_str(), d["state"].as_str()),
        (Some("PAY"), Some("ACTIVE"))
    );
    assert_eq!(d["paymentLimit"]["minorUnits"], "5000");
    assert_eq!(
        p.core.grant_delegation(alice, key, &grant).await?["id"],
        d["id"],
        "same key replays"
    );
    // A second open delegation to the same person, self-delegation, a non-owner grant and a missing limit fail.
    assert!(
        p.core
            .grant_delegation(alice, Uuid::new_v4(), &grant)
            .await
            .is_err()
    );
    let own = p.core.delegate_reference(alice).await?["reference"].clone();
    assert!(p.core.grant_delegation(alice, Uuid::new_v4(), &json!({"accountId": p.alice_account, "reference": own, "authority": "VIEW", "durationDays": 1})).await.is_err());
    assert!(
        p.core
            .grant_delegation(carol, Uuid::new_v4(), &grant)
            .await
            .is_err()
    );
    assert!(p.core.grant_delegation(alice, Uuid::new_v4(), &json!({"accountId": p.alice_account, "reference": code, "authority": "PAY", "durationDays": 1})).await.is_err());
    // Bob may read and pay, never service the account; Carol still has nothing.
    assert!(p.can(&p.bob, Action::ReadAccount, p.alice_account).await?);
    assert!(
        p.can(&p.bob, Action::CreatePayment, p.alice_account)
            .await?
    );
    assert!(
        !p.can(&p.bob, Action::ServiceAccount, p.alice_account)
            .await?
    );
    assert!(
        !p.can(&p.carol, Action::ReadAccount, p.alice_account)
            .await?
    );
    let authority = p
        .core
        .authority(AccountId(p.alice_account), Action::CreatePayment)
        .await?;
    assert_eq!(
        authority.delegate(bob).and_then(|d| d.payment_limit_minor),
        Some(5000)
    );
    assert_eq!(p.core.delegated_accounts(bob).await?.len(), 1);
    assert!(p.core.delegated_accounts(alice).await?.is_empty());
    let lists = p.core.delegations_for(alice).await?;
    assert_eq!(
        (
            lists["granted"].as_array().unwrap().len(),
            lists["received"].as_array().unwrap().len()
        ),
        (1, 0)
    );
    // Grant terms are immutable even to the module role.
    let core_pool = p.db.module(Module::Core).await?;
    assert!(
        sqlx::query("UPDATE core.account_delegations SET payment_limit_minor=999999")
            .execute(&core_pool)
            .await
            .is_err()
    );
    // Carol cannot revoke; Alice can; the delegate loses authority at once and history keeps both events.
    let id = uuid(&d["id"]);
    assert!(
        p.core
            .revoke_delegation(carol, id, "not mine")
            .await
            .is_err()
    );
    assert_eq!(
        p.core.revoke_delegation(alice, id, "trip finished").await?["state"],
        "REVOKED"
    );
    assert!(!p.can(&p.bob, Action::ReadAccount, p.alice_account).await?);
    assert!(p.core.delegated_accounts(bob).await?.is_empty());
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT event FROM core.delegation_history WHERE delegation_id=$1 ORDER BY recorded_at",
    )
    .bind(id)
    .fetch_all(&p.db.migrator)
    .await?;
    assert_eq!(events, ["GRANTED", "REVOKED"]);
    // A view-only delegation reads but cannot pay; the delegate may renounce it.
    let view = p.core.grant_delegation(alice, Uuid::new_v4(), &json!({"accountId": p.alice_account, "reference": code, "authority": "VIEW", "durationDays": 7})).await?;
    assert!(p.can(&p.bob, Action::ReadAccount, p.alice_account).await?);
    assert!(
        !p.can(&p.bob, Action::CreatePayment, p.alice_account)
            .await?
    );
    p.core
        .revoke_delegation(bob, uuid(&view["id"]), "no longer needed")
        .await?;
    assert!(!p.can(&p.bob, Action::ReadAccount, p.alice_account).await?);
    p.db.close().await?;
    Ok(())
}

#[tokio::test]
async fn partner_consent_scopes_accounts_quota_and_revocation_hold() -> Result<()> {
    let p = P::new().await?;
    let client = "partner-budget";
    let partner = p
        .active_partner(client, json!(["accounts:read"]), None)
        .await?;
    // Requests are bounded by the registration; the same key replays; nothing is granted yet.
    assert!(
        p.partners
            .request_consent(
                client,
                Uuid::new_v4(),
                &json!({"scopes": ["payments:initiate"], "durationDays": 30})
            )
            .await
            .is_err()
    );
    assert!(
        p.partners
            .request_consent(
                "partner-unknown",
                Uuid::new_v4(),
                &json!({"scopes": ["accounts:read"], "durationDays": 30})
            )
            .await
            .is_err()
    );
    let key = Uuid::new_v4();
    let request = json!({"scopes": ["accounts:read"], "durationDays": 30});
    let pending = p.partners.request_consent(client, key, &request).await?;
    assert_eq!(pending["state"], "AWAITING_AUTHORIZATION");
    assert!(
        pending.get("personId").is_none(),
        "the partner view never names the customer"
    );
    assert_eq!(
        p.partners.request_consent(client, key, &request).await?["consentId"],
        pending["consentId"]
    );
    let consent = uuid(&pending["consentId"]);
    assert!(p.partners.binding(client, consent).await?.is_none());
    // Alice cannot consent to Bob's account; she authorizes her own.
    let alice = p.person(&p.alice).await?;
    assert!(
        p.partners
            .authorize(&p.alice, alice, consent, true, &[p.bob_account])
            .await
            .is_err()
    );
    let granted = p
        .partners
        .authorize(&p.alice, alice, consent, true, &[p.alice_account])
        .await?;
    assert_eq!(granted["state"], "ACTIVE");
    // Bob can no longer see or claim the consent once it is bound to Alice.
    let bob = p.person(&p.bob).await?;
    assert!(p.partners.customer_view(bob, consent).await.is_err());
    assert!(
        p.partners
            .authorize(&p.bob, bob, consent, true, &[p.bob_account])
            .await
            .is_err()
    );
    // A consented partner actor reads the consented account only, and cannot create payments with this scope.
    let binding = p
        .partners
        .binding(client, consent)
        .await?
        .expect("active binding");
    assert_eq!(binding.account_ids, vec![p.alice_account]);
    let partner_actor = binding.actor(client);
    assert!(
        p.can(&partner_actor, Action::ReadAccount, p.alice_account)
            .await?
    );
    assert!(
        !p.can(&partner_actor, Action::CreatePayment, p.alice_account)
            .await?
    );
    assert!(
        !p.can(&partner_actor, Action::ServiceAccount, p.alice_account)
            .await?
    );
    assert!(
        !p.can(&partner_actor, Action::ReadAccount, p.bob_account)
            .await?
    );
    // Another client presenting the same consent id gets nothing (client binding is checked in the database).
    let mut other = partner_actor.clone();
    other.client_id = "partner-other".into();
    assert!(!p.can(&other, Action::ReadAccount, p.alice_account).await?);
    assert!(
        p.partners
            .binding("partner-other", consent)
            .await?
            .is_none()
    );
    // Quota: three requests per minute for this client, then refusal.
    for _ in 0..3 {
        assert!(p.partners.quota_permits(client, 3).await?);
    }
    assert!(!p.partners.quota_permits(client, 3).await?);
    // Customer revocation applies to the very next decision; the customer's own history remains.
    assert!(
        p.partners
            .revoke(&p.bob, bob, consent, "not mine")
            .await
            .is_err()
    );
    let revoked = p
        .partners
        .revoke(&p.alice, alice, consent, "stopped using the app")
        .await?;
    assert_eq!(
        (revoked["state"].as_str(), revoked["revokedBy"].as_str()),
        (Some("REVOKED"), Some("CUSTOMER"))
    );
    assert!(
        !p.can(&partner_actor, Action::ReadAccount, p.alice_account)
            .await?
    );
    assert!(p.partners.binding(client, consent).await?.is_none());
    assert_eq!(
        p.partners.partner_status(client, consent).await?.unwrap()["state"],
        "REVOKED"
    );
    let history: Vec<String> = sqlx::query_scalar(
        "SELECT state FROM identity.consent_history WHERE consent_id=$1 ORDER BY recorded_at",
    )
    .bind(consent)
    .fetch_all(&p.db.migrator)
    .await?;
    assert_eq!(history, ["AWAITING_AUTHORIZATION", "ACTIVE", "REVOKED"]);
    let identity_pool = p.db.module(Module::Identity).await?;
    assert!(
        sqlx::query("UPDATE identity.consents SET state='ACTIVE' WHERE id=$1")
            .bind(consent)
            .execute(&identity_pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM identity.consent_history")
            .execute(&p.db.migrator)
            .await
            .is_err()
    );
    // Suspending the client stops every one of its consents at once.
    let second = p
        .partners
        .request_consent(client, Uuid::new_v4(), &request)
        .await?;
    let second = uuid(&second["consentId"]);
    p.partners
        .authorize(&p.alice, alice, second, true, &[p.alice_account])
        .await?;
    let second_actor = p
        .partners
        .binding(client, second)
        .await?
        .unwrap()
        .actor(client);
    assert!(
        p.can(&second_actor, Action::ReadAccount, p.alice_account)
            .await?
    );
    let admin = p.person(&p.admin).await?;
    assert_eq!(
        p.partners
            .decide(&p.admin, admin, partner, "SUSPEND", "security review")
            .await?["state"],
        "SUSPENDED"
    );
    assert!(
        !p.can(&second_actor, Action::ReadAccount, p.alice_account)
            .await?
    );
    assert!(p.partners.active_client(client).await?.is_none());
    p.db.close().await?;
    Ok(())
}

#[tokio::test]
async fn partner_callbacks_are_queued_once_and_versions_report_sunset() -> Result<()> {
    let p = P::new().await?;
    let client = "partner-pay";
    p.active_partner(
        client,
        json!(["accounts:read", "payments:initiate"]),
        Some("http://127.0.0.1:19/hook"),
    )
    .await?;
    let consent = p
        .partners
        .request_consent(
            client,
            Uuid::new_v4(),
            &json!({"scopes": ["payments:initiate"], "durationDays": 1}),
        )
        .await?;
    let consent = uuid(&consent["consentId"]);
    let alice = p.person(&p.alice).await?;
    p.partners
        .authorize(&p.alice, alice, consent, true, &[p.alice_account])
        .await?;
    let binding = p.partners.binding(client, consent).await?.unwrap();
    let actor = binding.actor(client);
    assert!(
        p.can(&actor, Action::CreatePayment, p.alice_account)
            .await?
    );
    assert!(
        p.can(&actor, Action::ReadAccount, p.alice_account).await?,
        "a payment initiation reads its payer account"
    );
    p.partners.revoke(&p.alice, alice, consent, "done").await?;
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM identity.partner_callbacks WHERE consent_id=$1 ORDER BY created_at",
    )
    .bind(consent)
    .fetch_all(&p.db.migrator)
    .await?;
    assert_eq!(events, ["consent.authorized", "consent.revoked"]);
    // Port 19 refuses connections: each attempt is recorded and retried with backoff, never marked delivered.
    let http = reqwest::Client::new();
    assert!(p.partners.deliver_callback(&http).await?);
    let (state, attempts): (String, i32) = sqlx::query_as("SELECT state,attempts FROM identity.partner_callbacks WHERE consent_id=$1 AND event_type='consent.authorized'").bind(consent).fetch_one(&p.db.migrator).await?;
    assert_eq!((state.as_str(), attempts), ("PENDING", 1));
    // API versions: current, deprecated with a sunset date, and sunset.
    assert_eq!(
        p.partners.api_version("2026-11-01").await?.unwrap()["state"],
        "CURRENT"
    );
    assert_eq!(
        p.partners.api_version("2026-05-01").await?.unwrap()["state"],
        "SUNSET"
    );
    assert!(p.partners.api_version("2025-01-01").await?.is_none());
    p.db.close().await?;
    Ok(())
}

#[tokio::test]
async fn assisted_sessions_need_customer_authorization_and_channel_policy() -> Result<()> {
    use bank_controls::assisted::Assisted;
    let p = P::new().await?;
    let assisted = Assisted::new(p.db.module(Module::Controls).await?);
    let (agent, other) = (p.admin.clone(), p.admin2.clone());
    let (agent_person, other_person) = (p.person(&agent).await?, p.person(&other).await?);
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'assisted_service',now()+interval '1 day')")
        .bind(Uuid::new_v4())
        .bind(agent_person.0)
        .execute(&p.db.migrator)
        .await?;
    let alice = p.person(&p.alice).await?;
    let open = |channel: &str, verification: &str| json!({"channel": channel, "customerPerson": alice.0, "verification": verification, "purpose": "Customer called about a lost card"});
    // Only staff with the assisted role, a verification method permitted for the channel, and a real customer.
    assert!(
        assisted
            .open(
                &other,
                other_person,
                Uuid::new_v4(),
                &open("CALL_CENTER", "VERIFIED_CONTACT_CALLBACK")
            )
            .await
            .is_err()
    );
    assert!(
        assisted
            .open(
                &agent,
                agent_person,
                Uuid::new_v4(),
                &open("CALL_CENTER", "ID_DOCUMENT_SIGHTED")
            )
            .await
            .is_err()
    );
    let session = assisted
        .open(
            &agent,
            agent_person,
            Uuid::new_v4(),
            &open("CALL_CENTER", "VERIFIED_CONTACT_CALLBACK"),
        )
        .await?;
    assert_eq!(session["state"], "AWAITING_CUSTOMER");
    let id = uuid(&session["id"]);
    let actor = Assisted::actor(&agent, id, "CALL_CENTER", alice);
    assert_eq!(actor.client_id, "assisted-call-center");
    // Nothing is possible before the customer authorizes in a bank channel.
    assert!(!p.can(&actor, Action::ReadAccount, p.alice_account).await?);
    let bob = p.person(&p.bob).await?;
    assert!(
        assisted.authorize(&p.bob, bob, id, true).await.is_err(),
        "another customer cannot authorize"
    );
    assert_eq!(
        assisted.authorize(&p.alice, alice, id, true).await?["state"],
        "AUTHORIZED"
    );
    // Confirmed operations only, on the represented customer's own accounts.
    assert!(p.can(&actor, Action::ReadAccount, p.alice_account).await?);
    assert!(
        p.can(&actor, Action::ServiceAccount, p.alice_account)
            .await?
    );
    assert!(
        !p.can(&actor, Action::CreatePayment, p.alice_account)
            .await?,
        "payments are not confirmed for any channel"
    );
    assert!(!p.can(&actor, Action::ReadAccount, p.bob_account).await?);
    // Another staff member cannot reuse the session, and a forged represented party fails.
    let stolen = Assisted::actor(&other, id, "CALL_CENTER", alice);
    assert!(!p.can(&stolen, Action::ReadAccount, p.alice_account).await?);
    let forged = Assisted::actor(&agent, id, "CALL_CENTER", bob);
    assert!(!p.can(&forged, Action::ReadAccount, p.bob_account).await?);
    // The customer ends the session; access stops at once and every decision is on record.
    assert_eq!(
        assisted.close(&p.alice, alice, id).await?["state"],
        "CLOSED"
    );
    assert!(!p.can(&actor, Action::ReadAccount, p.alice_account).await?);
    let reasons: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM controls.assisted_actions WHERE session_id=$1 ORDER BY recorded_at",
    )
    .bind(id)
    .fetch_all(&p.db.migrator)
    .await?;
    assert!(
        reasons.contains(&"SESSION_NOT_ACTIVE".to_owned())
            && reasons.contains(&"OPERATION_NOT_ENABLED_FOR_CHANNEL".to_owned())
            && reasons.contains(&"SESSION_PARTY_MISMATCH".to_owned())
            && reasons.contains(&"CHANNEL_POLICY".to_owned()),
        "{reasons:?}"
    );
    let view = assisted.staff_view(&agent, agent_person).await?;
    assert_eq!(view["policies"].as_array().unwrap().len(), 3);
    assert!(
        sqlx::query("DELETE FROM controls.assisted_actions")
            .execute(&p.db.migrator)
            .await
            .is_err()
    );
    p.db.close().await?;
    Ok(())
}
