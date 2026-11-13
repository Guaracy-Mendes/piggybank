use super::*;
use bank_controls::audit::{AuditLog, AuditRecord};
use serde_json::Value;

#[tokio::test]
async fn relay_progresses_past_fifty_malformed_events_without_discarding_them() -> Result<()> {
    let f = Fixture::new().await?;
    for n in 1..=50 {
        sqlx::query("INSERT INTO finance.audit_pending(id,record) VALUES($1,$2)")
            .bind(Uuid::from_u128(n))
            .bind(json!({"damaged":true}))
            .execute(&f.f.pool)
            .await?;
    }
    let good = Uuid::from_u128(u128::MAX - 1);
    let record = AuditRecord::new(&f.maker, "finance.test.relay", good, good);
    sqlx::query("INSERT INTO finance.audit_pending(id,record) VALUES($1,$2)")
        .bind(good)
        .bind(serde_json::to_value(&record)?)
        .execute(&f.f.pool)
        .await?;
    for _ in 0..5 {
        let _ = f.f.relay_audit().await;
    }
    let delivered: bool = sqlx::query_scalar(
        "SELECT delivered_at IS NOT NULL FROM finance.audit_pending WHERE id=$1",
    )
    .bind(good)
    .fetch_one(&f.f.pool)
    .await?;
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM finance.audit_pending WHERE id<=$1 AND delivered_at IS NULL",
    )
    .bind(Uuid::from_u128(50))
    .fetch_one(&f.f.pool)
    .await?;
    let verified = AuditLog::new(f.f.controls.pool.clone())
        .verify("finance")
        .await?;
    f.close().await?;
    assert!(
        delivered,
        "valid event must not starve behind malformed audit records"
    );
    assert_eq!(retained, 50);
    assert!(verified);
    Ok(())
}

