use anyhow::Result;
use bank_controls::entitlements::Entitlements;
use bank_core::Core;
use bank_platform::{
    database::Module,
    domain::{AccountId, ActorContext, Currency, Money, TrustContext},
    testing::TestDatabase,
};
use serde_json::{Value, json};
use uuid::Uuid;
struct F {
    db: TestDatabase,
    core: Core,
    c: Entitlements,
    a: ActorContext,
    b: ActorContext,
    account: Uuid,
}
impl F {
    async fn new() -> Result<Self> {
        Self::from_db(TestDatabase::new().await?).await
    }
    async fn from_db(db: TestDatabase) -> Result<Self> {
        let core = Core::new(db.module(Module::Core).await?);
        let c = Entitlements::new(db.module(Module::Controls).await?);
        let actor = |s: &str| ActorContext {
            issuer: "urn:consumer-fixture".into(),
            subject: s.into(),
            client_id: "customer-web".into(),
            trust: TrustContext::Customer,
            person_id: None,
            represented_party: None,
            roles: vec![],
        };
        let a = actor("alice");
        let b = actor("bob");
        for who in [&a, &b] {
            let p = Uuid::new_v4();
            sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
                .bind(&who.issuer)
                .bind(&who.subject)
                .bind(p)
                .execute(&db.migrator)
                .await?;
            sqlx::query(
                "INSERT INTO identity.subjects(issuer,subject,trust) VALUES($1,$2,'customer')",
            )
            .bind(&who.issuer)
            .bind(&who.subject)
            .execute(&db.migrator)
            .await?;
        }
        let person = c.person(&a).await?.0;
        let product = Uuid::new_v4();
        let party = Uuid::new_v4();
        let account = Uuid::new_v4();
        sqlx::query("INSERT INTO core.product_versions VALUES($1,'UI_FIXTURE',1,'2026-01-01','2030-01-01','{\"label\":\"SYNTHETIC\",\"chartVersion\":\"synthetic-chart-v1\"}',$2,$3,$4,now())").bind(product).bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(Uuid::new_v4()).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.parties(id,issuer,subject,source_application,identity_digest) VALUES($1,$2,$3,$4,'fixture')").bind(party).bind(&a.issuer).bind(&a.subject).bind(Uuid::new_v4()).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'CUSTOMER','ACTIVE',false),('4f1a780b-3e31-48ef-a294-000000000003','SETTLEMENT','ACTIVE',false) ON CONFLICT DO NOTHING").bind(account).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.account_registry(id,application_id,party_id,product_version_id,acceptance_id,opening_digest) VALUES($1,$2,$3,$4,$5,'fixture')").bind(account).bind(Uuid::new_v4()).bind(party).bind(product).bind(Uuid::new_v4()).execute(&db.migrator).await?;
        sqlx::query("INSERT INTO core.account_relationships VALUES($1,$2,'OWNER',NULL)")
            .bind(account)
            .bind(person)
            .execute(&db.migrator)
            .await?;
        core.fund_synthetic(
            Uuid::new_v4(),
            AccountId(account),
            AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?),
            Money::new(100000, Currency::synthetic()),
        )
        .await?;
        Ok(Self {
            db,
            core,
            c,
            a,
            b,
            account,
        })
    }
    async fn cmd(&self, action: &str, input: Value) -> Result<Value> {
        self.core
            .consumer_command(&self.c, &self.a, Uuid::new_v4(), action, &input)
            .await
    }
    async fn approve(&self, r: &Value) -> Result<()> {
        let id: Uuid = serde_json::from_value(r["id"].clone())?;
        sqlx::query("INSERT INTO identity.step_up_proofs(id,challenge_id,issuer,subject,intent_digest,authenticated_at,expires_at) SELECT $1,challenge_id,$2,$3,digest,clock_timestamp(),clock_timestamp()+interval '3 minutes' FROM core.consumer_authorizations WHERE id=$4").bind(Uuid::new_v4()).bind(&self.a.issuer).bind(&self.a.subject).bind(id).execute(&self.db.migrator).await?;
        Ok(())
    }
    async fn close(self) -> Result<()> {
        self.core.pool.close().await;
        self.c.pool.close().await;
        self.db.close().await
    }
}
#[tokio::test]
async fn vault_money_is_native_scoped_locked_and_replay_safe() -> Result<()> {
    let f = F::new().await?;
    let v = f
        .cmd(
            "VAULT_CREATE",
            json!({"name":"Emergency","targetMinor":"50000"}),
        )
        .await?;
    let id = v["id"].as_str().unwrap();
    let key = Uuid::new_v4();
    let input = json!({"id":id,"accountId":f.account,"amountMinor":"25000"});
    let one = f
        .core
        .consumer_command(&f.c, &f.a, key, "VAULT_DEPOSIT", &input)
        .await?;
    let two = f
        .core
        .consumer_command(&f.c, &f.a, key, "VAULT_DEPOSIT", &input)
        .await?;
    assert_eq!(one, two);
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        75000
    );
    assert!(
        f.core
            .consumer_command(&f.c, &f.b, Uuid::new_v4(), "VAULT_WITHDRAW", &input)
            .await
            .is_err()
    );
    f.cmd("VAULT_UPDATE", json!({"id":id,"locked":true}))
        .await?;
    assert!(f.cmd("VAULT_WITHDRAW", input.clone()).await.is_err());
    f.cmd("VAULT_UPDATE", json!({"id":id,"locked":false}))
        .await?;
    f.cmd("VAULT_WITHDRAW", input).await?;
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        100000
    );
    f.close().await
}
#[tokio::test]
async fn brokerage_requires_exact_step_up_and_native_fill_replays_once() -> Result<()> {
    let f = F::new().await?;
    f.cmd("BROKER_OPEN", json!({"accepted":true})).await?;
    f.cmd(
        "BROKER_DEPOSIT",
        json!({"accountId":f.account,"amountMinor":"50000"}),
    )
    .await?;
    let q=f.core.wealth_quote(&f.c,&f.a,&json!({"symbol":"AAPL","side":"BUY","orderType":"MARKET","mode":"AMOUNT","amountMinor":"10000","timeInForce":"DAY"})).await?;
    assert!(
        f.cmd("TRADE_EXECUTE", json!({"authorizationId":q["id"]}))
            .await
            .is_err()
    );
    f.approve(&q).await?;
    let key = Uuid::new_v4();
    let input = json!({"authorizationId":q["id"]});
    let (a, b) = tokio::join!(
        f.core
            .consumer_command(&f.c, &f.a, key, "TRADE_EXECUTE", &input),
        f.core
            .consumer_command(&f.c, &f.a, key, "TRADE_EXECUTE", &input)
    );
    let a = a?;
    assert_eq!(a, b?);
    assert_eq!(a["state"], "FILLED");
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    assert_eq!(w["orders"].as_array().unwrap().len(), 1);
    assert_eq!(w["positions"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.core.wealth_snapshot(&f.c, &f.b).await?["positions"],
        json!([])
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.wealth_fills")
            .fetch_one(&f.core.pool)
            .await?,
        1
    );
    assert!(
        sqlx::query("UPDATE core.wealth_positions SET units=999")
            .execute(&f.core.pool)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn open_limit_reservations_and_cancel_restore_only_original_available_cash() -> Result<()> {
    let f = F::new().await?;
    f.cmd("BROKER_OPEN", json!({"accepted":true})).await?;
    f.cmd(
        "BROKER_DEPOSIT",
        json!({"accountId":f.account,"amountMinor":"50000"}),
    )
    .await?;
    let q=f.core.wealth_quote(&f.c,&f.a,&json!({"symbol":"AAPL","side":"BUY","orderType":"LIMIT","mode":"UNITS","quantityUnits":"1000000","limitMinor":"10000","timeInForce":"GTC"})).await?;
    f.approve(&q).await?;
    let r = f
        .cmd("TRADE_EXECUTE", json!({"authorizationId":q["id"]}))
        .await?;
    assert_eq!(r["state"], "OPEN");
    assert_eq!(
        f.core.wealth_snapshot(&f.c, &f.a).await?["account"]["heldMinor"],
        "10000"
    );
    f.cmd("ORDER_CANCEL", json!({"id":r["id"]})).await?;
    assert_eq!(
        f.core.wealth_snapshot(&f.c, &f.a).await?["account"]["heldMinor"],
        "0"
    );
    f.close().await
}

#[tokio::test]
async fn card_roundups_are_capped_and_approved_purchases_cannot_execute_twice() -> Result<()> {
    let f = F::new().await?;
    let vault = f
        .cmd(
            "VAULT_CREATE",
            json!({"name":"Spare change","targetMinor":"10000"}),
        )
        .await?;
    let card = f
        .cmd(
            "CARD_CREATE",
            json!({"accountId":f.account,"label":"Everyday"}),
        )
        .await?;
    let r=f.core.consumer_review(&f.c,&f.a,"ROUNDUP_SET",&json!({"cardId":card["id"],"vaultId":vault["id"],"multiplier":3,"weeklyCapMinor":"100","enabled":true})).await?;
    f.approve(&r).await?;
    f.cmd("ROUNDUP_SET", json!({"authorizationId":r["id"]}))
        .await?;
    let purchase = json!({"id":card["id"],"merchant":"Synthetic Coffee","category":"Dining","amountMinor":"750"});
    let q = f
        .core
        .consumer_review(&f.c, &f.a, "CARD_PURCHASE", &purchase)
        .await?;
    f.approve(&q).await?;
    let key = Uuid::new_v4();
    let data = json!({"authorizationId":q["id"]});
    let one = f
        .core
        .consumer_command(&f.c, &f.a, key, "CARD_PURCHASE", &data)
        .await?;
    assert_eq!(
        one,
        f.core
            .consumer_command(&f.c, &f.a, key, "CARD_PURCHASE", &data)
            .await?
    );
    assert!(f.cmd("CARD_PURCHASE", data).await.is_err());
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        99150
    );
    let v: Uuid = serde_json::from_value(vault["id"].clone())?;
    assert_eq!(
        f.core.balance(AccountId(v)).await?.booked.minor_units(),
        100
    );
    let q = f
        .core
        .consumer_review(&f.c, &f.a, "CARD_PURCHASE", &purchase)
        .await?;
    f.approve(&q).await?;
    f.cmd("CARD_UPDATE", json!({"id":card["id"],"frozen":true}))
        .await?;
    assert!(
        f.cmd("CARD_PURCHASE", json!({"authorizationId":q["id"]}))
            .await
            .is_err()
    );
    f.cmd("CARD_UPDATE", json!({"id":card["id"],"frozen":false}))
        .await?;
    f.cmd("CARD_PURCHASE", json!({"authorizationId":q["id"]}))
        .await?;
    assert_eq!(
        f.core.balance(AccountId(v)).await?.booked.minor_units(),
        100
    );
    let d = f.core.consumer_snapshot(&f.c, &f.a).await?;
    assert_eq!(d["analytics"]["spentMinor"], "1500");
    assert_eq!(d["roundups"]["savedMinor"], "100");
    assert_eq!(d["roundups"]["saved30DaysMinor"], "100");
    assert_eq!(d["roundups"]["roundupCount"], 1);
    let notifications = d["notifications"].as_array().unwrap().len();
    f.cmd("PREFERENCES", json!({"notifications":false})).await?;
    f.cmd(
        "BUDGET",
        json!({"category":"Dining","limitMinor":"1000","threshold":80}),
    )
    .await?;
    f.core.consumer_tick().await?;
    assert_eq!(
        f.core.consumer_snapshot(&f.c, &f.a).await?["notifications"]
            .as_array()
            .unwrap()
            .len(),
        notifications
    );

    assert!(
        f.cmd("PREFERENCES", json!({"goals":"invalid"}))
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn shared_withdrawal_requires_unanimity_and_cannot_disable_consensus() -> Result<()> {
    let f = F::new().await?;
    let v = f
        .cmd(
            "VAULT_CREATE",
            json!({"name":"Together","targetMinor":"50000","mutual":true}),
        )
        .await?;
    let id: Uuid = serde_json::from_value(v["id"].clone())?;
    let bob = f.c.person(&f.b).await?.0;
    sqlx::query("INSERT INTO core.consumer_members VALUES($1,$2,'INVITED')")
        .bind(id)
        .bind(bob)
        .execute(&f.db.migrator)
        .await?;
    f.core
        .consumer_command(&f.c, &f.b, Uuid::new_v4(), "VAULT_JOIN", &json!({"id":id}))
        .await?;
    f.cmd(
        "VAULT_DEPOSIT",
        json!({"id":id,"accountId":f.account,"amountMinor":"20000"}),
    )
    .await?;
    assert!(
        f.cmd(
            "VAULT_WITHDRAW",
            json!({"id":id,"accountId":f.account,"amountMinor":"1000"})
        )
        .await
        .is_err()
    );
    assert!(
        f.cmd("VAULT_UPDATE", json!({"id":id,"mutual":false}))
            .await
            .is_err()
    );
    let r = f
        .cmd(
            "GROUP_WITHDRAW",
            json!({"id":id,"accountId":f.account,"amountMinor":"5000"}),
        )
        .await?;
    assert_eq!(
        f.core.balance(AccountId(id)).await?.held.minor_units(),
        5000
    );
    let p = f
        .core
        .consumer_command(
            &f.c,
            &f.b,
            Uuid::new_v4(),
            "GROUP_APPROVE",
            &json!({"id":r["id"]}),
        )
        .await?;
    assert_eq!(p["state"], "POSTED");
    let snapshot = f.core.consumer_snapshot(&f.c, &f.a).await?;
    let contributors = snapshot["vaultContributors"].as_array().unwrap();
    assert_eq!(
        contributors.iter().find(|c| c["isYou"] == true).unwrap()["contributedMinor"],
        "20000"
    );
    assert_eq!(
        contributors.iter().find(|c| c["isYou"] == false).unwrap()["contributedMinor"],
        "0"
    );
    assert!(contributors.iter().all(|c| c.get("personId").is_none()));

    assert_eq!(
        f.core.balance(AccountId(id)).await?.booked.minor_units(),
        15000
    );
    assert_eq!(f.core.balance(AccountId(id)).await?.held.minor_units(), 0);
    f.core
        .consumer_command(
            &f.c,
            &f.b,
            Uuid::new_v4(),
            "GROUP_APPROVE",
            &json!({"id":r["id"]}),
        )
        .await?;
    assert_eq!(
        f.core.balance(AccountId(id)).await?.booked.minor_units(),
        15000
    );
    f.close().await
}
#[tokio::test]
async fn schedules_use_retained_consent_and_concurrent_ticks_post_one_due_run() -> Result<()> {
    let f = F::new().await?;
    let v = f
        .cmd(
            "VAULT_CREATE",
            json!({"name":"Habit","targetMinor":"50000"}),
        )
        .await?;
    let day: String =
        sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'UTC')::date::text")
            .fetch_one(&f.db.migrator)
            .await?;
    let q=f.core.consumer_review(&f.c,&f.a,"SCHEDULE_CREATE",&json!({"kind":"SAVE","accountId":f.account,"vaultId":v["id"],"amountMinor":"500","frequency":"DAILY","nextRun":day})).await?;
    f.approve(&q).await?;
    let s = f
        .cmd("SCHEDULE_CREATE", json!({"authorizationId":q["id"]}))
        .await?;
    assert!(
        f.cmd("SCHEDULE_CREATE", json!({"authorizationId":q["id"]}))
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(f.core.consumer_tick(), f.core.consumer_tick());
    a?;
    b?;
    f.core.consumer_tick().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core.consumer_schedule_runs")
        .fetch_one(&f.db.migrator)
        .await?;
    assert_eq!(count, 1);
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        99500
    );
    f.cmd("SCHEDULE_UPDATE", json!({"id":s["id"],"state":"PAUSED"}))
        .await?;
    f.cmd("SCHEDULE_UPDATE", json!({"id":s["id"],"state":"CANCELLED"}))
        .await?;
    assert!(
        f.cmd("SCHEDULE_UPDATE", json!({"id":s["id"],"state":"ACTIVE"}))
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn developer_tokens_are_scoped_hashed_revocable_and_statements_are_owner_only() -> Result<()>
{
    let f = F::new().await?;
    let q = f
        .core
        .consumer_review(
            &f.c,
            &f.a,
            "TOKEN_CREATE",
            &json!({"label":"Read accounts","scope":"accounts:read"}),
        )
        .await?;
    f.approve(&q).await?;
    let key = Uuid::new_v4();
    let input = json!({"authorizationId":q["id"]});
    let r = f
        .core
        .consumer_command(&f.c, &f.a, key, "TOKEN_CREATE", &input)
        .await?;
    let secret = r["token"].as_str().unwrap();
    assert_eq!(
        f.core.consumer_token_read(secret, "accounts:read").await?["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.core
            .consumer_token_read(secret, "transactions:read")
            .await
            .is_err()
    );
    let replay = f
        .core
        .consumer_command(&f.c, &f.a, key, "TOKEN_CREATE", &input)
        .await?;
    assert!(replay.get("token").is_none());
    let retained: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM core.consumer_commands WHERE result::text LIKE '%'||$1||'%')",
    )
    .bind(secret)
    .fetch_one(&f.db.migrator)
    .await?;
    assert!(!retained);
    f.cmd("TOKEN_REVOKE", json!({"id":r["id"]})).await?;
    assert!(
        f.core
            .consumer_token_read(secret, "accounts:read")
            .await
            .is_err()
    );
    let s = f
        .cmd(
            "STATEMENT",
            json!({"accountId":f.account,"from":"2026-01-01","to":"2027-01-01"}),
        )
        .await?;
    let id: Uuid = serde_json::from_value(s["id"].clone())?;
    let d = f.core.consumer_statement(&f.c, &f.a, id).await?;
    assert_eq!(d["items"].as_array().unwrap().len(), 1);
    assert_eq!(d["items"][0]["deltaMinor"], "100000");
    assert!(f.core.consumer_statement(&f.c, &f.b, id).await.is_err());
    f.close().await
}
#[tokio::test]
async fn dividends_follow_record_time_positions_and_reinvest_only_once() -> Result<()> {
    let f = F::new().await?;
    f.cmd("BROKER_OPEN", json!({"accepted":true})).await?;
    f.cmd(
        "BROKER_DEPOSIT",
        json!({"accountId":f.account,"amountMinor":"60000"}),
    )
    .await?;
    let q=f.core.wealth_quote(&f.c,&f.a,&json!({"symbol":"VOO","side":"BUY","orderType":"MARKET","mode":"UNITS","quantityUnits":"1000000","timeInForce":"DAY"})).await?;
    f.approve(&q).await?;
    f.cmd("TRADE_EXECUTE", json!({"authorizationId":q["id"]}))
        .await?;
    let q = f
        .core
        .consumer_review(
            &f.c,
            &f.a,
            "DRIP_SET",
            &json!({"symbol":"VOO","enabled":true}),
        )
        .await?;
    f.approve(&q).await?;
    f.cmd("DRIP_SET", json!({"authorizationId":q["id"]}))
        .await?;
    let action = Uuid::new_v4();
    sqlx::query("INSERT INTO core.wealth_corporate_actions VALUES($1,'VOO',clock_timestamp(),clock_timestamp(),100,'SYNTHETIC_CORPORATE_ACTION')").bind(action).execute(&f.db.migrator).await?;
    f.core.consumer_tick().await?;
    f.core.consumer_tick().await?;
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    assert_eq!(w["dividends"].as_array().unwrap().len(), 1);
    assert_eq!(w["dividends"][0]["amountMinor"], "100");
    assert!(
        w["positions"][0]["units"]
            .as_str()
            .unwrap()
            .parse::<i64>()?
            > 1000000
    );
    let batches: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core.journal_batches WHERE posting_kind='DIVIDEND_REINVESTMENT'",
    )
    .fetch_one(&f.db.migrator)
    .await?;
    assert_eq!(batches, 1);
    f.close().await
}

#[tokio::test]
async fn limit_fill_and_cancel_race_resolve_one_reservation_and_one_effect() -> Result<()> {
    let f = F::new().await?;
    f.cmd("BROKER_OPEN", json!({"accepted":true})).await?;
    f.cmd(
        "BROKER_DEPOSIT",
        json!({"accountId":f.account,"amountMinor":"50000"}),
    )
    .await?;
    let q=f.core.wealth_quote(&f.c,&f.a,&json!({"symbol":"AAPL","side":"BUY","orderType":"LIMIT","mode":"UNITS","quantityUnits":"1000000","limitMinor":"10000","timeInForce":"GTC"})).await?;
    f.approve(&q).await?;
    let order = f
        .cmd("TRADE_EXECUTE", json!({"authorizationId":q["id"]}))
        .await?;
    sqlx::query("UPDATE core.wealth_assets SET price_minor=9000 WHERE symbol='AAPL'")
        .execute(&f.db.migrator)
        .await?;
    let (tick, _cancel) = tokio::join!(
        f.core.consumer_tick(),
        f.cmd("ORDER_CANCEL", json!({"id":order["id"]}))
    );
    tick?;
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    assert_eq!(w["account"]["heldMinor"], "0");
    match w["orders"][0]["state"].as_str().unwrap() {
        "FILLED" => {
            assert_eq!(w["positions"][0]["units"], "1000000");
            assert_eq!(w["account"]["cashMinor"], "41000");
        }
        "CANCELLED" => {
            assert_eq!(w["positions"], json!([]));
            assert_eq!(w["account"]["cashMinor"], "50000");
        }
        state => panic!("Unexpected terminal state {state}"),
    }
    f.close().await
}
#[tokio::test]
async fn unlinked_customer_can_read_empty_onboarding_but_cannot_create_financial_state()
-> Result<()> {
    let f = F::new().await?;
    let mut a = f.a.clone();
    a.subject = "not-yet-onboarded".into();
    let d = f.core.consumer_snapshot(&f.c, &a).await?;
    assert_eq!(d["needsOnboarding"], true);
    assert_eq!(d["accounts"], json!([]));
    assert_eq!(
        f.core.wealth_snapshot(&f.c, &a).await?["account"],
        Value::Null
    );
    assert!(
        f.core
            .consumer_command(
                &f.c,
                &a,
                Uuid::new_v4(),
                "VAULT_CREATE",
                &json!({"name":"Denied","targetMinor":"100"})
            )
            .await
            .is_err()
    );
    f.close().await
}

#[tokio::test]
async fn fresh_approval_is_rechecked_after_waiting_for_native_cash_lock() -> Result<()> {
    let f = F::new().await?;
    f.cmd("BROKER_OPEN", json!({"accepted":true})).await?;
    f.cmd(
        "BROKER_DEPOSIT",
        json!({"accountId":f.account,"amountMinor":"50000"}),
    )
    .await?;
    let q=f.core.wealth_quote(&f.c,&f.a,&json!({"symbol":"AAPL","side":"BUY","orderType":"MARKET","mode":"AMOUNT","amountMinor":"2500","timeInForce":"DAY"})).await?;
    f.approve(&q).await?;
    let cash: Uuid =
        serde_json::from_value(f.core.wealth_snapshot(&f.c, &f.a).await?["account"]["id"].clone())?;
    sqlx::query("UPDATE identity.step_up_proofs SET expires_at=clock_timestamp()+interval '2 seconds' WHERE intent_digest=$1").bind(q["digest"].as_str().unwrap()).execute(&f.db.migrator).await?;
    let mut tx = f.db.migrator.begin().await?;
    sqlx::query("SELECT 1 FROM core.ledger_accounts WHERE id=$1 FOR UPDATE")
        .bind(cash)
        .execute(&mut *tx)
        .await?;
    {
        let execute = f.cmd("TRADE_EXECUTE", json!({"authorizationId":q["id"]}));
        tokio::pin!(execute);
        tokio::select! { result=&mut execute=>panic!("Execution did not wait for native cash lock: {result:?}"),_=tokio::time::sleep(std::time::Duration::from_millis(2300))=>{} }
        tx.commit().await?;
        assert!(execute.await.is_err());
    }
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    assert_eq!(w["orders"], json!([]));
    assert_eq!(w["positions"], json!([]));
    assert_eq!(w["account"]["heldMinor"], "0");
    assert_eq!(w["account"]["cashMinor"], "50000");
    f.close().await
}

async fn recurring(f: &F, day: &str, amount: &str) -> Result<Value> {
    let v = f
        .cmd(
            "VAULT_CREATE",
            json!({"name":"Recurring goal","targetMinor":"50000"}),
        )
        .await?;
    let q=f.core.consumer_review(&f.c,&f.a,"SCHEDULE_CREATE",&json!({"kind":"SAVE","accountId":f.account,"vaultId":v["id"],"amountMinor":amount,"frequency":"DAILY","nextRun":day})).await?;
    f.approve(&q).await?;
    f.cmd("SCHEDULE_CREATE", json!({"authorizationId":q["id"]}))
        .await
}
async fn utc_day(f: &F, plus: i32) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT ((clock_timestamp() AT TIME ZONE 'UTC')::date+$1)::text")
            .bind(plus)
            .fetch_one(&f.db.migrator)
            .await?,
    )
}
async fn revision(f: &F, id: &Value, day: &str, amount: &str) -> Result<Value> {
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    let s = w["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == *id)
        .unwrap();
    f.core.consumer_review(&f.c,&f.a,"SCHEDULE_REPLACE",&json!({"id":id,"kind":"SAVE","accountId":f.account,"vaultId":s["vaultId"],"amountMinor":amount,"frequency":"DAILY","nextRun":day})).await
}
#[tokio::test]
async fn recurring_replacement_requires_new_consent_preserves_pause_and_replays_one_version()
-> Result<()> {
    let f = F::new().await?;
    let day = utc_day(&f, 1).await?;
    let original = recurring(&f, &day, "500").await?;
    f.cmd(
        "SCHEDULE_UPDATE",
        json!({"id":original["id"],"state":"PAUSED"}),
    )
    .await?;
    let q = revision(&f, &original["id"], &day, "1000").await?;
    assert_eq!(q["details"]["replaces"]["amountMinor"], "500");
    assert_eq!(q["details"]["replaces"]["state"], "PAUSED");
    assert!(
        f.cmd("SCHEDULE_REPLACE", json!({"authorizationId":q["id"]}))
            .await
            .is_err()
    );
    f.approve(&q).await?;
    let key = Uuid::new_v4();
    let input = json!({"authorizationId":q["id"]});
    let (a, b) = tokio::join!(
        f.core
            .consumer_command(&f.c, &f.a, key, "SCHEDULE_REPLACE", &input),
        f.core
            .consumer_command(&f.c, &f.a, key, "SCHEDULE_REPLACE", &input)
    );
    let r = a?;
    assert_eq!(r, b?);
    assert_eq!(r["state"], "PAUSED");
    assert!(f.cmd("SCHEDULE_REPLACE", input.clone()).await.is_err());
    assert!(
        f.core
            .consumer_command(&f.c, &f.b, Uuid::new_v4(), "SCHEDULE_REPLACE", &input)
            .await
            .is_err()
    );
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    let rows = w["schedules"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let old = rows.iter().find(|x| x["id"] == original["id"]).unwrap();
    let new = rows.iter().find(|x| x["id"] == r["id"]).unwrap();
    assert_eq!(old["amountMinor"], "500");
    assert_eq!(old["state"], "CANCELLED");
    assert_eq!(new["familyId"], original["id"]);
    assert_eq!(new["amountMinor"], "1000");
    assert_eq!(w["automationSummary"]["activeCount"], 0);
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        100000
    );
    assert!(
        sqlx::query("UPDATE core.consumer_schedules SET amount_minor=1001 WHERE id=$1")
            .bind(serde_json::from_value::<Uuid>(r["id"].clone())?)
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn replacement_before_due_run_posts_new_terms_once_and_retains_native_family_totals()
-> Result<()> {
    let f = F::new().await?;
    let day = utc_day(&f, 0).await?;
    let original = recurring(&f, &day, "500").await?;
    let q = revision(&f, &original["id"], &day, "1000").await?;
    f.approve(&q).await?;
    let r = f
        .cmd("SCHEDULE_REPLACE", json!({"authorizationId":q["id"]}))
        .await?;
    let (a, b) = tokio::join!(f.core.consumer_tick(), f.core.consumer_tick());
    a?;
    b?;
    f.core.consumer_tick().await?;
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    assert_eq!(w["automationSummary"]["totalSavedMinor"], "1000");
    assert_eq!(w["automationSummary"]["saveMonthlyMinor"], "30417");
    let row = w["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == r["id"])
        .unwrap();
    assert_eq!(row["completedRuns"], 1);
    assert_eq!(row["familyCompletedRuns"], 1);
    assert_eq!(row["recentRuns"][0]["executedMinor"], "1000");
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        99000
    );
    let id: Uuid = serde_json::from_value(r["id"].clone())?;
    sqlx::query("UPDATE core.consumer_schedules SET next_run=$2::text::date WHERE id=$1")
        .bind(id)
        .bind(&day)
        .execute(&f.db.migrator)
        .await?;
    f.core.consumer_tick().await?;
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        99000
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.consumer_schedule_family_runs")
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    f.close().await
}
#[tokio::test]
async fn due_run_after_review_invalidates_replacement_and_completed_dates_cannot_be_recharged()
-> Result<()> {
    let f = F::new().await?;
    let day = utc_day(&f, 0).await?;
    let original = recurring(&f, &day, "500").await?;
    let q = revision(&f, &original["id"], &day, "1000").await?;
    f.approve(&q).await?;
    f.core.consumer_tick().await?;
    assert!(
        f.cmd("SCHEDULE_REPLACE", json!({"authorizationId":q["id"]}))
            .await
            .is_err()
    );
    assert!(revision(&f, &original["id"], &day, "1000").await.is_err());
    let next = utc_day(&f, 1).await?;
    let q = revision(&f, &original["id"], &next, "1000").await?;
    f.approve(&q).await?;
    let r = f
        .cmd("SCHEDULE_REPLACE", json!({"authorizationId":q["id"]}))
        .await?;
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    let row = w["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == r["id"])
        .unwrap();
    assert_eq!(row["familyExecutedMinor"], "500");
    assert_eq!(row["completedRuns"], 0);
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        99500
    );
    f.close().await
}
#[tokio::test]
async fn forward_upgrade_keeps_original_runs_and_monthly_anchors_survive_short_and_leap_months()
-> Result<()> {
    let f = F::from_db(TestDatabase::at_version(59).await?).await?;
    let day = utc_day(&f, 0).await?;
    recurring(&f, &day, "500").await?;
    f.core.consumer_tick().await?;
    let before: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(r)) FROM core.consumer_schedule_runs r")
            .fetch_one(&f.db.migrator)
            .await?;
    bank_platform::database::MIGRATOR
        .run(&f.db.migrator)
        .await?;
    let after: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(r)) FROM core.consumer_schedule_runs r")
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(before, after);
    let dates:Vec<String>=sqlx::query_scalar("SELECT core.consumer_next_run(d,'MONTHLY',31)::text FROM unnest(ARRAY['2027-01-31'::date,'2027-02-28','2028-01-31','2028-02-29']) d").fetch_all(&f.db.migrator).await?;
    assert_eq!(
        dates,
        vec!["2027-02-28", "2027-03-31", "2028-02-29", "2028-03-31"]
    );
    assert_eq!(
        f.core.wealth_snapshot(&f.c, &f.a).await?["automationSummary"]["totalSavedMinor"],
        "500"
    );
    f.close().await
}

#[tokio::test]
async fn recurring_investment_revision_funds_and_buys_once_with_native_cost_summary() -> Result<()>
{
    let f = F::new().await?;
    f.cmd("BROKER_OPEN", json!({"accepted":true})).await?;
    let future = utc_day(&f, 1).await?;
    let q=f.core.consumer_review(&f.c,&f.a,"SCHEDULE_CREATE",&json!({"kind":"INVEST","accountId":f.account,"symbol":"AAPL","amountMinor":"500","frequency":"WEEKLY","nextRun":future})).await?;
    f.approve(&q).await?;
    let s = f
        .cmd("SCHEDULE_CREATE", json!({"authorizationId":q["id"]}))
        .await?;
    let q=f.core.consumer_review(&f.c,&f.a,"SCHEDULE_REPLACE",&json!({"id":s["id"],"kind":"INVEST","accountId":f.account,"symbol":"VOO","amountMinor":"1000","frequency":"DAILY","nextRun":utc_day(&f,0).await?})).await?;
    f.approve(&q).await?;
    let r = f
        .cmd("SCHEDULE_REPLACE", json!({"authorizationId":q["id"]}))
        .await?;
    f.core.consumer_tick().await?;
    f.core.consumer_tick().await?;
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    assert_eq!(w["automationSummary"]["totalInvestedMinor"], "1000");
    assert_eq!(w["positions"][0]["symbol"], "VOO");
    assert_eq!(
        f.core
            .balance(AccountId(f.account))
            .await?
            .booked
            .minor_units(),
        99000
    );
    let row = w["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == r["id"])
        .unwrap();
    assert_eq!(row["familyCompletedRuns"], 1);
    assert_eq!(row["recentRuns"][0]["executedMinor"], "1000");
    f.close().await
}

async fn streaks(f: &F, days: &[&str], today: &str) -> Result<Value> {
    Ok(
        sqlx::query_scalar("SELECT core.consumer_streaks($1::text[]::date[],$2::date)")
            .bind(days)
            .bind(today)
            .fetch_one(&f.db.migrator)
            .await?,
    )
}
async fn reached(f: &F, function: &str, days: &[&str], n: i32) -> Result<Option<String>> {
    let sql = match function {
        "day" => "SELECT core.consumer_day_streak_reached($1::text[]::date[],$2)::text",
        _ => "SELECT core.consumer_month_streak_reached($1::text[]::date[],$2)::text",
    };
    Ok(sqlx::query_scalar(sql)
        .bind(days)
        .bind(n)
        .fetch_one(&f.db.migrator)
        .await?)
}
#[tokio::test]
async fn saving_streaks_use_utc_date_islands_across_month_and_year_boundaries() -> Result<()> {
    let f = F::new().await?;
    let days = [
        "2026-01-30",
        "2026-01-31",
        "2026-02-01",
        "2026-02-02",
        "2026-02-03",
        "2026-02-05",
        "2026-02-10",
    ];
    let s = streaks(&f, &days, "2026-02-06").await?;
    assert_eq!(s["currentDays"], 1, "yesterday keeps a streak alive");
    assert_eq!(s["longestDays"], 5);
    assert_eq!(s["currentMonths"], 2);
    assert_eq!(s["longestMonths"], 2);
    assert_eq!(s["savingDays"], 6, "future dates are ignored");
    assert_eq!(s["lastSavingDate"], "2026-02-05");
    assert_eq!(streaks(&f, &days, "2026-02-07").await?["currentDays"], 0);
    assert_eq!(streaks(&f, &days, "2026-02-03").await?["currentDays"], 5);
    assert_eq!(streaks(&f, &days, "2026-04-01").await?["currentMonths"], 0);
    assert_eq!(streaks(&f, &[], "2026-02-06").await?["longestDays"], 0);
    assert_eq!(
        reached(&f, "day", &days, 3).await?.as_deref(),
        Some("2026-02-01")
    );
    assert_eq!(
        reached(&f, "day", &days, 5).await?.as_deref(),
        Some("2026-02-03")
    );
    assert_eq!(reached(&f, "day", &days, 6).await?, None);
    let months = [
        "2025-11-20",
        "2025-12-02",
        "2026-01-15",
        "2026-02-15",
        "2026-03-09",
        "2026-03-15",
        "2026-04-15",
        "2026-06-01",
    ];
    assert_eq!(
        reached(&f, "month", &months, 3).await?.as_deref(),
        Some("2026-01-15"),
        "December to January continues a monthly streak"
    );
    assert_eq!(
        reached(&f, "month", &months, 5).await?.as_deref(),
        Some("2026-03-09"),
        "the first saving day of the completing month"
    );
    assert_eq!(
        reached(&f, "month", &months, 6).await?.as_deref(),
        Some("2026-04-15")
    );
    assert_eq!(
        reached(&f, "month", &months, 7).await?,
        None,
        "May breaks the run"
    );
    let m = streaks(&f, &months, "2026-06-20").await?;
    assert_eq!(m["longestMonths"], 6);
    assert_eq!(m["currentMonths"], 1);
    f.close().await
}
#[tokio::test]
async fn progress_derives_owned_net_worth_badges_and_interest_from_native_facts() -> Result<()> {
    let f = F::new().await?;
    let person = f.c.person(&f.a).await?.0;
    let settlement = Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?;
    let before: String = sqlx::query_scalar("SELECT clock_timestamp()::text")
        .fetch_one(&f.db.migrator)
        .await?;
    let today = utc_day(&f, 0).await?;
    let v = f
        .cmd(
            "VAULT_CREATE",
            json!({"name":"Emergency","targetMinor":"50000"}),
        )
        .await?;
    let deposit = json!({"id":v["id"],"accountId":f.account,"amountMinor":"25000"});
    f.cmd("VAULT_DEPOSIT", deposit.clone()).await?;
    let p = f.core.consumer_snapshot(&f.c, &f.a).await?["progress"].clone();
    assert_eq!(p["savings"]["currentDays"], 1);
    assert_eq!(p["savings"]["contributions"], 1);
    assert_eq!(p["savings"]["lifetimeContributedMinor"], "25000");
    assert_eq!(
        p["savings"]["heatmap"],
        json!([{"date":today,"amountMinor":"25000","count":1}])
    );
    let badge = |p: &Value, code: &str| {
        p["badges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["code"] == code)
            .unwrap()
            .clone()
    };
    assert_eq!(badge(&p, "FIRST_SAVE")["unlocked"], true);
    assert_eq!(badge(&p, "FIRST_SAVE")["unlockedOn"], today);
    assert_eq!(badge(&p, "STREAK_7")["unlocked"], false);
    assert_eq!(badge(&p, "STREAK_7")["progress"], "1");
    assert_eq!(badge(&p, "GOAL_REACHED")["unlocked"], false);
    assert_eq!(badge(&p, "SAVED_1K")["progress"], "25000");
    assert_eq!(p["badges"].as_array().unwrap().len(), 8);
    f.cmd("VAULT_DEPOSIT", deposit.clone()).await?;
    f.cmd(
        "VAULT_WITHDRAW",
        json!({"id":v["id"],"accountId":f.account,"amountMinor":"10000"}),
    )
    .await?;
    let p = f.core.consumer_snapshot(&f.c, &f.a).await?["progress"].clone();
    assert_eq!(
        badge(&p, "GOAL_REACHED")["unlockedOn"],
        today,
        "a reached goal stays earned after a withdrawal"
    );
    assert_eq!(
        p["savings"]["contributions"], 2,
        "withdrawals are not savings"
    );
    let now = &p["netWorth"]["current"];
    assert_eq!(now["bankMinor"], "60000");
    assert_eq!(now["vaultMinor"], "40000");
    assert_eq!(now["totalMinor"], "100000");
    // Every fixture fact was posted during this test, after the UTC month start.
    assert_eq!(p["netWorth"]["monthStart"]["totalMinor"], "0");
    assert_eq!(p["netWorth"]["deltaMinor"], "100000");
    assert_eq!(p["netWorth"]["deltaBasisPoints"], Value::Null);
    assert_eq!(p["netWorth"]["history"].as_array().unwrap().len(), 6);
    // A NULL instant means one second after the current statement time.
    let at = |t: Option<String>| {
        sqlx::query_scalar::<_, Value>("SELECT core.consumer_net_worth_at($1,coalesce($2::timestamptz,clock_timestamp()+interval '1 second'))")
            .bind(person)
            .bind(t)
            .fetch_one(&f.db.migrator)
    };
    let earlier = at(Some(before)).await?;
    assert_eq!(earlier["bankMinor"], "100000");
    assert_eq!(earlier["vaultMinor"], "0");
    f.cmd("BROKER_OPEN", json!({"accepted":true})).await?;
    f.cmd(
        "BROKER_DEPOSIT",
        json!({"accountId":f.account,"amountMinor":"30000"}),
    )
    .await?;
    let q=f.core.wealth_quote(&f.c,&f.a,&json!({"symbol":"AAPL","side":"BUY","orderType":"MARKET","mode":"AMOUNT","amountMinor":"10000","timeInForce":"DAY"})).await?;
    f.approve(&q).await?;
    assert_eq!(
        f.cmd("TRADE_EXECUTE", json!({"authorizationId":q["id"]}))
            .await?["state"],
        "FILLED"
    );
    let d = f.core.consumer_snapshot(&f.c, &f.a).await?;
    let w = f.core.wealth_snapshot(&f.c, &f.a).await?;
    let sum = |items: &Value, field: &str| -> i64 {
        items
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x[field].as_str().unwrap().parse::<i64>().unwrap())
            .sum()
    };
    let dashboard = sum(&d["accounts"], "bookedMinor")
        + 40000
        + w["account"]["cashMinor"].as_str().unwrap().parse::<i64>()?
        + sum(&w["positions"], "valueMinor");
    let current = &d["progress"]["netWorth"]["current"];
    assert_eq!(current["totalMinor"], dashboard.to_string());
    let replayed = at(None).await?;
    assert_eq!(replayed["complete"], true);
    assert_eq!(
        replayed["totalMinor"], current["totalMinor"],
        "journal and position-event replay matches live balances"
    );
    assert_eq!(
        f.core.consumer_snapshot(&f.c, &f.b).await?["progress"]["savings"]["contributions"],
        0,
        "another person never sees these facts"
    );
    let accrual = Uuid::new_v4();
    let batch: Uuid =
        sqlx::query_scalar("SELECT batch_id FROM core.post($1,'PERIODIC_ACCRUAL',$2)")
            .bind(accrual)
            .bind(
                json!([{"accountId":f.account,"delta":123},{"accountId":settlement,"delta":-123}]),
            )
            .fetch_one(&f.db.migrator)
            .await?;
    sqlx::query("SELECT core.post($1,'CORRECTION',$2,NULL,$3)")
        .bind(Uuid::new_v4())
        .bind(json!([{"accountId":f.account,"delta":-23},{"accountId":settlement,"delta":23}]))
        .bind(batch)
        .execute(&f.db.migrator)
        .await?;
    let i = f.core.consumer_snapshot(&f.c, &f.a).await?["progress"]["interest"].clone();
    assert_eq!(
        i["lifetimeMinor"], "100",
        "corrections reduce lifetime interest"
    );
    assert_eq!(i["postings"], 2);
    assert_eq!(i["months"].as_array().unwrap().len(), 1);
    assert_eq!(i["months"][0]["amountMinor"], "100");
    // A held symbol without any retained price or own fill is reported, never guessed.
    sqlx::query("INSERT INTO core.wealth_position_events(operation_id,person_id,symbol,units_delta,cost_delta) VALUES($1,$2,'VOO',1000000,0)")
        .bind(Uuid::new_v4())
        .bind(person)
        .execute(&f.db.migrator)
        .await?;
    let unknown = at(None).await?;
    assert_eq!(unknown["complete"], false);
    assert_eq!(unknown["missingPrices"], json!(["VOO"]));
    f.close().await
}
#[tokio::test]
async fn completed_recurring_save_unlocks_autopilot_badge_once() -> Result<()> {
    let f = F::new().await?;
    let today = utc_day(&f, 0).await?;
    recurring(&f, &today, "500").await?;
    let before = f.core.consumer_snapshot(&f.c, &f.a).await?["progress"]["badges"].clone();
    assert_eq!(before[1]["code"], "AUTO_SAVER");
    assert_eq!(before[1]["unlocked"], false);
    f.core.consumer_tick().await?;
    f.core.consumer_tick().await?;
    let p = f.core.consumer_snapshot(&f.c, &f.a).await?["progress"].clone();
    assert_eq!(p["badges"][1]["unlocked"], true);
    assert_eq!(p["badges"][1]["unlockedOn"], today);
    assert_eq!(p["savings"]["contributions"], 1);
    assert_eq!(p["savings"]["lifetimeContributedMinor"], "500");
    f.close().await
}
/// A customer with an active funded account and a receiving reference (fixture rows only).
async fn customer(f: &F, subject: &str, reference: &str) -> Result<(ActorContext, Uuid)> {
    let mut who = f.a.clone();
    who.subject = subject.into();
    let person = match f.c.person(&who).await {
        Ok(p) => p.0,
        Err(_) => {
            let p = Uuid::new_v4();
            sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
                .bind(&who.issuer)
                .bind(&who.subject)
                .bind(p)
                .execute(&f.db.migrator)
                .await?;
            sqlx::query(
                "INSERT INTO identity.subjects(issuer,subject,trust) VALUES($1,$2,'customer')",
            )
            .bind(&who.issuer)
            .bind(&who.subject)
            .execute(&f.db.migrator)
            .await?;
            p
        }
    };
    let account = Uuid::new_v4();
    let party = Uuid::new_v4();
    let product: Uuid =
        sqlx::query_scalar("SELECT product_version_id FROM core.account_registry WHERE id=$1")
            .bind(f.account)
            .fetch_one(&f.db.migrator)
            .await?;
    sqlx::query("INSERT INTO core.parties(id,issuer,subject,source_application,identity_digest) VALUES($1,$2,$3,$4,'fixture')").bind(party).bind(&who.issuer).bind(&who.subject).bind(Uuid::new_v4()).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'CUSTOMER','ACTIVE',false)").bind(account).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO core.account_registry(id,application_id,party_id,product_version_id,acceptance_id,opening_digest) VALUES($1,$2,$3,$4,$5,'fixture')").bind(account).bind(Uuid::new_v4()).bind(party).bind(product).bind(Uuid::new_v4()).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO core.account_relationships VALUES($1,$2,'OWNER',NULL)")
        .bind(account)
        .bind(person)
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("INSERT INTO core.receiving_references(account_id,reference) VALUES($1,$2)")
        .bind(account)
        .bind(reference)
        .execute(&f.db.migrator)
        .await?;
    f.core
        .fund_synthetic(
            Uuid::new_v4(),
            AccountId(account),
            AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?),
            Money::new(50000, Currency::synthetic()),
        )
        .await?;
    Ok((who, account))
}
async fn as_who(f: &F, who: &ActorContext, action: &str, input: Value) -> Result<Value> {
    f.core
        .consumer_command(&f.c, who, Uuid::new_v4(), action, &input)
        .await
}

