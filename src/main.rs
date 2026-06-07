mod config;
mod error;
mod http;
mod providers;
mod routes;
mod types;

use std::sync::Arc;

use config::AppConfig;
use error::AppError;
use http::HttpTransport;
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

    let config = AppConfig::load()?;
    let bind_addr = config.bind_addr;
    let state = AppState {
        config: Arc::new(config),
        transport: HttpTransport::new()?,
    };

    let listener = TcpListener::bind(bind_addr).await?;
    info!("ModelPort listening on http://{bind_addr}");

    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
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
