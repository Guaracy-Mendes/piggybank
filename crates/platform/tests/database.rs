use bank_platform::database::{MIGRATOR, connect};
use sqlx::Executor;

#[tokio::test]
async fn durable_operations_deduplicate_and_fence_expired_workers() -> anyhow::Result<()> {
    use bank_platform::{
        database::Module,
        domain::{ActorContext, TrustContext},
        operations::{Completion, JobSpec, Operations},
        testing::TestDatabase,
    };
    use serde_json::json;
    use uuid::Uuid;
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Platform).await?;
    let operations = Operations::new(pool.clone());
    let actor = ActorContext {
        issuer: "urn:fixture".into(),
        subject: Uuid::new_v4().to_string(),
        trust: TrustContext::Customer,
        client_id: "customer-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let command = json!({"sourceRef":Uuid::new_v4(),"minorUnits":"2500"});
    let job = || JobSpec {
        kind: "test.step".into(),
        step_key: "initial".into(),
        payload: json!({"resourceRef":"synthetic"}),
        delay_seconds: 0,
    };
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let service = operations.clone();
        let actor = actor.clone();
        let command = command.clone();
        let job = job();
        tasks.push(tokio::spawn(async move {
            service
                .accept(&actor, "fixture.command", "same-key", &command, job)
                .await
        }));
    }
    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        ids.insert(task.await??.id);
    }
    assert_eq!(ids.len(), 1);
    let id = *ids.iter().next().unwrap();
    assert_eq!(operations.status(&actor, id).await?.state, "ACCEPTED");
    assert!(
        operations
            .accept(
                &actor,
                "fixture.command",
                "same-key",
                &json!({"minorUnits":"2501"}),
                job()
            )
            .await
            .is_err()
    );
    let mut foreign = actor.clone();
    foreign.subject = Uuid::new_v4().to_string();
    assert!(operations.status(&foreign, id).await.is_err());
    let old = operations.claim(Uuid::new_v4(), 1).await?.unwrap();
    sqlx::query("UPDATE platform.jobs SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(old.id)
        .execute(&db.migrator)
        .await?;
    let current = operations.claim(Uuid::new_v4(), 30).await?.unwrap();
    assert!(current.fence > old.fence);
    assert!(
        operations
            .complete(&old, Completion::Done(json!({"wrong":true})))
            .await
            .is_err()
    );
    assert!(operations.retry(&old, "STALE", 0).await.is_err());
    let next = JobSpec {
        kind: "test.step".into(),
        step_key: "second".into(),
        payload: json!({}),
        delay_seconds: 0,
    };
    operations
        .complete(&current, Completion::Continue(next.clone()))
        .await?;
    operations
        .complete(&current, Completion::Continue(next))
        .await?; // Lost commit acknowledgement retry.
    let last = operations.claim(Uuid::new_v4(), 30).await?.unwrap();
    operations
        .complete(&last, Completion::Done(json!({"resourceRef":"done"})))
        .await?;
    operations
        .complete(&last, Completion::Done(json!({"resourceRef":"done"})))
        .await?;
    assert!(
        operations
            .complete(&last, Completion::Done(json!({"resourceRef":"changed"})))
            .await
            .is_err()
    );
    assert_eq!(
        Operations::new(pool.clone())
            .status(&actor, id)
            .await?
            .state,
        "SUCCEEDED"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM platform.jobs WHERE operation_id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 2);
    let pending = operations
        .accept(&actor, "fixture.command", "exhausted", &command, job())
        .await?;
    sqlx::query("UPDATE platform.jobs SET max_attempts=1 WHERE operation_id=$1")
        .bind(pending.id)
        .execute(&db.migrator)
        .await?;
    let lease = operations.claim(Uuid::new_v4(), 30).await?.unwrap();
    operations.retry(&lease, "PROVIDER_UNKNOWN", 0).await?;
    assert_eq!(
        operations.status(&actor, pending.id).await?.state,
        "REVIEW_REQUIRED"
    );
    let invalid = JobSpec {
        kind: String::new(),
        ..job()
    };
    assert!(
        operations
            .accept(&actor, "fixture.command", "invalid-job", &command, invalid)
            .await
            .is_err()
    );
    let accepted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform.operations WHERE idempotency_key='invalid-job'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(accepted, 0);
    pool.close().await;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn migrations_and_module_grants_hold_on_real_postgresql() -> anyhow::Result<()> {
    let admin_url = std::env::var("BANK_TEST_ADMIN_URL").expect("Real PostgreSQL integration requires BANK_TEST_ADMIN_URL; use make test-integration (no skipped test)");
    let admin = connect(&admin_url).await?;
    let database = format!("piggybank_test_{}", uuid::Uuid::new_v4().simple());
    // Identifier is generated exclusively from a fixed prefix and a UUID's hex;
    // PostgreSQL does not support bind parameters for database identifiers.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE {database} OWNER bank_migrator"
    )))
    .execute(&admin)
    .await?;
    let base = url::Url::parse(&admin_url)?;
    let role_url = |module: &str, password: &str| -> String {
        let mut value = base.clone();
        value.set_path(&format!("/{database}"));
        value.set_username(module).unwrap();
        value.set_password(Some(password)).unwrap();
        value.into()
    };
    let migrator = connect(&role_url("bank_migrator", "synthetic-local-only")).await?;
    let result = async {
        MIGRATOR.run(&migrator).await?;
        MIGRATOR.run(&migrator).await?;
        let applied: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
                .fetch_one(&migrator)
                .await?;
        assert_eq!(applied, MIGRATOR.iter().count() as i64);
        migrator
            .execute("CREATE TABLE core.ownership_probe(id integer PRIMARY KEY)")
            .await?;
        let core = connect(&role_url("bank_core", "synthetic-core-only")).await?;
        let platform = connect(&role_url("bank_platform", "synthetic-platform-only")).await?;
        core.execute("INSERT INTO core.ownership_probe VALUES(1)")
            .await?;
        assert!(
            platform
                .execute("SELECT * FROM core.ownership_probe")
                .await
                .is_err()
        );
        assert!(
            core.execute("CREATE TABLE core.escape(id integer)")
                .await
                .is_err()
        );
        assert!(
            core.execute("ALTER TABLE core.ownership_probe ADD COLUMN escaped bool")
                .await
                .is_err()
        );
        assert!(
            core.execute("CREATE TABLE public.escape(id integer)")
                .await
                .is_err()
        );
        assert!(core.execute("SET ROLE bank_migrator").await.is_err());
        let mut transaction = core.begin().await?;
        sqlx::query("INSERT INTO core.ownership_probe VALUES(2)")
            .execute(&mut *transaction)
            .await?;
        transaction.rollback().await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core.ownership_probe")
            .fetch_one(&core)
            .await?;
        assert_eq!(count, 1);
        core.close().await;
        platform.close().await;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    migrator.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&admin)
    .await?;
    admin.close().await;
    result
}

