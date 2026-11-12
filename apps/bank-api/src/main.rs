use bank_platform::{config::Config, health, init_tracing, shutdown_signal};
mod administration;
mod approvals;
mod cases;
mod finance;
mod ingress;
mod journeys;
mod operations;
mod payments;
mod providers;
mod servicing;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let config = Config::load(4180)?;
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
        .merge(payments::router(identity.clone(), payments))
        .merge(finance::router(identity.clone(), finance))
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
