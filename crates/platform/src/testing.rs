//! Explicit disposable PostgreSQL support. Required integration tests never skip.
use crate::database::{MIGRATOR, Module, connect};
use anyhow::Result;
use sqlx::PgPool;

pub struct TestDatabase {
    pub migrator: PgPool,
    admin: PgPool,
    base: url::Url,
    name: String,
}
impl TestDatabase {
    pub async fn new() -> Result<Self> {
        let configured = std::env::var("BANK_TEST_ADMIN_URL").expect(
            "Real PostgreSQL integration needs BANK_TEST_ADMIN_URL; use make test-integration",
        );
        let base = url::Url::parse(&configured)?;
        let admin = connect(&configured).await?;
        let name = format!("piggybank_test_{}", uuid::Uuid::new_v4().simple());
        // Fixed prefix + generated UUID hex: identifier cannot contain user input.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {name} OWNER bank_migrator"
        )))
        .execute(&admin)
        .await?;
        let mut url = base.clone();
        url.set_path(&format!("/{name}"));
        url.set_username("bank_migrator").unwrap();
        url.set_password(Some("synthetic-local-only")).unwrap();
        let migrator = connect(url.as_str()).await?;
        MIGRATOR.run(&migrator).await?;
        Ok(Self {
            migrator,
            admin,
            base,
            name,
        })
    }
    pub fn module_url(&self, module: Module) -> String {
        let mut url = self.base.clone();
        url.set_path(&format!("/{}", self.name));
        url.set_username(&format!("bank_{}", module.name()))
            .unwrap();
        url.set_password(Some(&format!("synthetic-{}-only", module.name())))
            .unwrap();
        url.into()
    }
    pub async fn module(&self, module: Module) -> Result<PgPool> {
        connect(&self.module_url(module)).await
    }
    pub async fn close(self) -> Result<()> {
        self.migrator.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {} WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await?;
        self.admin.close().await;
        Ok(())
    }
}
