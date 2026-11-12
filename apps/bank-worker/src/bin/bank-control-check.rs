//! Explicitly synthetic operator fixture command; does not approve onboarding or activate accounts.
use bank_controls::checks::{CheckKind, Checks};
use bank_platform::{
    config::Config,
    database::Module,
    domain::{ActorContext, TrustContext, fingerprint},
};
use uuid::Uuid;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = Config::load(4181)?;
    let resource = std::env::args()
        .nth(1)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Usage: bank-control-check <resource-uuid> <IDENTITY|SANCTIONS|RISK|FRAUD>"
            )
        })?
        .parse::<Uuid>()?;
    let kind: CheckKind = serde_json::from_value(serde_json::json!(
        std::env::args().nth(2).unwrap_or_else(|| "IDENTITY".into())
    ))?;
    let pool = Module::Controls.connect().await?;
    let service = Checks::new(pool.clone())?;
    let actor = ActorContext {
        issuer: "urn:piggybank:local-diagnostics".into(),
        subject: "local-operator".into(),
        trust: TrustContext::Service,
        client_id: "bank-control-check".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let check = service
        .request(
            &actor,
            resource,
            kind,
            &fingerprint(&serde_json::json!({"fixture":"local-control-diagnostic"}))?,
        )
        .await?;
    println!("{}", serde_json::to_string(&check)?);
    pool.close().await;
    Ok(())
}