async fn enqueue(f: &Fixture, id: Uuid, record: &AuditRecord) -> Result<()> {
    sqlx::query("INSERT INTO finance.audit_pending(id,record) VALUES($1,$2)")
        .bind(id)
        .bind(serde_json::to_value(record)?)
        .execute(&f.f.pool)
        .await?;
    Ok(())
}
#[tokio::test]
async fn native_receipt_recovers_committed_controls_append_without_reappend() -> Result<()> {
    let f = Fixture::new().await?;
    let id = Uuid::new_v4();
    let record = AuditRecord::new(&f.maker, "finance.test.ack", id, id);
    enqueue(&f, id, &record).await?;
    let digest = AuditLog::new(f.f.controls.pool.clone())
        .append(id, "finance", &record)
        .await?;
    sqlx::query(
        "REVOKE EXECUTE ON FUNCTION controls.append_audit(uuid,text,jsonb) FROM bank_controls",
    )
    .execute(&f.db.migrator)
    .await?;
    // No append permission remains. Receipt inquiry must independently recover the accepted event.
    f.f.relay_audit().await?;
    sqlx::query(
        "GRANT EXECUTE ON FUNCTION controls.append_audit(uuid,text,jsonb) TO bank_controls",
    )
    .execute(&f.db.migrator)
    .await?;
    let info = f.f.audit_delivery(&f.maker, id).await?;
    assert_eq!(info.state, "DELIVERED");
    assert_eq!(info.receipt.unwrap().controls_digest, digest);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM controls.audit_events WHERE id=$1")
            .bind(id)
            .fetch_one(&f.f.controls.pool)
            .await?,
        1
    );
    assert_eq!(f.f.audit_delivery(&f.checker, id).await?.state, "DELIVERED");
    assert!(f.f.audit_delivery(&actor("unassigned"), id).await.is_err());
    for _ in 0..3 {
        f.f.relay_audit().await?;
    }
    assert_eq!(
        sqlx::query_scalar::<_, Value>("SELECT record FROM finance.audit_pending WHERE id=$1")
            .bind(id)
            .fetch_one(&f.f.pool)
            .await?,
        serde_json::to_value(record)?
    );
    f.close().await
}
#[tokio::test]
async fn identity_conflict_and_invalid_codes_remain_pending_while_later_events_deliver()
-> Result<()> {
    let f = Fixture::new().await?;
    let bad = Uuid::from_u128(1);
    let good = Uuid::from_u128(3);
    let original = AuditRecord::new(&f.maker, "finance.test.original", bad, bad);
    AuditLog::new(f.f.controls.pool.clone())
        .append(bad, "different-stream", &original)
        .await?;
    enqueue(&f, bad, &original).await?;
    let invalid = Uuid::from_u128(2);
    let mut invalid_record = AuditRecord::new(&f.maker, "finance.test.invalid", invalid, invalid);
    invalid_record.resource = "arbitrary-text-is-not-a-resource-uuid".into();
    enqueue(&f, invalid, &invalid_record).await?;
    enqueue(
        &f,
        good,
        &AuditRecord::new(&f.maker, "finance.test.good", good, good),
    )
    .await?;
    assert!(f.f.relay_audit().await.is_err());
    assert_eq!(
        f.f.audit_delivery(&f.maker, bad)
            .await?
            .last_attempt
            .unwrap()
            .outcome,
        "AUDIT_IDENTITY_CONFLICT"
    );
    assert_eq!(
        f.f.audit_delivery(&f.maker, invalid)
            .await?
            .last_attempt
            .unwrap()
            .outcome,
        "INVALID_RECORD"
    );
    assert_eq!(f.f.audit_delivery(&f.maker, good).await?.state, "DELIVERED");
    assert_eq!(
        f.f.audit_delivery(&f.maker, bad).await?.state,
        "PENDING_RETRY"
    );
    let status = f.f.audit_delivery_status(&f.checker).await?;
    assert_eq!(status.pending_count, "2");
    assert_eq!(status.confirmed_count, "1");
    f.close().await
}
#[tokio::test]
async fn delivery_flags_receipts_attempts_and_native_links_cannot_be_forged_or_rewritten()
-> Result<()> {
    let f = Fixture::new().await?;
    let source = f.source(250).await?;
    let native_before = f.core.financial_batch(source).await?;
    let id = Uuid::new_v4();
    let record = AuditRecord::new(&f.maker, "finance.test.guards", id, id);
    enqueue(&f, id, &record).await?;
    assert!(
        sqlx::query("UPDATE finance.audit_pending SET delivered_at=clock_timestamp() WHERE id=$1")
            .bind(id)
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO finance.audit_pending(id,record,delivered_at) VALUES($1,$2,clock_timestamp())").bind(Uuid::new_v4()).bind(serde_json::to_value(&record)?).execute(&f.f.pool).await.is_err());
    assert!(sqlx::query("INSERT INTO finance.audit_delivery_receipts VALUES($1,repeat('a',64),1,clock_timestamp(),'NATIVE_CONFIRMATION')").bind(id).execute(&f.f.pool).await.is_err());
    assert!(
        sqlx::query("SELECT finance.confirm_audit_delivery($1)")
            .bind(id)
            .execute(&f.core.pool)
            .await
            .is_err()
    );
    f.f.relay_audit().await?;
    assert_eq!(f.f.audit_delivery(&f.maker, id).await?.state, "DELIVERED");
    assert!(
        sqlx::query("UPDATE finance.audit_delivery_attempts SET outcome='INVALID_RECORD'")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("TRUNCATE finance.audit_delivery_receipts")
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    assert_eq!(
        fingerprint(&native_before)?,
        fingerprint(&f.core.financial_batch(source).await?)?
    );
    sqlx::query("ALTER TABLE finance.audit_delivery_receipts DISABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    sqlx::query("UPDATE finance.audit_delivery_receipts SET controls_digest=repeat('b',64) WHERE audit_id=$1").bind(id).execute(&f.db.migrator).await?;
    sqlx::query("ALTER TABLE finance.audit_delivery_receipts ENABLE TRIGGER USER")
        .execute(&f.db.migrator)
        .await?;
    assert!(f.f.audit_delivery(&f.maker, id).await.is_err());
    f.close().await
}
#[tokio::test]
async fn exact_raw_record_canonical_form_is_not_silently_rewritten_during_relay() -> Result<()> {
    let f = Fixture::new().await?;
    let id = Uuid::new_v4();
    let record = AuditRecord::new(&f.maker, "finance.test.raw", id, id);
    let mut raw = serde_json::to_value(record)?;
    raw["occurredAt"] = json!(Utc::now().to_rfc3339()); // +00:00 differs from the original canonical UTC wire form.
    sqlx::query("INSERT INTO finance.audit_pending(id,record) VALUES($1,$2)")
        .bind(id)
        .bind(&raw)
        .execute(&f.f.pool)
        .await?;
    assert!(f.f.relay_audit().await.is_err());
    let info = f.f.audit_delivery(&f.maker, id).await?;
    assert_eq!(info.last_attempt.unwrap().outcome, "NON_CANONICAL_RECORD");
    assert!(info.receipt.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, Value>("SELECT record FROM finance.audit_pending WHERE id=$1")
            .bind(id)
            .fetch_one(&f.f.pool)
            .await?,
        raw
    );
    f.close().await
}
#[tokio::test]
async fn forward_upgrade_preserves_legacy_time_and_requires_original_native_evidence() -> Result<()>
{
    use bank_platform::database::MIGRATOR;
    for has_native in [true, false] {
        let db = TestDatabase::at_version(51).await?;
        let finance = db.module(Module::Finance).await?;
        let controls = db.module(Module::Controls).await?;
        let id = Uuid::new_v4();
        let record = AuditRecord::new(&actor("legacy"), "finance.test.legacy", id, id);
        sqlx::query("INSERT INTO finance.audit_pending(id,record) VALUES($1,$2)")
            .bind(id)
            .bind(serde_json::to_value(&record)?)
            .execute(&finance)
            .await?;
        if has_native {
            AuditLog::new(controls.clone())
                .append(id, "finance", &record)
                .await?;
        }
        let original = Utc::now();
        sqlx::query("UPDATE finance.audit_pending SET delivered_at=$2 WHERE id=$1")
            .bind(id)
            .bind(original)
            .execute(&finance)
            .await?;
        let result = MIGRATOR.run(&db.migrator).await;
        if has_native {
            assert!(result.is_ok());
            let receipt: Value = sqlx::query_scalar("SELECT finance.audit_delivery_receipt($1)")
                .bind(id)
                .fetch_one(&finance)
                .await?;
            assert_eq!(receipt["origin"], "LEGACY_VERIFIED");
            let saved: chrono::DateTime<Utc> =
                sqlx::query_scalar("SELECT delivered_at FROM finance.audit_pending WHERE id=$1")
                    .bind(id)
                    .fetch_one(&finance)
                    .await?;
            assert_eq!(saved.timestamp_micros(), original.timestamp_micros());
        } else {
            assert!(result.is_err());
            let version: i64 =
                sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
                    .fetch_one(&db.migrator)
                    .await?;
            assert_eq!(version, 51);
        }
        finance.close().await;
        controls.close().await;
        db.close().await?;
    }
    Ok(())
}

#[tokio::test]
async fn concurrent_workers_keep_one_native_event_and_persist_wrapping_progress_for_late_lower_ids()
-> Result<()> {
    let f = Fixture::new().await?;
    for n in 100..170 {
        let id = Uuid::from_u128(n);
        enqueue(
            &f,
            id,
            &AuditRecord::new(&f.maker, "finance.test.concurrent", id, id),
        )
        .await?;
    }
    let first = f.f.clone();
    let second = f.f.clone();
    let reader = f.f.clone();
    let viewer = f.maker.clone();
    let (a, b, c) = tokio::join!(first.relay_audit(), second.relay_audit(), async move {
        for _ in 0..3 {
            let status = reader.audit_delivery_status(&viewer).await?;
            for item in status.items {
                assert_eq!(item.delivered_at.is_some(), item.receipt.is_some());
                assert_eq!(item.state == "DELIVERED", item.receipt.is_some());
                if let Some(receipt) = item.receipt {
                    assert_eq!(receipt.audit_id, item.id);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    });
    a?;
    b?;
    c?;
    for _ in 0..3 {
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM finance.audit_pending WHERE delivered_at IS NULL",
        )
        .fetch_one(&f.f.pool)
        .await?;
        if pending == 0 {
            break;
        }
        f.f.clone().relay_audit().await?;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM controls.audit_events WHERE stream='finance'"
        )
        .fetch_one(&f.f.controls.pool)
        .await?,
        70
    );
    let late = Uuid::nil();
    enqueue(
        &f,
        late,
        &AuditRecord::new(&f.maker, "finance.test.late", late, late),
    )
    .await?;
    let after: Uuid = sqlx::query_scalar("SELECT last_id FROM finance.audit_relay_cursor WHERE id")
        .fetch_one(&f.f.pool)
        .await?;
    assert!(after > late);
    f.f.clone().relay_audit().await?; // Empty suffix wraps without discarding the late event.
    assert!(f.f.audit_delivery(&f.maker, late).await?.receipt.is_some());
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT last_id FROM finance.audit_relay_cursor WHERE id")
            .fetch_one(&f.f.pool)
            .await?,
        Uuid::nil()
    );
    f.f.clone().relay_audit().await?;
    assert_eq!(f.f.audit_delivery(&f.maker, late).await?.state, "DELIVERED");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM controls.audit_events WHERE stream='finance'"
        )
        .fetch_one(&f.f.controls.pool)
        .await?,
        71
    );
    assert!(
        AuditLog::new(f.f.controls.pool.clone())
            .verify("finance")
            .await?
    );
    f.close().await
}