#[tokio::test]
async fn transactional_events_survive_redelivery_gaps_and_replay() -> anyhow::Result<()> {
    use bank_platform::{
        database::Module,
        domain::{ActorContext, EventEnvelope, TrustContext},
        events::{EventStore, ReplayAuthorizer, jetstream_named},
        operations::{Completion, JobSpec, Operations},
        testing::TestDatabase,
    };
    use futures_util::{StreamExt, future::BoxFuture};
    use serde_json::json;
    use std::time::Duration;
    use uuid::Uuid;
    struct Authorization(bool);
    impl ReplayAuthorizer for Authorization {
        fn authorize_replay<'a>(
            &'a self,
            _: &'a ActorContext,
        ) -> BoxFuture<'a, anyhow::Result<bool>> {
            Box::pin(async move { Ok(self.0) })
        }
    }
    let db = TestDatabase::new().await?;
    let platform = db.module(Module::Platform).await?;
    let data = db.module(Module::Data).await?;
    let producer = EventStore::new(platform.clone(), Module::Platform);
    let consumer = EventStore::new(data.clone(), Module::Data);
    let namespace = format!("bank.rust_test_{}", Uuid::new_v4().simple());
    let stream_name = format!("BANK_RUST_TEST_{}", Uuid::new_v4().simple());
    let js = jetstream_named(
        &std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:14222".into()),
        &stream_name,
        &namespace,
    )
    .await?;
    let stream = js.get_stream(&stream_name).await?;
    let name = format!("test_{}", Uuid::new_v4().simple());
    let pull = stream
        .get_or_create_consumer(
            &name,
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some(name.clone()),
                filter_subject: format!("{namespace}.platform.operation.changed"),
                deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New,
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_millis(300),
                max_ack_pending: 4,
                ..Default::default()
            },
        )
        .await?;
    let actor = ActorContext {
        issuer: "urn:event-test".into(),
        subject: Uuid::new_v4().to_string(),
        trust: TrustContext::Customer,
        client_id: "customer-web".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let operations = Operations::new(platform.clone());
    let status = operations
        .accept(
            &actor,
            "platform.probe",
            "event-test",
            &json!({}),
            JobSpec {
                kind: "platform.probe".into(),
                step_key: "probe".into(),
                payload: json!({}),
                delay_seconds: 0,
            },
        )
        .await?;
    // No publisher was running: committed state and its exact event already exist together.
    let first: serde_json::Value = sqlx::query_scalar(
        "SELECT document FROM platform.outbox WHERE aggregate_id=$1 AND aggregate_version=1",
    )
    .bind(status.id)
    .fetch_one(&platform)
    .await?;
    let first: EventEnvelope = serde_json::from_value(first)?;
    let mut rolled_back = first.clone();
    rolled_back.event_id = bank_platform::domain::EventId::new();
    rolled_back.aggregate_id = Uuid::new_v4();
    let mut tx = platform.begin().await?;
    producer.stage(&mut tx, &rolled_back).await?;
    tx.rollback().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM platform.outbox WHERE event_id=$1")
            .bind(rolled_back.event_id.0)
            .fetch_one(&platform)
            .await?,
        0
    );
    let mut future = first.clone();
    future.event_id = bank_platform::domain::EventId::new();
    future.aggregate_version = 2;
    future.data["state"] = json!("RUNNING");
    assert!(!consumer.consume_operation(&name, &future).await?);
    assert!(
        consumer
            .replay(&name, future.event_id.0, &actor, &Authorization(false))
            .await
            .is_err()
    );
    assert!(producer.publish_one_to(&js, &namespace).await?);
    let mut messages = pull.stream().max_messages_per_batch(1).messages().await?;
    let message = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await?
        .unwrap()?;
    let delivered: EventEnvelope = serde_json::from_slice(&message.payload)?;
    assert_eq!(delivered.event_id, first.event_id);
    assert!(consumer.consume_operation(&name, &delivered).await?);
    // Crash after local commit and before ACK: broker redelivery applies no second effect.
    drop(message);
    drop(messages);
    let mut messages = pull.stream().max_messages_per_batch(1).messages().await?;
    let duplicate = tokio::time::timeout(Duration::from_secs(5), messages.next())
        .await?
        .unwrap()?;
    assert!(
        duplicate
            .info()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .delivered
            >= 2
    );
    assert!(
        consumer
            .consume_operation(&name, &serde_json::from_slice(&duplicate.payload)?)
            .await?
    );
    duplicate
        .double_ack()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    assert!(
        consumer
            .replay(&name, future.event_id.0, &actor, &Authorization(true))
            .await?
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM data.inbox WHERE consumer=$1")
        .bind(&name)
        .fetch_one(&data)
        .await?;
    assert_eq!(count, 2);
    let mut conflict = first.clone();
    conflict.data["kind"] = json!("altered");
    assert!(!consumer.consume_operation(&name, &conflict).await?);
    let mut unsupported = first.clone();
    unsupported.event_id = bank_platform::domain::EventId::new();
    unsupported.schema_version = 2;
    assert!(!consumer.consume_operation(&name, &unsupported).await?);
    consumer.quarantine_raw(&name, b"invalid JSON").await?;
    consumer.quarantine_raw(&name, b"invalid JSON").await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM data.invalid_deliveries")
            .fetch_one(&data)
            .await?,
        1
    );
    let lease = operations.claim(Uuid::new_v4(), 30).await?.unwrap();
    operations
        .complete(&lease, Completion::Done(json!({})))
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM platform.outbox WHERE aggregate_id=$1")
            .bind(status.id)
            .fetch_one(&platform)
            .await?,
        3
    );
    stream.delete_consumer(&name).await?;
    js.delete_stream(&stream_name).await?;
    platform.close().await;
    data.close().await;
    db.close().await?;
    Ok(())
}

