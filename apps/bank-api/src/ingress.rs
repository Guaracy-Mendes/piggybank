use anyhow::{Context, Result};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sqlx::PgPool;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use uuid::Uuid;

pub struct Ingress {
    pool: PgPool,
    limit: i32,
    scope: String,
}
impl Ingress {
    pub async fn new() -> Result<Arc<Self>> {
        let limit = std::env::var("BANK_RATE_LIMIT")
            .unwrap_or_else(|_| "500".into())
            .parse::<i32>()
            .context("invalid BANK_RATE_LIMIT")?;
        anyhow::ensure!(
            (1..=1_000_000).contains(&limit),
            "BANK_RATE_LIMIT must be between 1 and 1000000"
        );
        let scope = std::env::var("BANK_RATE_SCOPE").unwrap_or_else(|_| "bank-api".into());
        Ok(Arc::new(Self {
            pool: bank_platform::database::Module::Platform.connect().await?,
            limit,
            scope,
        }))
    }
    /// Quota counters are advisory: a crash may lose the last few increments of the current minute, which only
    /// under-counts. Every request from one address updates the same row, so the row lock must be held as briefly as
    /// possible (WP-30 CF-02). That means one statement and one round trip, with the commit not waiting for a WAL
    /// flush. `set_config(..., true)` scopes the setting to this statement's implicit transaction.
    async fn permitted(&self, ip: &str) -> Result<bool> {
        let result=sqlx::query_scalar::<_,i32>("WITH relaxed AS (SELECT set_config('synchronous_commit','off',true)) INSERT INTO platform.rate_limits(scope,window_start,requests) SELECT $1,date_trunc('minute',now()),1 FROM relaxed ON CONFLICT(scope,window_start) DO UPDATE SET requests=platform.rate_limits.requests+1 WHERE platform.rate_limits.requests<$2 RETURNING requests")
            .bind(format!("{}:{ip}",self.scope)).bind(self.limit).fetch_optional(&self.pool).await?;
        Ok(result.is_some())
    }
}

pub fn error(status: StatusCode, id: Uuid) -> Response {
    let (code, message) = match status {
        StatusCode::BAD_REQUEST => ("INVALID_REQUEST", "The request is invalid."),
        StatusCode::UNAUTHORIZED => ("UNAUTHENTICATED", "Authentication is required."),
        StatusCode::FORBIDDEN => ("FORBIDDEN", "This action is not permitted."),
        StatusCode::NOT_FOUND => ("NOT_FOUND", "The requested resource is unavailable."),
        StatusCode::METHOD_NOT_ALLOWED => ("METHOD_NOT_ALLOWED", "The method is not supported."),
        StatusCode::CONFLICT => (
            "CONFLICT",
            "The request conflicts with the current operation or version.",
        ),
        StatusCode::PAYLOAD_TOO_LARGE => {
            ("PAYLOAD_TOO_LARGE", "The request exceeds the size limit.")
        }
        StatusCode::UNPROCESSABLE_ENTITY => ("INVALID_FIELDS", "The request fields are invalid."),
        StatusCode::TOO_MANY_REQUESTS => ("RATE_LIMITED", "The request limit has been reached."),
        StatusCode::GONE => ("VERSION_SUNSET", "This API version is no longer supported."),
        StatusCode::GATEWAY_TIMEOUT => (
            "TIMEOUT",
            "The request outcome is not available. Check the existing operation before retrying.",
        ),
        _ => ("UNAVAILABLE", "The service is temporarily unavailable."),
    };
    let mut response = (
        status,
        Json(serde_json::json!({"error":{"code":code,"message":message},"requestId":id})),
    )
        .into_response();
    if status == StatusCode::TOO_MANY_REQUESTS {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    }
    response
}

pub async fn boundary(State(state): State<Arc<Ingress>>, request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get("x-request-id")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4);
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = std::time::Instant::now();
    let exempt = request.uri().path().starts_with("/health/");
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|s| s.0.ip().to_string())
        .unwrap_or_else(|| "internal".into());
    let mut response = async {
        if !exempt {
            match state.permitted(&ip).await {
                Ok(true) => {}
                Ok(false) => return error(StatusCode::TOO_MANY_REQUESTS, id),
                Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, id),
            }
        }
        if request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|length| length > 65536)
        {
            return error(StatusCode::PAYLOAD_TOO_LARGE, id);
        }
        let (mut parts, body) = request.into_parts();
        parts.extensions.insert(id);
        let bytes = match tokio::time::timeout(Duration::from_secs(3), to_bytes(body, 65536)).await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, id),
            Err(_) => return error(StatusCode::GATEWAY_TIMEOUT, id),
        };
        let request = Request::from_parts(parts, Body::from(bytes));
        let response = match tokio::time::timeout(Duration::from_secs(5), next.run(request)).await {
            Ok(value) => value,
            Err(_) => return error(StatusCode::GATEWAY_TIMEOUT, id),
        };
        // Preserve diagnostic readiness JSON; all other errors use a safe envelope.
        if !exempt && (response.status().is_client_error() || response.status().is_server_error()) {
            // Protocol headers a client needs to act on are kept; everything else is replaced (WP-33 adds
            // Sunset and Deprecation for versioned partner contracts).
            let mut safe = error(response.status(), id);
            for name in [header::ALLOW, header::RETRY_AFTER] {
                if let Some(value) = response.headers().get(&name).cloned() {
                    safe.headers_mut().insert(name, value);
                }
            }
            for name in ["sunset", "deprecation"] {
                if let Some(value) = response.headers().get(name).cloned() {
                    safe.headers_mut().insert(name, value);
                }
            }
            safe
        } else {
            response
        }
    }
    .await;
    let headers = response.headers_mut();
    headers.insert(
        "x-request-id",
        HeaderValue::from_str(&id.to_string()).expect("UUID header"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    crate::metrics::record(
        method.as_str(),
        &path,
        response.status().as_u16(),
        started.elapsed(),
    );
    tracing::info!(%id,%method,status=response.status().as_u16(),"request completed");
    response
}
