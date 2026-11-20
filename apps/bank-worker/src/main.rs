use bank_platform::{
    config::Config,
    database::Module,
    domain::EventEnvelope,
    events::{EventStore, jetstream},
    health, init_tracing,
    operations::{Completion, Operations},
    shutdown_signal,
};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::{sync::watch, task::JoinSet};
use uuid::Uuid;

async fn relay(
    stores: Vec<EventStore>,
    js: async_nats::jetstream::Context,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        for store in &stores {
            if *stop.borrow() {
                return;
            }
            if store.publish_one(&js).await.is_err() {
                tracing::warn!(
                    module = store.module.name(),
                    code = "OUTBOX_RETRY",
                    "event publication pending"
                );
            }
        }
        tokio::select! { _ = stop.changed() => return, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
    }
}
async fn project(
    store: EventStore,
    js: async_nats::jetstream::Context,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let stream = js.get_stream(bank_platform::events::stream_name()?).await?;
    let consumer = stream
        .get_or_create_consumer(
            "data-operation-v1",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("data-operation-v1".into()),
                filter_subject: format!(
                    "{}.platform.operation.changed",
                    bank_platform::events::subject_prefix()?
                ),
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_secs(10),
                max_ack_pending: 32,
                max_batch: 32,
                ..Default::default()
            },
        )
        .await?;
    let mut messages = consumer
        .stream()
        .max_messages_per_batch(16)
        .messages()
        .await?;
    loop {
        let next =
            tokio::select! { _ = stop.changed() => return Ok(()), next = messages.next() => next };
        let Some(message) = next else {
            anyhow::bail!("Consumer stream ended")
        };
        let message = message?;
        let applied = match serde_json::from_slice::<EventEnvelope>(&message.payload) {
            Ok(event) if event.validate().is_ok() => store
                .consume_operation("data-operation-v1", &event)
                .await
                .map(|_| ()),
            _ => {
                store
                    .quarantine_raw("data-operation-v1", &message.payload)
                    .await
            }
        };
        if applied.is_ok() {
            message
                .double_ack()
                .await
                .map_err(|_| anyhow::anyhow!("Broker acknowledgement pending"))?;
        } else {
            tracing::warn!(
                code = "PROJECTION_RETRY",
                "projection remains unacknowledged"
            );
        }
    }
}
async fn project_core(
    data: std::sync::Arc<bank_data::Data>,
    js: async_nats::jetstream::Context,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let stream = js.get_stream(bank_platform::events::stream_name()?).await?;
    let consumer = stream
        .get_or_create_consumer(
            bank_data::CONSUMER,
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some(bank_data::CONSUMER.into()),
                filter_subject: format!("{}.core.>", bank_platform::events::subject_prefix()?),
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                ack_wait: Duration::from_secs(10),
                max_ack_pending: 32,
                max_batch: 32,
                ..Default::default()
            },
        )
        .await?;
    let mut messages = consumer
        .stream()
        .max_messages_per_batch(16)
        .messages()
        .await?;
    loop {
        let next = tokio::select! {_=stop.changed()=>return Ok(()),next=messages.next()=>next};
        let Some(message) = next else {
            anyhow::bail!("Core consumer ended")
        };
        let message = message?;
        let result = match serde_json::from_slice::<EventEnvelope>(&message.payload) {
            Ok(event) if event.validate().is_ok() => data.consume(&event).await.map(|_| ()),
            _ => {
                EventStore::new(data.pool.clone(), Module::Data)
                    .quarantine_raw(bank_data::CONSUMER, &message.payload)
                    .await
            }
        };
        if result.is_ok() {
            message
                .double_ack()
                .await
                .map_err(|_| anyhow::anyhow!("Core projection acknowledgement pending"))?;
        } else {
            tracing::warn!(
                code = "CORE_PROJECTION_RETRY",
                "Core fact remains unacknowledged"
            );
        }
    }
}
async fn work(
    operations: Operations,
    payments: std::sync::Arc<bank_payments::Payments>,
    finance: std::sync::Arc<bank_finance::Finance>,
    js: async_nats::jetstream::Context,
    journeys: std::sync::Arc<bank_journeys::LocalJourneys>,
    mut stop: watch::Receiver<bool>,
) {
    let owner = Uuid::new_v4();
    loop {
        if *stop.borrow() {
            return;
        }
        match operations.claim(owner, 30).await {
            Ok(Some(lease)) => {
                // Handlers must be registered explicitly. A diagnostic probe has no financial effects.
                let result = if lease.kind == "platform.probe" {
                    match js.query_account().await {
                        Ok(_) => {
                            operations
                                .complete(
                                    &lease,
                                    Completion::Done(serde_json::json!({"probe":"healthy"})),
                                )
                                .await
                        }
                        Err(_) => operations.retry(&lease, "DEPENDENCY_PENDING", 2).await,
                    }
                } else if lease.kind == "finance.periodic" {
                    let id = lease
                        .payload
                        .get("runId")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                    if let Some(id) = id {
                        match finance.process_periodic_run(id).await {
                            Ok(receipt) => {
                                operations
                                    .complete(
                                        &lease,
                                        Completion::Done(
                                            serde_json::to_value(receipt)
                                                .expect("Period receipt serializes"),
                                        ),
                                    )
                                    .await
                            }
                            Err(_) => {
                                operations
                                    .retry(&lease, "NATIVE_PERIOD_REQUIRES_REVIEW", 3)
                                    .await
                            }
                        }
                    } else {
                        operations.retry(&lease, "INVALID_PERIODIC_JOB", 30).await
                    }
                } else if lease.kind == "finance.export" {
                    let id = lease
                        .payload
                        .get("exportId")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                    if let Some(id) = id {
                        let generation = lease
                            .payload
                            .get("generation")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        match finance.process_export_at(id, generation).await {
                            Ok(
                                done @ (bank_finance::Progress::Done { .. }
                                | bank_finance::Progress::Superseded { .. }),
                            ) => {
                                operations
                                    .complete(
                                        &lease,
                                        Completion::Done(
                                            serde_json::to_value(done)
                                                .expect("Finance progress serializes"),
                                        ),
                                    )
                                    .await
                            }
                            Ok(bank_finance::Progress::Pending) => {
                                operations.retry(&lease, "GL_OUTCOME_PENDING", 3).await
                            }
                            Ok(bank_finance::Progress::Review { code }) => {
                                operations.complete(&lease, Completion::Review(code)).await
                            }
                            Err(_) => operations.retry(&lease, "GL_RECOVERY_PENDING", 3).await,
                        }
                    } else {
                        operations.retry(&lease, "INVALID_FINANCE_JOB", 30).await
                    }
                } else if lease.kind == "credit.allocate" {
                    let id = lease
                        .payload
                        .get("movementId")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                    if let Some(id) = id {
                        match payments.process_credit(id).await {
                            Ok(done @ bank_payments::Progress::Done { .. }) => {
                                operations
                                    .complete(
                                        &lease,
                                        Completion::Done(
                                            serde_json::to_value(done)
                                                .expect("Credit progress serializes"),
                                        ),
                                    )
                                    .await
                            }
                            Ok(bank_payments::Progress::Pending) => {
                                operations.retry(&lease, "CREDIT_CONTROLS_PENDING", 3).await
                            }
                            Ok(bank_payments::Progress::Review { code }) => {
                                match payments.own_credit_exception(id).await {
                                    Ok(()) => {
                                        operations.complete(&lease, Completion::Review(code)).await
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            Err(_) => operations.retry(&lease, "CREDIT_RECOVERY_PENDING", 3).await,
                        }
                    } else {
                        operations.retry(&lease, "INVALID_CREDIT_JOB", 30).await
                    }
                } else if lease.kind == "payment.recover" {
                    let id = lease
                        .payload
                        .get("requestId")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                    if let Some(id) = id {
                        match payments.process_recovery(id).await {
                            Ok(done @ bank_payments::Progress::Done { .. }) => {
                                operations
                                    .complete(
                                        &lease,
                                        Completion::Done(
                                            serde_json::to_value(done)
                                                .expect("Recovery progress primitives serialize"),
                                        ),
                                    )
                                    .await
                            }
                            Ok(bank_payments::Progress::Pending) => {
                                operations
                                    .retry(&lease, "ORIGINAL_PAYMENT_OUTCOME_PENDING", 3)
                                    .await
                            }
                            Ok(bank_payments::Progress::Review { code }) => {
                                operations.complete(&lease, Completion::Review(code)).await
                            }
                            Err(_) => {
                                operations
                                    .retry(&lease, "PAYMENT_RECOVERY_PENDING", 3)
                                    .await
                            }
                        }
                    } else {
                        operations.retry(&lease, "INVALID_RECOVERY_JOB", 30).await
                    }
                } else if lease.kind == "payment.execute" {
                    let id = lease
                        .payload
                        .get("paymentId")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                    if let Some(id) = id {
                        match payments.process(id).await {
                            Ok(done @ bank_payments::Progress::Done { .. }) => {
                                operations
                                    .complete(
                                        &lease,
                                        Completion::Done(
                                            serde_json::to_value(done)
                                                .expect("Payment progress primitives serialize"),
                                        ),
                                    )
                                    .await
                            }
                            Ok(bank_payments::Progress::Pending) => {
                                operations
                                    .retry(&lease, "PAYMENT_CONTROLS_PENDING", 3)
                                    .await
                            }
                            Ok(bank_payments::Progress::Review { code }) => {
                                match payments.own_exception(id).await {
                                    Ok(()) => {
                                        operations.complete(&lease, Completion::Review(code)).await
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            Err(_) => {
                                operations
                                    .retry(&lease, "PAYMENT_RECOVERY_PENDING", 3)
                                    .await
                            }
                        }
                    } else {
                        operations.retry(&lease, "INVALID_PAYMENT_JOB", 30).await
                    }
                } else if lease.kind == "profile.contact" {
                    let id = lease
                        .payload
                        .get("changeId")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                    if let Some(id) = id {
                        match journeys.process_contact(id).await {
                            Ok(done @ bank_journeys::Progress::Done { .. }) => {
                                operations
                                    .complete(
                                        &lease,
                                        Completion::Done(
                                            serde_json::to_value(done)
                                                .expect("Progress primitives serialize"),
                                        ),
                                    )
                                    .await
                            }
                            Ok(bank_journeys::Progress::Pending) => {
                                operations.retry(&lease, "CONTACT_PENDING", 3).await
                            }
                            Ok(bank_journeys::Progress::Review { code }) => {
                                match journeys.own_contact_exception(id, lease.operation_id).await {
                                    Ok(()) => {
                                        operations
                                            .complete(&lease, Completion::Review(code.into()))
                                            .await
                                    }
                                    Err(error) => Err(error),
                                }
                            }
                            Err(_) => {
                                operations
                                    .retry(&lease, "CONTACT_RECOVERY_PENDING", 3)
                                    .await
                            }
                        }
                    } else {
                        operations.retry(&lease, "INVALID_CONTACT_JOB", 30).await
                    }
                } else if lease.kind == "journey.onboard" {
                    let application = lease
                        .payload
                        .get("applicationId")
                        .cloned()
                        .and_then(|v| serde_json::from_value::<Uuid>(v).ok());
                    let generation = lease
                        .payload
                        .get("generation")
                        .and_then(|v| v.as_i64())
                        .and_then(|v| i32::try_from(v).ok());
                    if let (Some(id), Some(generation)) = (application, generation) {
                        match journeys.process(id, generation).await {
                            Ok(done @ bank_journeys::Progress::Done { .. }) => {
                                operations
                                    .complete(
                                        &lease,
                                        Completion::Done(serde_json::to_value(done).expect(
                                            "Progress contains only serializable primitives",
                                        )),
                                    )
                                    .await
                            }
                            Ok(bank_journeys::Progress::Pending) => {
                                operations.retry(&lease, "CONTROLS_PENDING", 3).await
                            }
                            Ok(bank_journeys::Progress::Review { code }) => {
                                match journeys
                                    .own_exception(id, generation, lease.operation_id)
                                    .await
                                {
                                    Ok(()) => {
                                        operations
                                            .complete(&lease, Completion::Review(code.into()))
                                            .await
                                    }
                                    Err(error) => Err(error),
                                }
                            }
                            Err(_) => {
                                operations
                                    .retry(&lease, "ONBOARDING_RECOVERY_PENDING", 3)
                                    .await
                            }
                        }
                    } else {
                        operations.retry(&lease, "INVALID_JOB_PAYLOAD", 30).await
                    }
                } else {
                    operations.retry(&lease, "HANDLER_NOT_REGISTERED", 30).await
                };
                if result.is_err() {
                    tracing::warn!(operation_id=%lease.operation_id,code="JOB_RECOVERY_PENDING","job awaits lease recovery");
                }
            }
            Ok(None) => {}
            Err(_) => tracing::warn!(code = "JOB_STORE_PENDING", "job store unavailable"),
        }
        tokio::select! { _ = stop.changed() => return, _ = tokio::time::sleep(Duration::from_millis(200)) => {} }
    }
}
async fn control_work(checks: bank_controls::checks::Checks, mut stop: watch::Receiver<bool>) {
    let cases = bank_controls::cases::Cases::new(checks.pool.clone());
    loop {
        if *stop.borrow() {
            return;
        }
        if checks.poll_due().await.is_err() {
            tracing::warn!(
                code = "CONTROL_PENDING",
                "control request awaits safe retry or review"
            );
        }
        if checks.rescreen_due().await.is_err() {
            tracing::warn!(
                code = "RESCREEN_PENDING",
                "expired controls await reassessment"
            );
        }
        if cases.ensure_control_exceptions().await.is_err()
            || cases.escalate_overdue().await.is_err()
        {
            tracing::warn!(
                code = "CASE_MONITOR_PENDING",
                "case assignment or escalation awaits retry"
            );
        }
        tokio::select! {_=stop.changed()=>return,_=tokio::time::sleep(Duration::from_millis(250))=>{}}
    }
}
async fn onboarding_work(
    journeys: std::sync::Arc<bank_journeys::LocalJourneys>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        if *stop.borrow() {
            return;
        }
        if journeys.core.cleanup_history().await.is_err()
            || journeys.recover_contacts().await.is_err()
            || journeys.recover_submissions().await.is_err()
            || journeys.resume_ready().await.is_err()
        {
            tracing::warn!(
                code = "ONBOARDING_SCHEDULE_PENDING",
                "saved onboarding intent awaits recovery"
            );
        }
        tokio::select! {_=stop.changed()=>return,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
}
async fn payment_work(
    payments: std::sync::Arc<bank_payments::Payments>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        if *stop.borrow() {
            return;
        }
        // WP-36: seal queued batch entries into the outbound file once the daily cutoff passes.
        match payments.batch_cutoff_due().await {
            Ok(Some(file)) => {
                tracing::info!(entries = %file["entryCount"], "batch window closed at cutoff")
            }
            Ok(None) => {}
            Err(_) => tracing::warn!(
                code = "BATCH_CUTOFF_PENDING",
                "batch cutoff awaits recovery"
            ),
        }
        // WP-35: release holds for ATM authorizations the network never completed.
        if payments.atm_expire_due().await.is_err() {
            tracing::warn!(
                code = "ATM_EXPIRY_PENDING",
                "ATM authorization expiry awaits recovery"
            );
        }
        if payments.recover().await.is_err() {
            tracing::warn!(
                code = "PAYMENT_SCHEDULE_PENDING",
                "saved payment intent or audit awaits recovery"
            );
        }
        tokio::select! {_=stop.changed()=>return,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
}
async fn finance_work(
    finance: std::sync::Arc<bank_finance::Finance>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        if *stop.borrow() {
            return;
        }
        let consumer_result = match bank_platform::database::Module::Core.connect().await {
            Ok(pool) => bank_core::Core::new(pool).consumer_tick().await.map(|_| ()),
            Err(error) => Err(error),
        };
        if consumer_result.is_err() {
            tracing::warn!(
                code = "CONSUMER_RECOVERY_PENDING",
                "consumer automation awaits native recovery"
            );
        }
        if finance.forecast_tick().await.is_err() {
            tracing::warn!(
                code = "LIQUIDITY_FORECAST_PENDING",
                "scheduled liquidity forecast awaits recovery"
            );
        }
        if finance.recover().await.is_err() {
            tracing::warn!(
                code = "FINANCE_SCHEDULE_PENDING",
                "Finance discovery, scheduling or audit awaits recovery"
            );
        }
        tokio::select! {_=stop.changed()=>return,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
}
/// WP-26 operations loop: transaction monitoring and notifications every 5 s, data-quality scan every 30 s.
/// WP-37: incremental warehouse ingestion every 30 s (rows younger than the 30 s safe lag wait for the next pass).
async fn operations_work(
    monitoring: bank_controls::monitoring::Monitoring,
    alerts: bank_data::alerts::Alerts,
    warehouse: bank_data::analytics::Warehouse,
    mut stop: watch::Receiver<bool>,
) {
    let mut pass: u64 = 0;
    loop {
        if *stop.borrow() {
            return;
        }
        if monitoring.page_tick().await.is_err() {
            tracing::warn!(code = "PAGING_PENDING", "on-call paging awaits recovery");
        }
        if monitoring.tick(500).await.is_err() {
            tracing::warn!(
                code = "MONITORING_PENDING",
                "transaction monitoring awaits recovery"
            );
        }
        if alerts.derive(200).await.is_err() || alerts.deliver(200).await.is_err() {
            tracing::warn!(
                code = "NOTIFICATION_PENDING",
                "notification work awaits recovery"
            );
        }
        if pass.is_multiple_of(6) && alerts.quality_scan().await.is_err() {
            tracing::warn!(
                code = "DATA_QUALITY_PENDING",
                "data-quality scan awaits recovery"
            );
        }
        if pass.is_multiple_of(6) && warehouse.ingest_due(30.0).await.is_err() {
            tracing::warn!(
                code = "ANALYTICS_INGESTION_PENDING",
                "warehouse ingestion awaits recovery"
            );
        }
        pass += 1;
        tokio::select! {_=stop.changed()=>return,_=tokio::time::sleep(Duration::from_secs(5))=>{}}
    }
}
/// WP-25 statement cycle: the latest closed month per account, only once its finance basis is ready.
async fn statement_work(
    records: bank_documents::statements::Records,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        if *stop.borrow() {
            return;
        }
        match records.generate_due(100).await {
            Ok(created) if created > 0 => {
                tracing::info!(created, "period statements published")
            }
            Ok(_) => {}
            Err(_) => tracing::warn!(
                code = "STATEMENT_CYCLE_PENDING",
                "statement cycle awaits recovery"
            ),
        }
        tokio::select! {_=stop.changed()=>return,_=tokio::time::sleep(Duration::from_secs(60))=>{}}
    }
}
/// WP-33 authenticated partner callbacks: drains due deliveries, then polls every 2 seconds.
async fn partner_callback_work(
    partners: bank_identity::consents::Partners,
    mut stop: watch::Receiver<bool>,
) {
    let http = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    loop {
        if *stop.borrow() {
            return;
        }
        loop {
            match partners.deliver_callback(&http).await {
                Ok(true) => continue,
                Ok(false) => break,
                Err(_) => {
                    tracing::warn!(
                        code = "PARTNER_CALLBACK_PENDING",
                        "partner callback delivery awaits recovery"
                    );
                    break;
                }
            }
        }
        tokio::select! {_=stop.changed()=>return,_=tokio::time::sleep(Duration::from_secs(2))=>{}}
    }
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let config = Config::load(4181)?;
    let js = jetstream(&config.nats_url).await?;
    let modules = [
        Module::Platform,
        Module::Identity,
        Module::Core,
        Module::Controls,
        Module::Journeys,
        Module::Payments,
        Module::Finance,
        Module::Documents,
        Module::Data,
        Module::Simulator,
    ];
    let mut stores = Vec::new();
    for module in modules {
        stores.push(EventStore::new(module.connect().await?, module));
    }
    let platform = stores[0].pool.clone();
    let data = stores[8].clone();
    let records = bank_documents::statements::Records::new(stores[7].pool.clone());
    let monitoring = bank_controls::monitoring::Monitoring::new(stores[3].pool.clone());
    let alerts = bank_data::alerts::Alerts::new(stores[8].pool.clone());
    let warehouse = bank_data::analytics::Warehouse::new(stores[8].pool.clone());
    let checks = bank_controls::checks::Checks::new(stores[3].pool.clone())?;
    let journeys = bank_journeys::local().await?;
    let payments = bank_payments::local().await?;
    let finance = bank_finance::local().await?;
    let core_data = std::sync::Arc::new(bank_data::Data::new(
        stores[8].pool.clone(),
        std::sync::Arc::new(journeys.core.clone()),
    ));
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let (stop, receiver) = watch::channel(false);
    let mut tasks = JoinSet::new();
    tasks.spawn(onboarding_work(journeys.clone(), receiver.clone()));
    tasks.spawn(payment_work(payments.clone(), receiver.clone()));
    tasks.spawn(finance_work(finance.clone(), receiver.clone()));
    tasks.spawn(control_work(checks, receiver.clone()));
    tasks.spawn(statement_work(records, receiver.clone()));
    tasks.spawn(partner_callback_work(
        bank_identity::consents::Partners::new(Module::Identity.connect().await?),
        receiver.clone(),
    ));
    tasks.spawn(operations_work(
        monitoring,
        alerts,
        warehouse,
        receiver.clone(),
    ));
    tasks.spawn(relay(stores, js.clone(), receiver.clone()));
    tasks.spawn(work(
        Operations::new(platform),
        payments.clone(),
        finance,
        js.clone(),
        journeys,
        receiver.clone(),
    ));
    let core_js = js.clone();
    let core_stop = receiver.clone();
    tasks.spawn(async move {
        loop {
            if project_core(core_data.clone(), core_js.clone(), core_stop.clone())
                .await
                .is_ok()
                || *core_stop.borrow()
            {
                break;
            }
            tracing::warn!(
                code = "CORE_CONSUMER_RECONNECT",
                "restarting Core projection consumer"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    tasks.spawn(async move {
        loop {
            if project(data.clone(), js.clone(), receiver.clone())
                .await
                .is_ok()
                || *receiver.borrow()
            {
                break;
            }
            tracing::warn!(
                code = "CONSUMER_RECONNECT",
                "restarting durable projection consumer"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    tracing::info!(address=%config.bind,service="bank-worker","starting synthetic local worker");
    axum::serve(listener, health::router(config, "bank-worker"))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = stop.send(true);
        })
        .await?;
    while tasks.join_next().await.is_some() {}
    Ok(())
}