#[tokio::test]
async fn shared_role_pools_reuse_connections_without_crossing_database_or_authority()
-> anyhow::Result<()> {
    use bank_platform::{
        database::{Module, connect_shared},
        testing::TestDatabase,
    };
    let db = TestDatabase::new().await?;
    let payments_url = db.module_url(Module::Payments);
    let first = connect_shared(&payments_url).await?;
    let second = connect_shared(&payments_url).await?;
    let mut one = first.acquire().await?;
    let mut two = second.acquire().await?;
    let pid_one: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *one)
        .await?;
    let pid_two: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *two)
        .await?;
    assert_ne!(pid_one, pid_two);
    assert_eq!(first.size(), second.size());
    assert_eq!(first.size(), 2);
    let core = connect_shared(&db.module_url(Module::Core)).await?;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT current_user::text")
            .fetch_one(&core)
            .await?,
        "bank_core"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT current_user::text")
            .fetch_one(&mut *one)
            .await?,
        "bank_payments"
    );
    let separate = TestDatabase::new().await?;
    let other = connect_shared(&separate.module_url(Module::Payments)).await?;
    let original_database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&mut *one)
        .await?;
    let other_database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&other)
        .await?;
    assert_ne!(original_database, other_database);
    other.close().await;
    separate.close().await?;
    assert!(
        sqlx::query("SELECT * FROM core.ledger_accounts")
            .fetch_all(&mut *one)
            .await
            .is_err()
    );
    drop(one);
    drop(two);
    first.close().await;
    assert!(second.is_closed());
    let reopened = connect_shared(&payments_url).await?;
    assert!(!reopened.is_closed());
    reopened.close().await;
    core.close().await;
    db.close().await
}
