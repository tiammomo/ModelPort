use std::sync::Arc;

use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::{
    AppError,
    auth::AuthStore,
    config::{AppConfig, ConfigIssueSeverity, RuntimeConfig},
    control::ControlStore,
    enterprise_ledger::EnterpriseLedger,
    http::HttpTransport,
    metrics::Metrics,
    routes::{self, AppState, GatewaySecurityPolicy, RateLimiter, TrustedProxyConfig},
};

pub(crate) async fn serve() -> Result<(), AppError> {
    let config = AppConfig::load()?;
    let issues = config.validation_issues();
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == ConfigIssueSeverity::Error)
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(AppError::Config(format!(
            "startup configuration validation failed: {}",
            errors.join("; ")
        )));
    }
    for issue in issues
        .iter()
        .filter(|issue| issue.severity == ConfigIssueSeverity::Warning)
    {
        warn!(message = %issue.message, "configuration warning");
    }

    let bind_addr = config.bind_addr;
    let ledger = Arc::new(EnterpriseLedger::connect_from_env().await?);
    let reconciled = ledger.reconcile_expired().await?;
    if reconciled.requests > 0 || reconciled.attempts > 0 {
        warn!(
            requests = reconciled.requests,
            attempts = reconciled.attempts,
            "reconciled expired inference ledger leases during startup"
        );
    }
    ledger.spawn_reconciler();
    let state = AppState {
        config: Arc::new(RuntimeConfig::new(config.clone())),
        auth: Arc::new(AuthStore::load_or_bootstrap(&config)?),
        control: Arc::new(ControlStore::load()?),
        security: Arc::new(GatewaySecurityPolicy::from_env()),
        rate_limiter: Arc::new(RateLimiter::from_env()),
        stream_permits: Arc::new(tokio::sync::Semaphore::new(stream_concurrency_limit(
            config.max_concurrent_requests,
        ))),
        trusted_proxies: Arc::new(TrustedProxyConfig::from_env()?),
        transport: HttpTransport::new()?,
        metrics: Arc::new(Metrics::new()),
        ledger,
    };

    let listener = TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "ModelPort listening");

    axum::serve(
        listener,
        routes::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

fn stream_concurrency_limit(default: usize) -> usize {
    std::env::var("MODELPORT_MAX_CONCURRENT_STREAMS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
        .max(1)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "failed to install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
