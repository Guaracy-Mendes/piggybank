#[path = "cases/security.rs"]
mod security;
#[path = "cases/statements.rs"]
mod statements;
use bank_controls::{entitlements::Entitlements, governance::ChangeKind};
use bank_core::{Core, products::ProductVersion};
use bank_documents::{Terms, TermsDraft};
use bank_platform::{
    database::Module,
    domain::{ActorContext, Currency, Money, TrustContext},
    testing::TestDatabase,
};
use chrono::{Duration, Utc};
use sqlx::Executor;
use uuid::Uuid;
#[tokio::test]
async fn exact_terms_and_acceptance_remain_bound_immutable_and_current() -> anyhow::Result<()> {
    let db = TestDatabase::new().await?;
    let core_pool = db.module(Module::Core).await?;
    let control_pool = db.module(Module::Controls).await?;
    let pool = db.module(Module::Documents).await?;
    let core = Core::new(core_pool.clone());
    let controls = Entitlements::new(control_pool.clone());
    let terms = Terms::new(pool.clone());
    let actor = |subject: &str, trust| ActorContext {
        issuer: "urn:terms-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "fixture-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let maker = actor("maker", TrustContext::Workforce);
    let checker = actor("checker", TrustContext::Workforce);
    let customer = actor("customer", TrustContext::Customer);
    let foreign = actor("foreign", TrustContext::Customer);
    for (who, role) in [(&maker, "product_admin"), (&checker, "product_approver")] {
        let person = Uuid::new_v4();
        sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
            .bind(&who.issuer)
            .bind(&who.subject)
            .bind(person)
            .execute(&db.migrator)
            .await?;
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).bind(role).execute(&db.migrator).await?;
    }
    let money = |n| Money::new(n, Currency::synthetic());
    let today = Utc::now().date_naive();
    let product = ProductVersion {
        id: Uuid::new_v4(),
        code: "TERMS_FIXTURE".into(),
        version: 1,
        name: "Synthetic account".into(),
        label: "SYNTHETIC".into(),
        effective_from: today - Duration::days(1),
        effective_until: today + Duration::days(30),
        minimum_age: 18,
        payment_limit: money(10000),
        daily_limit: money(20000),
        flat_fee: money(0),
        fee_basis_points: 0,
        chart_version: "synthetic-chart-v1".into(),
    };
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Product,
            product.id,
            &product,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    controls.approve_change(&checker, proposal).await?;
    core.publish_product(&controls, &maker, proposal, &product)
        .await?;
    let draft=TermsDraft{id:Uuid::new_v4(),product_version_id:product.id,version:1,title:"Synthetic demonstration terms".into(),text:"Synthetic demonstration only. TST has no monetary value.\nExact UTF-8 version one: café.".into(),label:"SYNTHETIC".into(),effective_from:product.effective_from,effective_until:product.effective_until};
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Terms,
            draft.id,
            &draft,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    assert!(
        terms
            .publish(&core, &controls, &maker, proposal, &draft)
            .await
            .is_err()
    );
    controls.approve_change(&checker, proposal).await?;
    terms
        .publish(&core, &controls, &maker, proposal, &draft)
        .await?;
    terms
        .publish(&core, &controls, &maker, proposal, &draft)
        .await?;
    let application = Uuid::new_v4();
    let acceptance = terms.accept(&customer, draft.id, application).await?;
    assert_eq!(
        acceptance.id,
        terms.accept(&customer, draft.id, application).await?.id
    );
    assert_eq!(terms.version(draft.id).await?.text, draft.text);
    terms
        .validate_acceptance(&customer, acceptance.id, application, product.id)
        .await?;
    assert!(terms.acceptance(&foreign, acceptance.id).await.is_err());
    assert!(
        terms
            .validate_acceptance(&customer, acceptance.id, Uuid::new_v4(), product.id)
            .await
            .is_err()
    );
    assert!(
        pool.execute("DELETE FROM documents.acceptances")
            .await
            .is_err()
    );
    assert!(
        db.migrator
            .execute("UPDATE documents.terms SET digest='changed'")
            .await
            .is_err()
    );
    let mut changed = draft.clone();
    changed.text = "Changed terms".into();
    assert!(
        terms
            .publish(&core, &controls, &maker, proposal, &changed)
            .await
            .is_err()
    );
    let mut next = draft.clone();
    next.id = Uuid::new_v4();
    next.version = 2;
    next.text = "Synthetic demonstration terms, version two.".into();
    let proposal = controls
        .propose_change(
            &maker,
            ChangeKind::Terms,
            next.id,
            &next,
            Utc::now() + Duration::hours(1),
        )
        .await?;
    controls.approve_change(&checker, proposal).await?;
    terms
        .publish(&core, &controls, &maker, proposal, &next)
        .await?;
    assert!(
        terms
            .validate_acceptance(&customer, acceptance.id, application, product.id)
            .await
            .is_err()
    );
    assert!(
        terms
            .accept(&customer, draft.id, application)
            .await
            .is_err()
    );
    let renewed = terms.accept(&customer, next.id, application).await?;
    assert_ne!(renewed.id, acceptance.id);
    terms
        .validate_acceptance(&customer, renewed.id, application, product.id)
        .await?;
    assert_eq!(terms.version(draft.id).await?.text, draft.text);
    assert_eq!(
        terms.acceptance(&customer, acceptance.id).await?.digest,
        acceptance.digest
    );
    assert!(
        terms
            .current(product.id, today + Duration::days(31))
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM documents.outbox")
            .fetch_one(&pool)
            .await?,
        4
    );
    pool.close().await;
    core_pool.close().await;
    control_pool.close().await;
    db.close().await?;
    Ok(())
}
