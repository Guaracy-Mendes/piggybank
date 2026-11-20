use anyhow::Result;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{collections::HashMap, env, sync::OnceLock, time::Duration};
use tokio::sync::Mutex;

pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[derive(Clone, Copy, Debug)]
pub enum Module {
    Platform,
    Identity,
    Core,
    Controls,
    Journeys,
    Payments,
    Finance,
    Documents,
    Data,
    Simulator,
}
impl Module {
    pub fn name(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Identity => "identity",
            Self::Core => "core",
            Self::Controls => "controls",
            Self::Journeys => "journeys",
            Self::Payments => "payments",
            Self::Finance => "finance",
            Self::Documents => "documents",
            Self::Data => "data",
            Self::Simulator => "simulator",
        }
    }
    pub fn local_url(self) -> String {
        let name = self.name();
        env::var(format!("BANK_{}_DATABASE_URL", name.to_ascii_uppercase())).unwrap_or_else(|_| {
            format!("postgres://bank_{name}:synthetic-{name}-only@127.0.0.1:15432/piggybank")
        })
    }
    pub async fn connect(self) -> Result<PgPool> {
        connect_shared(&self.local_url()).await
    }
}

/// Co-located modules share one bounded pool for the exact role/database URL.
/// URL keys are private and never logged. Separate databases and authorities cannot share a pool.
pub async fn connect_shared(url: &str) -> Result<PgPool> {
    static POOLS: OnceLock<Mutex<HashMap<String, PgPool>>> = OnceLock::new();
    let mut pools = POOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .await;
    if let Some(pool) = pools.get(url).filter(|p| !p.is_closed()) {
        return Ok(pool.clone());
    }
    let pool = connect(url).await?;
    pools.insert(url.to_owned(), pool.clone());
    Ok(pool)
}

/// Per-URL pool ceiling. `BANK_DB_POOL_MAX` (1..=32, default 5) must keep the sum across processes and
/// modules below the PostgreSQL `max_connections` budget; pools are lazy, so idle modules hold none.
pub fn pool_max() -> Result<u32> {
    let value = env::var("BANK_DB_POOL_MAX").unwrap_or_else(|_| "5".into());
    let max = value
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("invalid BANK_DB_POOL_MAX"))?;
    anyhow::ensure!(
        (1..=32).contains(&max),
        "BANK_DB_POOL_MAX must be between 1 and 32"
    );
    Ok(max)
}

pub async fn connect(url: &str) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(pool_max()?)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(3))
        .connect(url)
        .await?)
}

pub async fn migrate(url: &str) -> Result<()> {
    let pool = connect(url).await?;
    MIGRATOR.run(&pool).await?;
    pool.close().await;
    Ok(())
}