#[tokio::test]
async fn splits_paid_requests_favourites_saving_rules_and_safe_to_spend_are_native_and_replay_safe()
-> Result<()> {
    let f = F::new().await?;
    sqlx::query(
        "INSERT INTO core.receiving_references(account_id,reference) VALUES($1,'REF-ALICE')",
    )
    .bind(f.account)
    .execute(&f.db.migrator)
    .await?;
    let (bob, bob_account) = customer(&f, "bob", "REF-BOB").await?;
    let (carol, _) = customer(&f, "carol", "REF-CAROL").await?;
    let money = |n| Money::new(n, Currency::synthetic());

    // UI-20: split an outgoing transaction equally; the first participants absorb the remainder.
    let expense = f
        .core
        .transfer(
            Uuid::new_v4(),
            AccountId(f.account),
            AccountId(bob_account),
            money(1001),
            None,
        )
        .await?;
    let input = json!({"accountId":f.account,"batchId":expense,"references":["REF-BOB","REF-CAROL"],"includeSelf":false,"memo":"Dinner"});
    let key = Uuid::new_v4();
    let split = f
        .core
        .consumer_command(&f.c, &f.a, key, "SPLIT_EXPENSE", &input)
        .await?;
    assert_eq!(
        (
            split["totalMinor"].as_str(),
            split["selfShareMinor"].as_str(),
            split["requests"].as_i64()
        ),
        (Some("1001"), Some("0"), Some(2))
    );
    assert_eq!(
        split,
        f.core
            .consumer_command(&f.c, &f.a, key, "SPLIT_EXPENSE", &input)
            .await?,
        "replay returns the original result"
    );
    assert!(
        f.cmd("SPLIT_EXPENSE", input.clone()).await.is_err(),
        "one split per transaction"
    );
    let shares: Vec<(i64, Uuid)> = sqlx::query_as("SELECT amount_minor,recipient_person FROM core.consumer_requests WHERE split_id=$1 ORDER BY amount_minor DESC")
        .bind(Uuid::parse_str(split["id"].as_str().unwrap())?).fetch_all(&f.db.migrator).await?;
    assert_eq!(
        shares.iter().map(|s| s.0).collect::<Vec<_>>(),
        vec![501, 500]
    );
    // Including the requester: the requester keeps the remainder.
    let second = f
        .core
        .transfer(
            Uuid::new_v4(),
            AccountId(f.account),
            AccountId(bob_account),
            money(1001),
            None,
        )
        .await?;
    let mine = f.cmd("SPLIT_EXPENSE", json!({"accountId":f.account,"batchId":second,"references":["REF-BOB"],"includeSelf":true,"memo":"Taxi"})).await?;
    assert_eq!(mine["selfShareMinor"], "501");
    for bad in [
        json!(["REF-ALICE"]),
        json!(["REF-NOBODY"]),
        json!([]),
        json!(["REF-BOB", "REF-BOB"]),
    ] {
        let third = f
            .core
            .transfer(
                Uuid::new_v4(),
                AccountId(f.account),
                AccountId(bob_account),
                money(300),
                None,
            )
            .await?;
        assert!(f.cmd("SPLIT_EXPENSE", json!({"accountId":f.account,"batchId":third,"references":bad,"includeSelf":false,"memo":""})).await.is_err());
    }
    // An incoming transaction cannot be split by its receiver.
    assert!(as_who(&f, &bob, "SPLIT_EXPENSE", json!({"accountId":bob_account,"batchId":expense,"references":["REF-CAROL"],"includeSelf":false,"memo":""})).await.is_err());

    // Paid-request linking: only a settled payment of the exact share from the payer to the requester.
    let bob_request: Uuid = sqlx::query_scalar(
        "SELECT id FROM core.consumer_requests WHERE split_id=$1 AND amount_minor=501",
    )
    .bind(Uuid::parse_str(split["id"].as_str().unwrap())?)
    .fetch_one(&f.db.migrator)
    .await?;
    let carol_request: Uuid = sqlx::query_scalar(
        "SELECT id FROM core.consumer_requests WHERE split_id=$1 AND amount_minor=500",
    )
    .bind(Uuid::parse_str(split["id"].as_str().unwrap())?)
    .fetch_one(&f.db.migrator)
    .await?;
    let pay = |amount: i64| {
        let f = &f;
        async move {
            let op = Uuid::new_v4();
            let batch = f
                .core
                .transfer(
                    op,
                    AccountId(bob_account),
                    AccountId(f.account),
                    money(amount),
                    None,
                )
                .await?;
            sqlx::query("INSERT INTO core.payment_executions(operation_id,payer_account,payee_account,principal,fee,product_version_id,business_date,request_digest,batch_id) SELECT $1,$2,$3,$4,0,product_version_id,current_date,'fixture',$5 FROM core.account_registry WHERE id=$2")
                .bind(op).bind(bob_account).bind(f.account).bind(amount).bind(batch).execute(&f.db.migrator).await?;
            anyhow::Ok(op)
        }
    };
    let short = pay(500).await?;
    assert!(
        as_who(
            &f,
            &bob,
            "REQUEST_MARK_PAID",
            json!({"id":bob_request,"paymentId":short})
        )
        .await
        .is_err(),
        "wrong amount"
    );
    let exact = pay(501).await?;
    assert!(
        f.cmd(
            "REQUEST_MARK_PAID",
            json!({"id":bob_request,"paymentId":exact})
        )
        .await
        .is_err(),
        "only the payer links"
    );
    let paid = as_who(
        &f,
        &bob,
        "REQUEST_MARK_PAID",
        json!({"id":bob_request,"paymentId":exact}),
    )
    .await?;
    assert_eq!(paid["state"], "PAID");
    assert!(
        as_who(
            &f,
            &carol,
            "REQUEST_MARK_PAID",
            json!({"id":carol_request,"paymentId":short})
        )
        .await
        .is_err(),
        "another customer's payment"
    );
    let snapshot = f.core.consumer_snapshot(&f.c, &f.a).await?;
    let s = snapshot["splits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == split["id"])
        .unwrap();
    assert_eq!(s["paid"], 1);

    // UI-22: favourites are known references only.
    f.cmd("FAVORITE_SET", json!({"reference":"REF-BOB","label":"Bob"}))
        .await?;
    assert!(
        f.cmd(
            "FAVORITE_SET",
            json!({"reference":"REF-NOBODY","label":"Nobody"})
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.core.consumer_snapshot(&f.c, &f.a).await?["favorites"],
        json!([{"reference":"REF-BOB","label":"Bob"}])
    );
    f.cmd("FAVORITE_REMOVE", json!({"reference":"REF-BOB"}))
        .await?;
    assert_eq!(
        f.core.consumer_snapshot(&f.c, &f.a).await?["favorites"],
        json!([])
    );

    // UI-13: an approved merchant rule saves a fixed amount per matching purchase, within its weekly cap, once.
    let vault = f
        .cmd(
            "VAULT_CREATE",
            json!({"name":"Coffee fund","targetMinor":"10000"}),
        )
        .await?;
    let card = f
        .cmd(
            "CARD_CREATE",
            json!({"accountId":f.account,"label":"Everyday"}),
        )
        .await?;
    let rule_input = json!({"cardId":card["id"],"vaultId":vault["id"],"matchKind":"MERCHANT","matchValue":"Synthetic Coffee","amountMinor":"200","weeklyCapMinor":"300"});
    let mut bad = rule_input.clone();
    bad["weeklyCapMinor"] = json!("100");
    assert!(
        f.core
            .consumer_review(&f.c, &f.a, "SAVE_RULE_SET", &bad)
            .await
            .is_err()
    );
    let review = f
        .core
        .consumer_review(&f.c, &f.a, "SAVE_RULE_SET", &rule_input)
        .await?;
    assert!(
        f.cmd("SAVE_RULE_SET", json!({"authorizationId":review["id"]}))
            .await
            .is_err(),
        "issuer approval required"
    );
    f.approve(&review).await?;
    let rule = f
        .cmd("SAVE_RULE_SET", json!({"authorizationId":review["id"]}))
        .await?;
    let purchase = |merchant: &str| json!({"id":card["id"],"merchant":merchant,"category":"Dining","amountMinor":"750"});
    let buy = |merchant: String| {
        let f = &f;
        let p = purchase(&merchant);
        async move {
            let q = f
                .core
                .consumer_review(&f.c, &f.a, "CARD_PURCHASE", &p)
                .await?;
            f.approve(&q).await?;
            let key = Uuid::new_v4();
            let data = json!({"authorizationId":q["id"]});
            let one = f
                .core
                .consumer_command(&f.c, &f.a, key, "CARD_PURCHASE", &data)
                .await?;
            let again = f
                .core
                .consumer_command(&f.c, &f.a, key, "CARD_PURCHASE", &data)
                .await?;
            anyhow::Ok((one, again))
        }
    };
    let saved_before = f.core.consumer_snapshot(&f.c, &f.a).await?["progress"]["savings"].clone();
    let (first, replay) = buy("synthetic coffee".into()).await?;
    assert_eq!(first["ruleSavings"][0]["amountMinor"], "200");
    assert_eq!(
        replay["ruleSavings"],
        json!([]),
        "a replay never saves twice"
    );
    let (capped, _) = buy("Synthetic Coffee".into()).await?;
    assert_eq!(capped["ruleSavings"][0]["amountMinor"], "100");
    let (full, _) = buy("Synthetic Coffee".into()).await?;
    assert_eq!(full["ruleSavings"], json!([]));
    let (other, _) = buy("Synthetic Books".into()).await?;
    assert_eq!(other["ruleSavings"], json!([]));
    let vault_id: Uuid = serde_json::from_value(vault["id"].clone())?;
    assert_eq!(
        f.core
            .balance(AccountId(vault_id))
            .await?
            .booked
            .minor_units(),
        300
    );
    // Schema 78: each posted rule saving (200 + 100) counts as a savings contribution and a saving day.
    let saved_after = f.core.consumer_snapshot(&f.c, &f.a).await?["progress"]["savings"].clone();
    let count = |v: &serde_json::Value| v["contributions"].as_i64().unwrap_or(0);
    let lifetime = |v: &serde_json::Value| {
        v["lifetimeContributedMinor"]
            .as_str()
            .unwrap_or("0")
            .parse::<i64>()
            .unwrap_or(0)
    };
    assert_eq!(count(&saved_after) - count(&saved_before), 2);
    assert_eq!(lifetime(&saved_after) - lifetime(&saved_before), 300);
    assert!(saved_after["currentDays"].as_i64().unwrap_or(0) >= 1);
    let rules = f.core.consumer_snapshot(&f.c, &f.a).await?["saveRules"].clone();
    assert_eq!(
        (
            rules[0]["savedThisWeekMinor"].as_str(),
            rules[0]["savedLifetimeMinor"].as_str()
        ),
        (Some("300"), Some("300"))
    );
    f.cmd("SAVE_RULE_DISABLE", json!({"id":rule["id"]})).await?;
    assert!(
        f.cmd("SAVE_RULE_DISABLE", json!({"id":rule["id"]}))
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE core.consumer_save_rules SET enabled=true")
            .execute(&f.db.migrator)
            .await
            .is_err(),
        "rules cannot be re-enabled in place"
    );

    // UI-17: safe-to-spend subtracts bill reminders due by month end and is labelled as a local rule.
    let before = f.core.consumer_snapshot(&f.c, &f.a).await?["safeToSpend"].clone();
    assert_eq!(before["basis"], "LOCAL_RULE_NOT_ADVICE");
    let through = before["through"].as_str().unwrap().to_owned();
    f.cmd(
        "BILL_CREATE",
        json!({"merchant":"Synthetic Power","amountMinor":"1234","nextDate":through}),
    )
    .await?;
    let after = f.core.consumer_snapshot(&f.c, &f.a).await?["safeToSpend"].clone();
    let n = |v: &Value, k: &str| v[k].as_str().unwrap().parse::<i64>().unwrap();
    assert_eq!(
        n(&after, "billsDueMinor"),
        n(&before, "billsDueMinor") + 1234
    );
    assert_eq!(
        n(&after, "safeToSpendMinor"),
        (n(&after, "availableMinor")
            - n(&after, "automationsDueMinor")
            - n(&after, "billsDueMinor"))
        .max(0)
    );
    f.close().await
}

#[tokio::test]
async fn vault_photo_goals_are_bounded_owner_only_metadata() -> Result<()> {
    let f = F::new().await?;
    let vault = f
        .cmd("VAULT_CREATE", json!({"name":"Trip","targetMinor":"5000"}))
        .await?;
    let id = vault["id"].clone();
    // A 1x1 transparent PNG.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    f.cmd(
        "VAULT_PHOTO",
        json!({"id":id,"photo":{"mime":"image/png","content":png}}),
    )
    .await?;
    let snap = f.core.consumer_snapshot(&f.c, &f.a).await?;
    assert_eq!(
        snap["vaultPhotos"][id.as_str().unwrap()],
        format!("data:image/png;base64,{png}")
    );
    for bad in [
        json!({"mime":"image/gif","content":png}),
        json!({"mime":"image/png","content":"not base64!"}),
        json!({"mime":"image/png","content":"A".repeat(43696)}),
        json!({"mime":"image/png","content":png,"extra":1}),
    ] {
        assert!(
            f.cmd("VAULT_PHOTO", json!({"id":id,"photo":bad}))
                .await
                .is_err()
        );
    }
    // Another customer cannot set or see the owner's photo.
    assert!(
        f.core
            .consumer_command(
                &f.c,
                &f.b,
                Uuid::new_v4(),
                "VAULT_PHOTO",
                &json!({"id":id,"photo":null})
            )
            .await
            .is_err()
    );
    assert_eq!(
        f.core.consumer_snapshot(&f.c, &f.b).await?["vaultPhotos"],
        json!({})
    );
    // Removal; the vault balance and journal are untouched.
    let batches: i64 = sqlx::query_scalar("SELECT count(*) FROM core.journal_batches")
        .fetch_one(&f.db.migrator)
        .await?;
    f.cmd("VAULT_PHOTO", json!({"id":id,"photo":null})).await?;
    assert_eq!(
        f.core.consumer_snapshot(&f.c, &f.a).await?["vaultPhotos"],
        json!({})
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM core.journal_batches")
        .fetch_one(&f.db.migrator)
        .await?;
    assert_eq!(batches, after);
    f.close().await
}
