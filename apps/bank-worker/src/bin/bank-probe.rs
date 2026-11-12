//! Local diagnostic command; creates no customer or financial objects.
use bank_platform::{
    config::Config,
    database::Module,
    domain::{ActorContext, TrustContext},
    operations::{JobSpec, Operations},
};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = Config::load(4181)?;
    let key = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Usage: bank-probe <idempotency-key>"))?;
    let pool = Module::Platform.connect().await?;
    let actor = ActorContext {
        issuer: "urn:piggybank:local-diagnostics".into(),
        subject: "local-operator".into(),
        trust: TrustContext::Service,
        client_id: "bank-probe".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let result = Operations::new(pool.clone())
        .accept(
            &actor,
            "platform.probe",
            &key,
            &serde_json::json!({"profile":"synthetic-local"}),
            JobSpec {
                kind: "platform.probe".into(),
                step_key: "probe".into(),
                payload: serde_json::json!({}),
                delay_seconds: 0,
            },
        )
        .await?;
    println!("{}", serde_json::to_string(&result)?);
    pool.close().await;
    Ok(())
}
