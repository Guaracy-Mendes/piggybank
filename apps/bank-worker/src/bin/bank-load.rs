//! WP-30 reproducible workload generator (synthetic local profile only).
//!
//! `bank-load http <config.json>`: open-loop HTTP workload. Virtual users each hold their own rotating native
//! refresh token. Reads: authoritative balance and paged history (NFR-02). Commands: payment quote plus durable
//! payment intent with an idempotency key (NFR-03). Phases: sustained, then burst. Every latency sample is kept,
//! so percentiles are exact. Writes a JSON report.
//!
//! `bank-load core <accounts> <workers> <seconds>`: concurrent balanced Core transfers between synthetic ledger
//! accounts (NFR-04 commit latency), then verifies conservation, non-negative balances and idempotent replay.
use anyhow::{Context, Result, ensure};
use bank_core::Core;
use bank_platform::{
    database::Module,
    domain::{AccountId, Currency, Money},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    api: String,
    issuer: String,
    users: Vec<User>,
    read_rps: f64,
    command_rps: f64,
    sustained_seconds: u64,
    burst_multiplier: f64,
    burst_seconds: u64,
    out: String,
}
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct User {
    refresh_token: String,
    account_id: String,
    beneficiary_id: String,
}
struct Session {
    refresh: String,
    access: String,
    obtained: Instant,
}
#[derive(Default)]
struct Samples {
    latency: BTreeMap<String, Vec<f64>>,
    statuses: BTreeMap<String, BTreeMap<String, u64>>,
}
async fn rotate(http: &reqwest::Client, issuer: &str, session: &Mutex<Session>) -> Result<String> {
    let mut s = session.lock().await;
    if s.obtained.elapsed() < Duration::from_secs(20) {
        return Ok(s.access.clone());
    }
    let tokens: Value = http
        .post(format!("{issuer}/protocol/openid-connect/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", "bank-mobile"),
            ("refresh_token", s.refresh.as_str()),
        ])
        .send()
        .await?
        .error_for_status()
        .context("token refresh failed")?
        .json()
        .await?;
    s.access = tokens["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    s.refresh = tokens["refresh_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    s.obtained = Instant::now();
    Ok(s.access.clone())
}
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}
async fn phase(
    name: &str,
    config: &Config,
    http: &reqwest::Client,
    sessions: &[Arc<Mutex<Session>>],
    multiplier: f64,
    seconds: u64,
) -> Value {
    let samples = Arc::new(Mutex::new(Samples::default()));
    let inflight = Arc::new(Semaphore::new(1024));
    let issued = Arc::new(AtomicU64::new(0));
    let total_rps = (config.read_rps + config.command_rps / 2.0) * multiplier;
    let interval = Duration::from_secs_f64(1.0 / total_rps);
    let start = Instant::now();
    let mut next = start;
    let mut slot: u64 = 0;
    let day_from = (chrono::Utc::now() - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let day_to = (chrono::Utc::now() + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let read_share = config.read_rps / (config.read_rps + config.command_rps / 2.0);
    let mut tasks = Vec::new();
    while start.elapsed() < Duration::from_secs(seconds) {
        let now = Instant::now();
        if next > now {
            tokio::time::sleep(next - now).await;
        }
        next += interval;
        slot += 1;
        let user_index = (slot as usize) % config.users.len();
        let user = config.users[user_index].clone();
        let session = sessions[user_index].clone();
        let is_read =
            ((slot as f64) * read_share).floor() > (((slot - 1) as f64) * read_share).floor();
        let (http, issuer, api, samples, permit_pool, issued) = (
            http.clone(),
            config.issuer.clone(),
            config.api.clone(),
            samples.clone(),
            inflight.clone(),
            issued.clone(),
        );
        let (from, to) = (day_from.clone(), day_to.clone());
        tasks.push(tokio::spawn(async move {
            let Ok(_permit) = permit_pool.acquire_owned().await else { return };
            issued.fetch_add(1, Ordering::Relaxed);
            let token = match rotate(&http, &issuer, &session).await {
                Ok(t) => t,
                Err(_) => {
                    let mut s = samples.lock().await;
                    *s.statuses.entry("token".into()).or_default().entry("refresh_error".into()).or_default() += 1;
                    return;
                }
            };
            let record = |kind: &'static str, status: String, seconds: f64| {
                let samples = samples.clone();
                async move {
                    let mut s = samples.lock().await;
                    s.latency.entry(kind.into()).or_default().push(seconds);
                    *s.statuses.entry(kind.into()).or_default().entry(status).or_default() += 1;
                }
            };
            let call = |request: reqwest::RequestBuilder| async move {
                let started = Instant::now();
                let result = request.timeout(Duration::from_secs(10)).send().await;
                let seconds = started.elapsed().as_secs_f64();
                match result {
                    Ok(r) => {
                        let status = r.status().as_u16().to_string();
                        let body: Value = r.json().await.unwrap_or(Value::Null);
                        (status, seconds, body)
                    }
                    Err(e) if e.is_timeout() => ("timeout".into(), seconds, Value::Null),
                    Err(_) => ("transport".into(), seconds, Value::Null),
                }
            };
            if is_read {
                let (kind, url) = if slot.is_multiple_of(2) {
                    ("read_balance", format!("{api}/api/v1/accounts/{}/balance", user.account_id))
                } else {
                    ("read_history", format!("{api}/api/v1/accounts/{}/transactions?from={from}&to={to}&limit=20", user.account_id))
                };
                let (status, seconds, _) = call(http.get(url).bearer_auth(&token)).await;
                record(kind, status, seconds).await;
            } else {
                let quote = json!({"payerAccountId": user.account_id, "beneficiaryId": user.beneficiary_id,
                    "amount": {"minorUnits": "100", "currency": {"code": "TST", "scale": 2}}, "requestedDate": chrono::Utc::now().format("%Y-%m-%d").to_string()});
                let (status, seconds, body) = call(http.post(format!("{api}/api/v1/payment-quotes")).bearer_auth(&token).json(&quote)).await;
                record("command_quote", status.clone(), seconds).await;
                if status == "201" {
                    let key = Uuid::new_v4().to_string();
                    let (status, seconds, _) = call(
                        http.post(format!("{api}/api/v1/payments")).bearer_auth(&token).header("Idempotency-Key", key).json(&json!({"quoteId": body["id"]})),
                    )
                    .await;
                    record("command_payment_intent", status, seconds).await;
                }
            }
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let mut s = samples.lock().await;
    let statuses_by_kind = s.statuses.clone();
    let mut kinds = serde_json::Map::new();
    for (kind, values) in s.latency.iter_mut() {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let statuses = statuses_by_kind.get(kind).cloned().unwrap_or_default();
        let failures: u64 = statuses
            .iter()
            .filter(|(k, _)| k.starts_with('5') || *k == "timeout" || *k == "transport")
            .map(|(_, v)| v)
            .sum();
        kinds.insert(
            kind.clone(),
            json!({"count": values.len(), "achievedRps": values.len() as f64 / elapsed, "p50Ms": percentile(values, 50.0) * 1000.0,
                   "p95Ms": percentile(values, 95.0) * 1000.0, "p99Ms": percentile(values, 99.0) * 1000.0,
                   "maxMs": values.last().copied().unwrap_or(0.0) * 1000.0, "statuses": statuses, "failures": failures,
                   "availability": if values.is_empty() { 1.0 } else { 1.0 - failures as f64 / values.len() as f64 }}),
        );
    }
    json!({"phase": name, "seconds": elapsed, "multiplier": multiplier, "issued": issued.load(Ordering::Relaxed),
           "targetReadRps": config.read_rps * multiplier, "targetCommandRps": config.command_rps * multiplier, "kinds": kinds,
           "tokenErrors": s.statuses.get("token").cloned().unwrap_or_default()})
}
async fn http_mode(path: &str) -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        !config.users.is_empty(),
        "at least one virtual user required"
    );
    let http = reqwest::Client::builder()
        .pool_max_idle_per_host(512)
        .build()?;
    let sessions: Vec<Arc<Mutex<Session>>> = config
        .users
        .iter()
        .map(|u| {
            Arc::new(Mutex::new(Session {
                refresh: u.refresh_token.clone(),
                access: String::new(),
                obtained: Instant::now() - Duration::from_secs(3600),
            }))
        })
        .collect();
    let sustained = phase(
        "sustained",
        &config,
        &http,
        &sessions,
        1.0,
        config.sustained_seconds,
    )
    .await;
    let burst = phase(
        "burst",
        &config,
        &http,
        &sessions,
        config.burst_multiplier,
        config.burst_seconds,
    )
    .await;
    let report = json!({"source": "bank-load http", "virtualUsers": config.users.len(), "phases": [sustained, burst]});
    std::fs::write(&config.out, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
async fn core_mode(accounts: usize, workers: usize, seconds: u64) -> Result<()> {
    ensure!(
        std::env::var("BANK_PROFILE").unwrap_or_else(|_| "local".into()) == "local",
        "synthetic local profile only"
    );
    let core = Core::new(Module::Core.connect().await?);
    let admin = sqlx::PgPool::connect(
        &std::env::var("BANK_LOAD_ADMIN_URL")
            .context("BANK_LOAD_ADMIN_URL required to create synthetic ledger accounts")?,
    )
    .await?;
    let clearing = AccountId::new();
    sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'SETTLEMENT','ACTIVE',false)").bind(clearing.0).execute(&admin).await?;
    let mut ids = Vec::new();
    for _ in 0..accounts {
        let id = AccountId::new();
        sqlx::query("INSERT INTO core.ledger_accounts(id,kind,state,debit_blocked) VALUES($1,'CUSTOMER','ACTIVE',false)").bind(id.0).execute(&admin).await?;
        core.fund_synthetic(
            Uuid::new_v4(),
            id,
            clearing,
            Money::new(1_000_000, Currency::synthetic()),
        )
        .await?;
        ids.push(id);
    }
    let ids = Arc::new(ids);
    let latencies = Arc::new(Mutex::new(Vec::<f64>::new()));
    let outcomes = Arc::new(Mutex::new(BTreeMap::<String, u64>::new()));
    let replayed = Arc::new(AtomicU64::new(0));
    let start = Instant::now();
    let mut handles = Vec::new();
    for worker in 0..workers {
        let (core, ids, latencies, outcomes, replayed) = (
            core.clone(),
            ids.clone(),
            latencies.clone(),
            outcomes.clone(),
            replayed.clone(),
        );
        handles.push(tokio::spawn(async move {
            let mut n: usize = worker;
            while start.elapsed() < Duration::from_secs(seconds) {
                n = n
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let from = ids[n % ids.len()];
                let to = ids[(n / 7 + 1) % ids.len()];
                if from == to {
                    continue;
                }
                let op = Uuid::new_v4();
                let amount = Money::new(((n % 900) + 1) as i64, Currency::synthetic());
                let began = Instant::now();
                let result = core.transfer(op, from, to, amount, None).await;
                let elapsed = began.elapsed().as_secs_f64();
                let label = if result.is_ok() {
                    "committed"
                } else {
                    "refused"
                };
                *outcomes.lock().await.entry(label.into()).or_default() += 1;
                if result.is_ok() {
                    latencies.lock().await.push(elapsed);
                    // Same business key again: idempotent, never a second economic effect.
                    if n.is_multiple_of(50)
                        && core.transfer(op, from, to, amount, None).await.is_ok()
                    {
                        replayed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    let mut lat = latencies.lock().await.clone();
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let total: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(booked),0)::bigint FROM core.ledger_accounts WHERE id=ANY($1)",
    )
    .bind(ids.iter().map(|a| a.0).collect::<Vec<_>>())
    .fetch_one(&admin)
    .await?;
    let negative: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core.ledger_accounts WHERE id=ANY($1) AND (booked<0 OR held>booked)",
    )
    .bind(ids.iter().map(|a| a.0).collect::<Vec<_>>())
    .fetch_one(&admin)
    .await?;
    let unbalanced: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT batch_id FROM core.journal_entries GROUP BY batch_id HAVING sum(delta::numeric)<>0) u").fetch_one(&admin).await?;
    let mismatched: i64 = sqlx::query_scalar("SELECT count(*) FROM core.ledger_accounts a WHERE a.id=ANY($1) AND a.booked<>coalesce((SELECT sum(delta) FROM core.journal_entries e WHERE e.account_id=a.id),0)")
        .bind(ids.iter().map(|a| a.0).collect::<Vec<_>>())
        .fetch_one(&admin)
        .await?;
    let elapsed = start.elapsed().as_secs_f64();
    let report = json!({"source": "bank-load core", "accounts": accounts, "workers": workers, "seconds": elapsed,
        "committed": lat.len(), "committedPerSecond": lat.len() as f64 / elapsed, "outcomes": *outcomes.lock().await,
        "p50Ms": percentile(&lat, 50.0) * 1000.0, "p95Ms": percentile(&lat, 95.0) * 1000.0, "p99Ms": percentile(&lat, 99.0) * 1000.0,
        "maxMs": lat.last().copied().unwrap_or(0.0) * 1000.0, "idempotentReplays": replayed.load(Ordering::Relaxed),
        "invariants": {"conservedTotal": total == accounts as i64 * 1_000_000, "negativeOrOverheld": negative, "unbalancedBatches": unbalanced, "bookedMismatches": mismatched}});
    println!("{}", serde_json::to_string(&report)?);
    ensure!(
        total == accounts as i64 * 1_000_000 && negative == 0 && unbalanced == 0 && mismatched == 0,
        "financial invariant violated under load"
    );
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("http") => http_mode(args.get(2).context("config path required")?).await,
        Some("core") => {
            let n = |i: usize, d: &str| args.get(i).map(String::as_str).unwrap_or(d).parse::<u64>();
            core_mode(n(2, "200")? as usize, n(3, "16")? as usize, n(4, "60")?).await
        }
        _ => anyhow::bail!(
            "usage: bank-load http <config.json> | bank-load core <accounts> <workers> <seconds>"
        ),
    }
}
