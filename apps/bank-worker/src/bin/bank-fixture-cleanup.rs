//! Drop only a generated HTTP test stream after all fixture processes stop.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = bank_platform::config::Config::load(4181)?;
    let suffix = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("HTTP test UUID required"))?;
    anyhow::ensure!(
        suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit()),
        "Only generated test streams can be removed"
    );
    uuid::Uuid::parse_str(&suffix)?;
    let js = async_nats::jetstream::new(async_nats::connect("nats://127.0.0.1:14222").await?);
    let name = format!("BANK_HTTP_TEST_{suffix}");
    match js.get_stream(&name).await {
        Ok(_) => {
            js.delete_stream(name).await?;
        }
        Err(error) => {
            let absent = matches!(error.kind(),async_nats::jetstream::context::GetStreamErrorKind::JetStream(details) if details.error_code()==async_nats::jetstream::ErrorCode::STREAM_NOT_FOUND);
            anyhow::ensure!(absent, "Fixture stream inspection failed");
        }
    }
    Ok(())
}
