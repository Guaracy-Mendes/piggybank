use super::*;
use bank_finance::corrections::{CorrectionInput, CorrectionRequest};
fn input(id: Uuid) -> CorrectionInput {
    CorrectionInput {
        original_batch_id: id,
        reason:
            "Reverse the exact mistaken synthetic funding while preserving its original journal"
                .into(),
    }
}
async fn approved(f: &Fixture, id: Uuid) -> Result<CorrectionRequest> {
    let p =
        f.f.propose_correction(&f.maker, Uuid::new_v4(), &input(id))
            .await?;
    f.f.approve_correction(&f.checker, p.id, &p.details_digest)
        .await
}
async fn journal_count(f: &Fixture) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.db.migrator)
            .await?,
    )
}
#[tokio::test]
async fn exact_independent_native_correction_recovers_registration_execution_ack_and_replay()
-> Result<()> {
    let f = Fixture::new().await?;
    let original = f.source(9_007_199_254_740_993).await?;
    let source = f.core.financial_batch(original).await?;
    let key = Uuid::new_v4();
    let i = input(original);
    let (a, b) = tokio::join!(
        f.f.propose_correction(&f.maker, key, &i),
        f.f.propose_correction(&f.maker, key, &i)
    );
    let a = a?;
    assert_eq!(a.id, b?.id);
    assert_eq!(a.state, "STAGED");
    assert_eq!(
        a.details
            .legs
            .iter()
            .find(|l| l.account_kind == "CUSTOMER")
            .unwrap()
            .reversing_minor,
        "-9007199254740993"
    );
    f.f.controls
        .propose_change_id(
            &f.maker,
            a.id,
            ChangeKind::FinanceCorrection,
            original,
            &a.details,
            a.expires_at,
        )
        .await?;
    assert!(
        f.f.correction_request(&f.maker, a.id)
            .await?
            .registered_at
            .is_none()
    );
    f.f.recover_corrections().await?;
    let alias = actor("funding-maker-alias");
    let person = f.f.controls.person(&f.maker).await?;
    f.f.controls.link_person(&alias, person).await?;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'finance_approver',now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('finance-operations',$1,'APPROVE')").bind(person.0).execute(&f.db.migrator).await?;
    assert!(
        f.f.approve_correction(&alias, a.id, &a.details_digest)
            .await
            .is_err()
    );
    assert!(
        f.f.approve_correction(&f.checker, a.id, &"0".repeat(64))
            .await
            .is_err()
    );
    let a =
        f.f.approve_correction(&f.checker, a.id, &a.details_digest)
            .await?;
    assert!(
        f.f.execute_correction(&f.checker, a.id, &a.details_digest)
            .await
            .is_err()
    );
    // The native money transaction committed, but Finance did not receive its response.
    let native =
        f.f.corrections
            .execute(&f.f.controls, &f.maker, a.id, &a.details)
            .await?;
    let seen = f.f.correction_request(&f.maker, a.id).await?;
    assert_eq!(seen.state, "EXECUTED");
    assert_eq!(seen.receipt.unwrap().batch_id, native.batch_id);
    let (x, y) = tokio::join!(
        f.f.execute_correction(&f.maker, a.id, &a.details_digest),
        f.f.execute_correction(&alias, a.id, &a.details_digest)
    );
    assert_eq!(x?.receipt.unwrap().batch_id, y?.receipt.unwrap().batch_id);
    assert_eq!(f.f.propose_correction(&f.maker, key, &i).await?.id, a.id);
    assert_eq!(journal_count(&f).await?, 2);
    assert_eq!(
        fingerprint(&source)?,
        fingerprint(&f.core.financial_batch(original).await?)?
    );
    for leg in &a.details.legs {
        assert_eq!(
            f.core
                .balance(AccountId(leg.account_id))
                .await?
                .booked
                .minor_units(),
            0
        );
    }
    let mut changed = i.clone();
    changed.reason = "Changed financial correction purpose".into();
    assert!(
        f.f.propose_correction(&f.maker, key, &changed)
            .await
            .is_err()
    );
    assert!(
        f.f.propose_correction(&f.maker, Uuid::new_v4(), &i)
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn scope_rejects_payments_reversals_changed_legs_and_wrong_approval_purpose() -> Result<()> {
    let f = Fixture::new().await?;
    let original = f.source(10000).await?;
    let p = approved(&f, original).await?;
    let receiver = AccountId::new();
    sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'CUSTOMER','ACTIVE',false)").bind(receiver.0).execute(&f.db.migrator).await?;
    let customer = p
        .details
        .legs
        .iter()
        .find(|l| l.account_kind == "CUSTOMER")
        .unwrap()
        .account_id;
    let payment = f
        .core
        .transfer(
            Uuid::new_v4(),
            AccountId(customer),
            receiver,
            money(1000),
            None,
        )
        .await?;
    assert!(
        f.f.propose_correction(&f.maker, Uuid::new_v4(), &input(payment))
            .await
            .is_err()
    );
    let wrong =
        f.f.controls
            .propose_change(
                &f.maker,
                ChangeKind::FinanceChart,
                original,
                &p.details,
                Utc::now() + Duration::hours(1),
            )
            .await?;
    f.f.controls.approve_change(&f.checker, wrong).await?;
    assert!(
        f.f.corrections
            .execute(&f.f.controls, &f.maker, wrong, &p.details)
            .await
            .is_err()
    );
    let native = f.db.module(Module::Core).await?;
    let maker = f.f.controls.person(&f.maker).await?.0;
    let checker = f.f.controls.person(&f.checker).await?.0;
    let mut changed = p.details.clone();
    changed.legs[0].reversing_minor = "1".into();
    assert!(
        sqlx::query("SELECT * FROM core.execute_finance_correction($1,$2,$3,$4,$5)")
            .bind(serde_json::to_value(changed)?)
            .bind(&p.details_digest)
            .bind(p.id)
            .bind(maker)
            .bind(checker)
            .execute(&native)
            .await
            .is_err()
    );
    assert!(
        f.f.execute_correction(&f.maker, p.id, &"0".repeat(64))
            .await
            .is_err()
    );
    assert_eq!(journal_count(&f).await?, 2);
    f.close().await
}
#[tokio::test]
async fn reserved_funds_and_restrictions_block_correction_without_releasing_holds() -> Result<()> {
    let f = Fixture::new().await?;
    let original = f.source(5000).await?;
    let p = approved(&f, original).await?;
    let customer = AccountId(
        p.details
            .legs
            .iter()
            .find(|l| l.account_kind == "CUSTOMER")
            .unwrap()
            .account_id,
    );
    let hold = Uuid::new_v4();
    f.core.reserve(hold, customer, money(5000)).await?;
    assert!(
        f.f.execute_correction(&f.maker, p.id, &p.details_digest)
            .await
            .is_err()
    );
    assert_eq!(f.core.balance(customer).await?.held.minor_units(), 5000);
    assert_eq!(journal_count(&f).await?, 1);
    let r = f.f.correction_request(&f.maker, p.id).await?;
    assert_eq!(r.state, "APPROVED");
    assert!(r.receipt.is_none());
    assert!(
        r.attempts
            .iter()
            .any(|a| a.observed_reason == "NATIVE_RECEIPT_NOT_OBSERVED")
    );
    f.core.release(hold).await?;
    sqlx::query("UPDATE core.ledger_accounts SET debit_blocked=true WHERE id=$1")
        .bind(customer.0)
        .execute(&f.db.migrator)
        .await?;
    assert!(
        f.f.execute_correction(&f.maker, p.id, &p.details_digest)
            .await
            .is_err()
    );
    assert_eq!(f.core.balance(customer).await?.booked.minor_units(), 5000);
    sqlx::query("UPDATE core.ledger_accounts SET debit_blocked=false WHERE id=$1")
        .bind(customer.0)
        .execute(&f.db.migrator)
        .await?;
    let r =
        f.f.execute_correction(&f.maker, p.id, &p.details_digest)
            .await?;
    assert_eq!(r.state, "EXECUTED");
    assert_eq!(f.core.balance(customer).await?.booked.minor_units(), 0);
    assert_eq!(f.core.balance(customer).await?.held.minor_units(), 0);
    assert!(
        f.f.propose_correction(
            &f.maker,
            Uuid::new_v4(),
            &input(r.receipt.unwrap().batch_id)
        )
        .await
        .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn competing_approved_requests_have_one_native_winner_and_immutable_evidence() -> Result<()> {
    let f = Fixture::new().await?;
    let original = f.source(5000).await?;
    let a = approved(&f, original).await?;
    let b = approved(&f, original).await?;
    let (x, y) = tokio::join!(
        f.f.execute_correction(&f.maker, a.id, &a.details_digest),
        f.f.execute_correction(&f.maker, b.id, &b.details_digest)
    );
    assert_ne!(x.is_ok(), y.is_ok());
    assert_eq!(journal_count(&f).await?, 2);
    let (winner, loser) = if let Ok(x) = x { (x, b) } else { (y?, a) };
    assert_eq!(
        f.f.correction_request(&f.maker, loser.id).await?.state,
        "SOURCE_ALREADY_CORRECTED"
    );
    let native = f.db.module(Module::Core).await?;
    assert!(
        sqlx::query("DELETE FROM core.finance_correction_receipts")
            .execute(&native)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE finance.correction_requests SET details='{}'")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM finance.correction_attempts")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM core.finance_correction_receipts")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert_eq!(
        winner
            .receipt
            .unwrap()
            .financial_batch
            .journal
            .original_batch_id,
        Some(original)
    );
    f.close().await
}
#[tokio::test]
async fn live_checker_expiry_after_waiting_for_financial_account_lock_prevents_booking()
-> Result<()> {
    let f = Fixture::new().await?;
    let original = f.source(7000).await?;
    let p = approved(&f, original).await?;
    let customer = p
        .details
        .legs
        .iter()
        .find(|l| l.account_kind == "CUSTOMER")
        .unwrap()
        .account_id;
    let mut guard = f.db.migrator.begin().await?;
    sqlx::query("SELECT id FROM core.ledger_accounts WHERE id=$1 FOR UPDATE")
        .bind(customer)
        .execute(&mut *guard)
        .await?;
    let finance = f.f.clone();
    let id = p.id;
    let digest = p.details_digest.clone();
    let maker = f.maker.clone();
    let task = tokio::spawn(async move { finance.execute_correction(&maker, id, &digest).await });
    let core = f.db.module(Module::Core).await?;
    let mut waiting = false;
    for _ in 0..100 {
        waiting=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename=current_user AND wait_event_type='Lock' AND query LIKE '%core.execute_finance_correction%')").fetch_one(&core).await?;
        if waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        waiting,
        "financial command must wait on the held account lock"
    );
    let checker = f.f.controls.person(&f.checker).await?.0;
    sqlx::query("UPDATE controls.role_grants SET valid_until=clock_timestamp()+interval '200 milliseconds' WHERE person_id=$1 AND role='finance_approver'").bind(checker).execute(&f.db.migrator).await?;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    guard.commit().await?;
    assert!(task.await?.is_err());
    assert_eq!(journal_count(&f).await?, 1);
    assert!(
        f.f.correction_request(&f.maker, p.id)
            .await?
            .receipt
            .is_none()
    );
    f.close().await
}
#[tokio::test]
async fn correction_registration_cursor_passes_fifty_revoked_makers_and_repairs_original_ack()
-> Result<()> {
    let f = Fixture::new().await?;
    let original = f.source(5000).await?;
    let p =
        f.f.propose_correction(&f.maker, Uuid::new_v4(), &input(original))
            .await?;
    let bad = actor("no-correction-maker-authority");
    f.f.controls.link_person(&bad, PartyId::new()).await?;
    let person = f.f.controls.person(&bad).await?.0;
    for n in 1..=50 {
        let id = Uuid::from_u128(n);
        let details = p.source.details(id, &p.details.reason)?;
        sqlx::query("INSERT INTO finance.correction_requests(id,original_batch_id,issuer,subject,client_id,request_key,actor_context,maker_person,source,details,details_digest,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)").bind(id).bind(original).bind(&bad.issuer).bind(&bad.subject).bind(&bad.client_id).bind(Uuid::new_v4()).bind(serde_json::to_value(&bad)?).bind(person).bind(serde_json::to_value(&p.source)?).bind(serde_json::to_value(&details)?).bind(fingerprint(&details)?).bind(p.expires_at).execute(&f.db.migrator).await?;
    }
    f.f.controls
        .propose_change_id(
            &f.maker,
            p.id,
            ChangeKind::FinanceCorrection,
            original,
            &p.details,
            p.expires_at,
        )
        .await?;
    f.f.recover_corrections().await?;
    assert!(
        f.f.correction_request(&f.maker, p.id)
            .await?
            .registered_at
            .is_none()
    );
    f.f.recover_corrections().await?;
    assert!(
        f.f.correction_request(&f.maker, p.id)
            .await?
            .registered_at
            .is_some()
    );
    assert_eq!(journal_count(&f).await?, 1);
    f.close().await
}
#[tokio::test]
async fn revoked_maker_and_trusted_receipt_archive_damage_cannot_claim_an_execution() -> Result<()>
{
    let f = Fixture::new().await?;
    let original = f.source(5000).await?;
    let p = approved(&f, original).await?;
    f.f.execute_correction(&f.maker, p.id, &p.details_digest)
        .await?;
    let maker = f.f.controls.person(&f.maker).await?.0;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=clock_timestamp() WHERE person_id=$1 AND capability='REVIEW'").bind(maker).execute(&f.db.migrator).await?;
    assert!(
        f.f.execute_correction(&f.maker, p.id, &p.details_digest)
            .await
            .is_err()
    );
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=NULL WHERE person_id=$1")
        .bind(maker)
        .execute(&f.db.migrator)
        .await?;
    // Trusted storage repair fault changes the receipt reference, not authoritative financial facts.
    sqlx::query("ALTER TABLE core.finance_correction_receipts DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE core.finance_correction_receipts SET batch_id=$1 WHERE execution_id=$2")
        .bind(original)
        .bind(p.id)
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("ALTER TABLE core.finance_correction_receipts ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(f.f.correction_request(&f.maker, p.id).await.is_err());
    assert_eq!(journal_count(&f).await?, 2);
    f.close().await
}
#[tokio::test]
async fn approval_expiring_during_original_source_lock_wait_cannot_book_a_correction() -> Result<()>
{
    let f = Fixture::new().await?;
    let original = f.source(7000).await?;
    let execution = Uuid::new_v4();
    let source = f.f.corrections.source(original).await?;
    let details = source.details(
        execution,
        "Exact synthetic correction with a short test approval",
    )?;
    f.f.controls
        .propose_change_id(
            &f.maker,
            execution,
            ChangeKind::FinanceCorrection,
            original,
            &details,
            Utc::now() + Duration::seconds(2),
        )
        .await?;
    f.f.controls.approve_change(&f.checker, execution).await?;
    let mut guard = f.db.migrator.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,1))")
        .bind(original.to_string())
        .execute(&mut *guard)
        .await?;
    let commands = f.f.corrections.clone();
    let controls = f.f.controls.clone();
    let maker = f.maker.clone();
    let command = details.clone();
    let task = tokio::spawn(async move {
        commands
            .execute(&controls, &maker, execution, &command)
            .await
    });
    let core = f.db.module(Module::Core).await?;
    let mut waiting = false;
    for _ in 0..100 {
        waiting=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename=current_user AND wait_event_type='Lock' AND query LIKE '%core.execute_finance_correction%')").fetch_one(&core).await?;
        if waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(waiting);
    tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
    guard.commit().await?;
    assert!(task.await?.is_err());
    assert_eq!(journal_count(&f).await?, 1);
    assert!(
        f.f.corrections
            .receipt(execution, &details)
            .await?
            .is_none()
    );
    f.close().await
}
