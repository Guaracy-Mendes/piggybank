#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("MIGRATION_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://bank_migrator:synthetic-local-only@127.0.0.1:15432/piggybank".into()
    });
    bank_platform::database::migrate(&url).await?;
    println!("Owned schema migrations applied successfully.");
    Ok(())
}
