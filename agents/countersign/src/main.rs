mod config;
mod fixer;
mod land;
mod ledger;
mod process;
mod switch;
mod verifier;
mod verify;

use anyhow::Result;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

use crate::config::{Config, Role};
use crate::fixer::Fixer;
use crate::switch::SwitchClient;
use crate::verifier::Verifier;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("countersign=info,warn")),
        )
        .with_target(false)
        .compact()
        .init();

    let config = Config::from_env()?;
    let client = Arc::new(SwitchClient::new(&config));
    tracing::info!(
        role = config.role.as_str(),
        agent = %config.agent_id,
        repo = %config.repo.display(),
        "starting Countersign"
    );

    match client.ops().await {
        Ok(ops) => {
            let count = ops
                .get("operations")
                .and_then(|value| value.as_object())
                .map(|operations| operations.len())
                .unwrap_or(0);
            tracing::info!(count, "operation surface discovered");
        }
        Err(error) => tracing::warn!("could not list operations: {error:#}"),
    }

    match config.role {
        Role::Verifier => {
            let handler = Arc::new(Verifier::new(config, client.clone())?);
            client.run(handler).await
        }
        Role::Fixer => {
            let handler = Arc::new(Fixer::new(config, client.clone()));
            client.run(handler).await
        }
    }
}
