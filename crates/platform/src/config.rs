use anyhow::{Context, Result, bail};
use std::{env, net::SocketAddr, time::Duration};

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    pub nats_url: String,
    pub issuer_url: String,
    pub dependency_timeout: Duration,
}

impl Config {
    pub fn load(default_port: u16) -> Result<Self> {
        let profile = env::var("BANK_PROFILE").unwrap_or_else(|_| "local".into());
        if profile != "local" {
            bail!(
                "Only the synthetic local profile is implemented; production requires qualified adapters and policies"
            );
        }
        Ok(Self {
            bind: env::var("BANK_BIND")
                .unwrap_or_else(|_| format!("127.0.0.1:{default_port}"))
                .parse()
                .context("invalid BANK_BIND")?,
            database_url: env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://bank_platform:synthetic-platform-only@127.0.0.1:15432/piggybank".into()
            }),
            nats_url: env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:14222".into()),
            issuer_url: env::var("OIDC_ISSUER")
                .unwrap_or_else(|_| "http://127.0.0.1:18280/realms/customers".into()),
            dependency_timeout: Duration::from_millis(1500),
        })
    }
}
