//! WP-29 Prometheus-format metrics: request counts and latency histograms per method/route template/status
//! class, plus queue, financial-invariant and alert gauges from Controls. The endpoint is disabled unless
//! `BANK_METRICS_TOKEN_FILE` is set, and then requires that bearer token (re-read on use, never logged).
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bank_controls::monitoring::Monitoring;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fmt::Write,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use subtle::ConstantTimeEq;

const BUCKETS: [f64; 10] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];
#[derive(Default)]
struct Stat {
    count: u64,
    sum: f64,
    buckets: [u64; BUCKETS.len()],
}
type Key = (String, String, String);
type Observation = (u64, f64, [u64; BUCKETS.len()]);
fn registry() -> &'static Mutex<BTreeMap<Key, Stat>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<Key, Stat>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
/// Collapses identifiers so route cardinality stays bounded.
pub fn template(path: &str) -> String {
    if !(path.starts_with("/api/")
        || path.starts_with("/auth/")
        || path.starts_with("/health/")
        || path == "/metrics"
        || path == "/openapi.json")
    {
        return "other".into();
    }
    path.split('/')
        .map(|segment| {
            let hexish =
                segment.len() >= 16 && segment.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
            if hexish || (!segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit())) {
                "{id}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}
pub fn record(method: &str, path: &str, status: u16, elapsed: Duration) {
    let key = (
        method.to_owned(),
        template(path),
        format!("{}xx", status / 100),
    );
    let seconds = elapsed.as_secs_f64();
    if let Ok(mut map) = registry().lock() {
        let stat = map.entry(key).or_default();
        stat.count += 1;
        stat.sum += seconds;
        for (i, le) in BUCKETS.iter().enumerate() {
            if seconds <= *le {
                stat.buckets[i] += 1;
            }
        }
    }
}
fn token() -> Option<String> {
    let path = std::env::var("BANK_METRICS_TOKEN_FILE").ok()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| t.len() >= 32)
}
pub fn router(monitoring: Monitoring) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .with_state(monitoring)
}
fn label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
async fn metrics(
    State(monitoring): State<Monitoring>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let expected = token().ok_or(StatusCode::NOT_FOUND)?;
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !bool::from(presented.as_bytes().ct_eq(expected.as_bytes())) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# HELP bank_http_requests_total Completed HTTP requests by route template and status class.\n# TYPE bank_http_requests_total counter"
    );
    let snapshot: Vec<(Key, Observation)> = registry()
        .lock()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), (v.count, v.sum, v.buckets)))
                .collect()
        })
        .unwrap_or_default();
    for ((method, route, status), (count, _, _)) in &snapshot {
        let _ = writeln!(
            out,
            "bank_http_requests_total{{method=\"{}\",route=\"{}\",status=\"{}\"}} {count}",
            label(method),
            label(route),
            label(status)
        );
    }
    let _ = writeln!(
        out,
        "# HELP bank_http_request_duration_seconds Request latency.\n# TYPE bank_http_request_duration_seconds histogram"
    );
    for ((method, route, status), (count, sum, buckets)) in &snapshot {
        let l = format!(
            "method=\"{}\",route=\"{}\",status=\"{}\"",
            label(method),
            label(route),
            label(status)
        );
        for (i, le) in BUCKETS.iter().enumerate() {
            let _ = writeln!(
                out,
                "bank_http_request_duration_seconds_bucket{{{l},le=\"{le}\"}} {}",
                buckets[i]
            );
        }
        let _ = writeln!(
            out,
            "bank_http_request_duration_seconds_bucket{{{l},le=\"+Inf\"}} {count}\nbank_http_request_duration_seconds_sum{{{l}}} {sum}\nbank_http_request_duration_seconds_count{{{l}}} {count}"
        );
    }
    let gauges: Value = monitoring
        .metrics_snapshot()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    for (name, field) in [
        ("outbox_undelivered", "undelivered"),
        ("outbox_oldest_age_seconds", "oldestAgeSeconds"),
    ] {
        let _ = writeln!(out, "# TYPE bank_{name} gauge");
        if let Some(modules) = gauges["outbox"].as_object() {
            for (module, v) in modules {
                let _ = writeln!(
                    out,
                    "bank_{name}{{module=\"{}\"}} {}",
                    label(module),
                    v[field].as_f64().unwrap_or(0.0)
                );
            }
        }
    }
    let _ = writeln!(out, "# TYPE bank_dead_letters_pending gauge");
    if let Some(modules) = gauges["deadLetters"].as_object() {
        for (module, v) in modules {
            let _ = writeln!(
                out,
                "bank_dead_letters_pending{{module=\"{}\"}} {}",
                label(module),
                v.as_f64().unwrap_or(0.0)
            );
        }
    }
    for (metric, field) in [
        ("payments_awaiting", "paymentsAwaiting"),
        ("cases_open", "casesOpen"),
        ("cases_overdue", "casesOverdue"),
        ("quality_alerts_open", "qualityAlertsOpen"),
        ("monitoring_alerts_open", "monitoringAlertsOpen"),
        ("notifications_pending", "notificationsPending"),
        ("notifications_failed", "notificationsFailed"),
        ("projection_lag_seconds", "projectionLagSeconds"),
        ("pages_unacknowledged", "pagesUnacknowledged"),
        ("pages_undelivered", "pagesUndelivered"),
        (
            "oldest_unacknowledged_page_seconds",
            "oldestUnacknowledgedPageSeconds",
        ),
        ("journal_batches", "journalBatches"),
        ("unbalanced_journal_batches", "unbalancedBatches"),
    ] {
        let _ = writeln!(
            out,
            "# TYPE bank_{metric} gauge\nbank_{metric} {}",
            gauges[field].as_f64().unwrap_or(0.0)
        );
    }
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], out).into_response())
}
#[cfg(test)]
mod tests {
    #[test]
    fn templates_collapse_identifiers() {
        assert_eq!(
            super::template("/api/v1/payments/6f1c0c1e-5b9a-4c1e-9d2a-1a2b3c4d5e6f"),
            "/api/v1/payments/{id}"
        );
        assert_eq!(
            super::template("/api/v1/documents/downloads/0123456789abcdef0123456789abcdef"),
            "/api/v1/documents/downloads/{id}"
        );
        assert_eq!(super::template("/assets/index-3f2a.js"), "other");
    }
}
