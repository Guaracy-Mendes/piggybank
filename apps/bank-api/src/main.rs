use bank_platform::{config::Config, health, init_tracing, shutdown_signal};
mod administration;
mod analytics;
mod approvals;
mod assisted;
mod atm;
mod audiences;
mod batch;
mod cases;
mod consumer;
mod consumer_pdf;
mod documents;
mod finance;
mod ingress;
mod journeys;
mod liquidity;
mod metrics;
mod monitoring;
mod operations;
mod partners;
mod payments;
mod periodic_offers;
mod providers;
mod routing;
mod security;
mod servicing;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let config = Config::load(4180)?;
    validate_secret_files()?;
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address = %config.bind, service = "bank-api", "starting synthetic local runtime");
    let identity = bank_identity::IdentityService::new().await?;
    let controls = bank_controls::entitlements::Entitlements::new(
        bank_platform::database::Module::Controls.connect().await?,
    );
    let gateway = ingress::Ingress::new().await?;
    let operations = bank_platform::operations::Operations::new(
        bank_platform::database::Module::Platform.connect().await?,
    );
    let checks = bank_controls::checks::Checks::new(
        bank_platform::database::Module::Controls.connect().await?,
    )?;
    let journeys = bank_journeys::local().await?;
    let payments = bank_payments::local().await?;
    let finance = bank_finance::local().await?;
    let app = health::router(config, "bank-api")
        .merge(bank_identity::router(identity.clone()))
        .merge(consumer::router(
            identity.clone(),
            journeys.core.clone(),
            controls.clone(),
        ))
        .merge(documents::router(
            identity.clone(),
            bank_documents::statements::Records::new(
                bank_platform::database::Module::Documents.connect().await?,
            ),
            controls.clone(),
        ))
        .merge(monitoring::router(
            identity.clone(),
            bank_controls::monitoring::Monitoring::new(
                bank_platform::database::Module::Controls.connect().await?,
            ),
            bank_data::alerts::Alerts::new(bank_platform::database::Module::Data.connect().await?),
            controls.clone(),
        ))
        .merge(metrics::router(bank_controls::monitoring::Monitoring::new(
            bank_platform::database::Module::Controls.connect().await?,
        )))
        .merge(security::router(
            identity.clone(),
            bank_controls::security::Security::new(
                bank_platform::database::Module::Controls.connect().await?,
            ),
            controls.clone(),
        ))
        .merge(assisted::router(
            identity.clone(),
            bank_controls::assisted::Assisted::new(
                bank_platform::database::Module::Controls.connect().await?,
            ),
            journeys.core.clone(),
            controls.clone(),
        ))
        .merge(partners::router(
            identity.clone(),
            bank_identity::consents::Partners::new(
                bank_platform::database::Module::Identity.connect().await?,
            ),
            journeys.core.clone(),
            controls.clone(),
            payments.clone(),
        ))
        .merge(analytics::router(
            identity.clone(),
            bank_data::analytics::Warehouse::new(
                bank_platform::database::Module::Data.connect().await?,
            ),
            controls.clone(),
        ))
        .merge(audiences::router(
            identity.clone(),
            bank_data::audience::Audiences::new(
                bank_platform::database::Module::Data.connect().await?,
            ),
            bank_data::insights::Insights::new(
                bank_platform::database::Module::Data.connect().await?,
            ),
            controls.clone(),
        ))
        .merge(atm::router(identity.clone(), payments.clone()))
        .merge(batch::router(identity.clone(), payments.clone()))
        .merge(routing::router(identity.clone(), payments.clone()))
        .merge(payments::router(identity.clone(), payments))
        .merge(liquidity::router(identity.clone(), finance.clone()))
        .merge(finance::router(identity.clone(), finance))
        .merge(periodic_offers::router(
            identity.clone(),
            journeys.core.clone(),
            journeys.terms.clone(),
            controls.clone(),
        ))
        .merge(journeys::router(identity.clone(), journeys.clone()))
        .merge(administration::router(
            identity.clone(),
            controls.clone(),
            journeys.core.clone(),
            journeys.terms.clone(),
        ))
        .merge(servicing::router(identity.clone(), journeys))
        .merge(approvals::router(identity.clone(), controls))
        .merge(cases::router(
            identity.clone(),
            bank_controls::cases::Cases::new(checks.pool.clone()),
        )?)
        .merge(operations::router(identity, operations))
        .merge(providers::router(checks))
        .route(
            "/openapi.json",
            axum::routing::get(|| async {
                axum::Json(
                    serde_json::from_str::<serde_json::Value>(include_str!(
                        "../../../contracts/openapi/implemented.json"
                    ))
                    .expect("validated OpenAPI"),
                )
            }),
        )
        .fallback(|| async { axum::http::StatusCode::NOT_FOUND })
        .method_not_allowed_fallback(|| async { axum::http::StatusCode::METHOD_NOT_ALLOWED })
        .layer(axum::middleware::from_fn_with_state(
            gateway,
            ingress::boundary,
        ));
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

/// WP-29 protected runtime configuration: a configured secret file that is unreadable or malformed stops
/// startup instead of failing later inside a request. Values are never logged.
fn validate_secret_files() -> anyhow::Result<()> {
    if std::env::var("BANK_SESSION_KEYS_FILE").is_ok() {
        bank_identity::seal::KeyRing::load()
            .map_err(|e| anyhow::anyhow!("BANK_SESSION_KEYS_FILE: {e}"))?;
    }
    for (name, minimum) in [
        ("BANK_IDENTITY_ADMIN_SECRET_FILE", 16),
        ("BANK_METRICS_TOKEN_FILE", 32),
    ] {
        if let Ok(path) = std::env::var(name) {
            let value = std::fs::read_to_string(&path)
                .map_err(|_| anyhow::anyhow!("{name} is unreadable"))?;
            anyhow::ensure!(
                value.trim().len() >= minimum,
                "{name} must hold at least {minimum} characters"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path)?.permissions().mode() & 0o077;
                anyhow::ensure!(mode == 0, "{name} must not be readable by group or others");
            }
        }
    }
    Ok(())
}
