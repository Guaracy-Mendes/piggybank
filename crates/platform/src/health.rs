use crate::config::Config;
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use std::{collections::BTreeMap, sync::Arc};
use tokio::time::timeout;

#[derive(Serialize)]
struct Health {
    status: &'static str,
    service: &'static str,
    dependencies: BTreeMap<&'static str, bool>,
}

pub fn router(config: Config, service: &'static str) -> Router {
    Router::new()
        .route(
            "/health/live",
            get(move || async move {
                Json(Health {
                    status: "alive",
                    service,
                    dependencies: BTreeMap::new(),
                })
            }),
        )
        .route("/health/ready", get(ready))
        .with_state(Arc::new((config, service)))
}

async fn ready(State(state): State<Arc<(Config, &'static str)>>) -> (StatusCode, Json<Health>) {
    let config = &state.0;
    let deadline = config.dependency_timeout;
    let database = async {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(deadline)
            .connect(&config.database_url)
            .await?;
        let expected = crate::database::MIGRATOR
            .iter()
            .map(|m| m.version)
            .max()
            .unwrap_or(0);
        let result =
            sqlx::query_scalar::<_, i64>("SELECT current_version FROM platform.schema_version")
                .fetch_one(&pool)
                .await;
        pool.close().await;
        result.and_then(|version| {
            if version == expected {
                Ok(version)
            } else {
                Err(sqlx::Error::Protocol("schema version mismatch".into()))
            }
        })
    };
    let broker = async {
        let client = async_nats::connect(&config.nats_url).await?;
        client.flush().await?;
        async_nats::jetstream::new(client).query_account().await?;
        Ok::<_, anyhow::Error>(())
    };
    let issuer = async {
        reqwest::Client::new()
            .get(format!(
                "{}/.well-known/openid-configuration",
                config.issuer_url
            ))
            .send()
            .await?
            .error_for_status()
    };
    let (database, broker, issuer) = tokio::join!(
        timeout(deadline, database),
        timeout(deadline, broker),
        timeout(deadline, issuer)
    );
    let dependencies = BTreeMap::from([
        ("postgres", matches!(database, Ok(Ok(_)))),
        ("jetstream", matches!(broker, Ok(Ok(_)))),
        ("oidc", matches!(issuer, Ok(Ok(_)))),
    ]);
    let healthy = dependencies.values().all(|value| *value);
    (
        if healthy {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(Health {
            status: if healthy { "ready" } else { "degraded" },
            service: state.1,
            dependencies,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use std::time::{Duration, Instant};
    use tower::ServiceExt;

    #[tokio::test]
    async fn readiness_fails_within_deadline_without_killing_liveness() {
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            database_url: "postgres://nobody:invalid@127.0.0.1:1/absent".into(),
            nats_url: "nats://127.0.0.1:1".into(),
            issuer_url: "http://127.0.0.1:1".into(),
            dependency_timeout: Duration::from_millis(100),
        };
        let app = router(config, "test");
        let started = Instant::now();
        let ready = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(started.elapsed() < Duration::from_secs(2));
        let live = app
            .oneshot(
                Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(live.status(), StatusCode::OK);
    }
}
