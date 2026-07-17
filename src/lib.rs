#![recursion_limit = "256"]

mod auth;
mod cli;
mod config;
mod control;
mod control_view;
mod database;
mod deployment;
mod domain;
mod enterprise_ledger;
mod error;
mod exchange;
mod fidelity;
mod http;
mod metrics;
mod oidc;
mod policy;
mod pricing;
mod provider_credentials;
mod provider_status;
mod providers;
mod routes;
mod server;
mod storage;
mod stream_lifecycle;
mod tool_use;
mod types;
mod usage;

pub use error::AppError;

use tracing_subscriber::{EnvFilter, fmt};

/// Runs either a CLI command or the long-lived gateway server.
pub async fn run(args: Vec<String>) -> Result<(), AppError> {
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("model_port=info,tower_http=info,axum=info")),
        )
        .try_init();

    if cli::handle(args)? {
        return Ok(());
    }
    server::serve().await
}
