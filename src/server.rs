use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::{
    AppError,
    auth::AuthStore,
    config::{AppConfig, ConfigIssueSeverity, RuntimeConfig},
    control::ControlStore,
    deployment,
    enterprise_ledger::EnterpriseLedger,
    finalization::FinalizationTracker,
    governance::{GovernanceStore, LocalScheduler, LocalSchedulerConfig},
    http::HttpTransport,
    metrics::Metrics,
    oidc::OidcService,
    routes::{
        self, AppState, GatewaySecurityPolicy, RateLimiter, RetentionPreviewStore,
        TrustedProxyConfig,
    },
    runtime_adapter::collector::RuntimeAdapterCollector,
    smart_router::SmartRouter,
    version,
};

pub(crate) async fn serve() -> Result<(), AppError> {
    let config = AppConfig::load()?;
    deployment::validate_environment()?;
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
    let finalizers = Arc::new(FinalizationTracker::default());
    let draining = Arc::new(AtomicBool::new(false));
    let metrics = Arc::new(Metrics::new());
    let reconciled = ledger.reconcile_expired().await?;
    metrics.record_ledger_operation("lease_reconciliation", true);
    metrics.record_reconciliation(reconciled.requests, reconciled.attempts);
    if reconciled.requests > 0 || reconciled.attempts > 0 {
        warn!(
            requests = reconciled.requests,
            attempts = reconciled.attempts,
            "reconciled expired inference ledger leases during startup"
        );
    }
    ledger.spawn_reconciler(metrics.clone());
    let state = AppState {
        config: Arc::new(RuntimeConfig::new(config.clone())),
        auth: Arc::new(AuthStore::load_or_bootstrap(&config)?),
        oidc: Arc::new(OidcService::from_env()?),
        control: Arc::new(ControlStore::load()?),
        security: Arc::new(GatewaySecurityPolicy::from_env()),
        rate_limiter: Arc::new(RateLimiter::from_env()),
        stream_permits: Arc::new(tokio::sync::Semaphore::new(stream_concurrency_limit(
            config.max_concurrent_requests,
        ))),
        trusted_proxies: Arc::new(TrustedProxyConfig::from_env()?),
        transport: HttpTransport::new()?,
        metrics,
        smart_router: Arc::new(SmartRouter::new()),
        governance: Arc::new(GovernanceStore::load()?),
        local_scheduler: LocalScheduler::new(LocalSchedulerConfig::from_env()?),
        ledger,
        finalizers: finalizers.clone(),
        draining: draining.clone(),
        retention_previews: Arc::new(RetentionPreviewStore::default()),
    };

    state.oidc.validate_console_access(&state.auth)?;
    let listener = TcpListener::bind(bind_addr).await?;
    let collector = RuntimeAdapterCollector::start(
        state.config.snapshot().runtime_adapters,
        state.ledger.clone(),
        state.metrics.clone(),
        draining.clone(),
    )?;
    info!(
        %bind_addr,
        version = version::VERSION,
        revision = version::REVISION,
        source_state = version::SOURCE_STATE,
        "ModelPort listening"
    );

    let serve_result = axum::serve(
        listener,
        routes::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(draining))
    .await;

    let collector_timeout = runtime_adapter_collector_drain_timeout();
    if !collector.shutdown(collector_timeout).await {
        warn!(
            timeout_seconds = collector_timeout.as_secs(),
            "timed out draining Runtime Adapter collection tasks during shutdown"
        );
    }
    serve_result?;

    let drain_timeout = finalization_drain_timeout();
    if !finalizers.drain(drain_timeout).await {
        warn!(
            pending = finalizers.active(),
            timeout_seconds = drain_timeout.as_secs(),
            "timed out draining inference ledger finalizers during shutdown"
        );
    }

    Ok(())
}

fn stream_concurrency_limit(default: usize) -> usize {
    std::env::var("MODELPORT_MAX_CONCURRENT_STREAMS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
        .max(1)
}

fn finalization_drain_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("MODELPORT_FINALIZATION_DRAIN_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30)
            .max(1),
    )
}

fn runtime_adapter_collector_drain_timeout() -> Duration {
    Duration::from_secs(10)
}

async fn shutdown_signal(draining: Arc<AtomicBool>) {
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
    draining.store(true, Ordering::Release);
    info!("shutdown signal received; readiness disabled and new inference is draining");
}
