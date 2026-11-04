mod api;
mod config;
mod fixer;
mod land;
mod ledger;
mod process;
mod switch;
mod verifier;
mod verify;

use anyhow::{Context, Result};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

use crate::config::{Config, Role};
use crate::fixer::Fixer;
use crate::ledger::Ledger;
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
            let ledger = Ledger::open(&config.ledger_path)?;
            if let Ok(port) = std::env::var("COUNTERSIGN_API_PORT") {
                let port: u16 = port.parse().context("COUNTERSIGN_API_PORT must be a port")?;
                let viewer_dir = config.repo.join("viewer/dist");
                let ledger = ledger.clone();
                tokio::spawn(async move {
                    if let Err(error) = api::serve(ledger, port, viewer_dir).await {
                        tracing::error!("ledger viewer failed: {error:#}");
                    }
                });
            }
            let handler = Arc::new(Verifier::new(config, client.clone(), ledger)?);
            client.run(handler).await
        }
        Role::Fixer => {
            let handler = Arc::new(Fixer::new(config, client.clone()));
            client.run(handler).await
        }
    }
}
