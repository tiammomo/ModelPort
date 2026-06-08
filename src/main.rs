mod config;
mod error;
mod http;
mod metrics;
mod providers;
mod routes;
mod types;

use std::sync::Arc;

use config::{AppConfig, ConfigIssueSeverity};
use error::AppError;
use http::HttpTransport;
use metrics::Metrics;
use routes::AppState;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("model_port=info,tower_http=info,axum=info")),
        )
        .init();

    if handle_cli()? {
        return Ok(());
    }

    let config = AppConfig::load()?;
    let bind_addr = config.bind_addr;
    let state = AppState {
        config: Arc::new(config),
        transport: HttpTransport::new()?,
        metrics: Arc::new(Metrics::new()),
    };

    let listener = TcpListener::bind(bind_addr).await?;
    info!("ModelPort listening on http://{bind_addr}");

    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn handle_cli() -> Result<bool, AppError> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(false),
        [flag] if flag == "-h" || flag == "--help" => {
            print_usage();
            Ok(true)
        }
        [command, subcommand] if command == "config" && subcommand == "validate" => {
            validate_config()?;
            Ok(true)
        }
        _ => Err(AppError::InvalidRequest(format!(
            "unknown command `{}`; run `model-port --help`",
            args.join(" ")
        ))),
    }
}

fn validate_config() -> Result<(), AppError> {
    let config = AppConfig::load()?;
    let issues = config.validation_issues();
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == ConfigIssueSeverity::Error)
        .count();
    let warnings = issues
        .iter()
        .filter(|issue| issue.severity == ConfigIssueSeverity::Warning)
        .count();

    println!("ModelPort configuration");
    println!("  bind: {}", config.bind_addr);
    println!("  default_provider: {}", config.default_provider);
    println!("  providers: {}", config.provider_order.join(", "));
    println!(
        "  auth: {}",
        if config.auth_token.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );

    for issue in &issues {
        let label = match issue.severity {
            ConfigIssueSeverity::Error => "ERROR",
            ConfigIssueSeverity::Warning => "WARN",
        };
        println!("[{label}] {}", issue.message);
    }

    if errors > 0 {
        return Err(AppError::Config(format!(
            "configuration validation failed with {errors} error(s) and {warnings} warning(s)"
        )));
    }

    println!("ModelPort configuration valid: {warnings} warning(s).");
    Ok(())
}

fn print_usage() {
    println!(
        "Usage:\n  model-port\n  model-port config validate\n\nCommands:\n  config validate    Load and validate ModelPort configuration without starting the server"
    );
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
