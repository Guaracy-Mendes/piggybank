//! Explicit synthetic balanced funding fixture; no real currency/provider is supported.
use bank_core::Core;
use bank_platform::{
    config::Config,
    database::Module,
    domain::{AccountId, Currency, Money},
};
use uuid::Uuid;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = Config::load(4181)?;
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(
        arguments.len() == 3,
        "Usage: bank-fund <operation-uuid> <account-uuid> <minor-units>"
    );
    let operation = arguments[0].parse::<Uuid>()?;
    let account = AccountId(arguments[1].parse::<Uuid>()?);
    let amount: Money = serde_json::from_value(
        serde_json::json!({"minorUnits":arguments[2],"currency":Currency::synthetic()}),
    )?;
    let core = Core::new(Module::Core.connect().await?);
    let batch = core
        .fund_synthetic(
            operation,
            account,
            AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?),
            amount,
        )
        .await?;
    println!("{batch}");
    Ok(())
}
