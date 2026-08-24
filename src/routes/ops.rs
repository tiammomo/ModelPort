use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, header::CONTENT_TYPE},
    response::IntoResponse,
    routing::get,
};
use serde_json::json;

use super::*;

#[cfg(test)]
use super::route_contract::RouteContract;

const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[cfg(test)]
pub(super) const ROUTES: &[RouteContract] = &[
    RouteContract::new("system", "/livez", &["GET"]),
    RouteContract::new("system", "/readyz", &["GET"]),
    RouteContract::new("system", "/health", &["GET"]),
    RouteContract::new("system", "/metrics", &["GET"]),
];

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
}

pub(super) async fn livez(State(state): State<AppState>) -> Json<serde_json::Value> {
    let started = Instant::now();
    state.metrics.record_route("livez", true, started.elapsed());
    Json(json!({
        "status": "ok",
        "service": "model-port",
        "build": crate::version::json(),
    }))
}

pub(super) async fn readyz(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let started = Instant::now();
    let result = async {
        authenticate_client(&state, &headers)?;
        if state.is_draining() {
            return Err(AppError::NotReady(
                "gateway is draining for shutdown".to_owned(),
            ));
        }
        state
            .auth
            .health_check()
            .map_err(|error| AppError::NotReady(format!("auth storage: {error}")))?;
        state
            .control
            .health_check()
            .map_err(|error| AppError::NotReady(format!("control storage: {error}")))?;
        if !state.governance.is_ready() {
            return Err(AppError::NotReady(
                "governance storage persistence is degraded".to_owned(),
            ));
        }
        state
            .ledger
            .health_check()
            .await
            .map_err(|error| AppError::NotReady(format!("enterprise ledger: {error}")))?;
        let degraded_operations = state.metrics.degraded_ledger_operations();
        if !degraded_operations.is_empty() {
            return Err(AppError::NotReady(format!(
                "enterprise ledger operations degraded: {}",
                degraded_operations.join(", ")
            )));
        }
        Ok(Json(detailed_health_body(&state)))
    }
    .await;
    state
        .metrics
        .record_route("readyz", result.is_ok(), started.elapsed());
    result
}

pub(super) async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<serde_json::Value> {
    let started = Instant::now();
    let detailed = state.security.expose_detailed_public_health
        || authenticate_client(&state, &headers).is_ok();
    state
        .metrics
        .record_route("health", true, started.elapsed());
    if detailed {
        Json(detailed_health_body(&state))
    } else {
        Json(json!({
            "status": "ok",
            "service": "model-port",
            "build": crate::version::json(),
        }))
    }
}

fn detailed_health_body(state: &AppState) -> serde_json::Value {
    let provider_health = state.control.provider_health_rows();
    let config = effective_config(state);

    json!({
        "status": if state.is_draining() { "draining" } else { "ok" },
        "service": "model-port",
        "build": crate::version::json(),
        "providers": config.provider_order,
        "storage": {
            "auth": state.auth.data_path(),
            "control": state.control.data_path(),
            "enterpriseLedger": state.ledger.location(),
            "governance": if state.governance.is_ready() { "ready" } else { "degraded" },
            "status": "ready",
            "pendingFinalizers": state.finalizers.active(),
            "degradedLedgerOperations": state.metrics.degraded_ledger_operations(),
        },
        "draining": state.is_draining(),
        "providerHealth": provider_health,
    })
}

pub(super) async fn metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let started = Instant::now();

    if let Err(err) = authenticate_client(&state, &headers) {
        state
            .metrics
            .record_route("metrics", false, started.elapsed());
        return Err(err);
    }

    state
        .metrics
        .record_route("metrics", true, started.elapsed());
    let mut metrics = state.metrics.render_prometheus();
    metrics.push_str(
        "\n# HELP modelport_ledger_pending_finalizers Streaming ledger finalizers pending database commit.\n",
    );
    metrics.push_str("# TYPE modelport_ledger_pending_finalizers gauge\n");
    metrics.push_str(&format!(
        "modelport_ledger_pending_finalizers {}\n",
        state.finalizers.active()
    ));
    append_runtime_metrics(&state, &mut metrics).await;
    Ok(([(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], metrics))
}

async fn append_runtime_metrics(state: &AppState, output: &mut String) {
    let database_ready = state.ledger.health_check().await.is_ok();
    let auth_ready = state.auth.health_check().is_ok();
    let control_ready = state.control.health_check().is_ok();
    let governance_ready = state.governance.is_ready();
    let ledger_operations_ready = state.metrics.degraded_ledger_operations().is_empty();
    let draining = state.is_draining();
    let gateway_ready = !draining
        && database_ready
        && auth_ready
        && control_ready
        && governance_ready
        && ledger_operations_ready;

    output.push_str("\n# HELP modelport_gateway_ready Whether all fail-closed gateway dependencies are currently ready.\n");
    output.push_str("# TYPE modelport_gateway_ready gauge\n");
    output.push_str(&format!(
        "modelport_gateway_ready {}\n",
        u8::from(gateway_ready)
    ));
    output.push_str(
        "# HELP modelport_gateway_draining Whether shutdown drain mode is rejecting new inference traffic.\n",
    );
    output.push_str("# TYPE modelport_gateway_draining gauge\n");
    output.push_str(&format!(
        "modelport_gateway_draining {}\n",
        u8::from(draining)
    ));
    output.push_str("# HELP modelport_database_ready Whether the PostgreSQL operational ledger responds to a readiness query.\n");
    output.push_str("# TYPE modelport_database_ready gauge\n");
    output.push_str(&format!(
        "modelport_database_ready {}\n",
        u8::from(database_ready)
    ));
    if let Some(pool) = state.ledger.database_pool_snapshot() {
        let in_use = pool.size.saturating_sub(pool.idle);
        let utilization = if pool.max == 0 {
            0.0
        } else {
            f64::from(in_use) / f64::from(pool.max)
        };
        output.push_str(
            "# HELP modelport_database_pool_connections PostgreSQL pool connections by state.\n",
        );
        output.push_str("# TYPE modelport_database_pool_connections gauge\n");
        output.push_str(&format!(
            "modelport_database_pool_connections{{state=\"open\"}} {}\n",
            pool.size
        ));
        output.push_str(&format!(
            "modelport_database_pool_connections{{state=\"idle\"}} {}\n",
            pool.idle
        ));
        output.push_str(&format!(
            "modelport_database_pool_connections{{state=\"in_use\"}} {in_use}\n"
        ));
        output.push_str(
            "# HELP modelport_database_pool_max_connections Configured PostgreSQL pool limit.\n",
        );
        output.push_str("# TYPE modelport_database_pool_max_connections gauge\n");
        output.push_str(&format!(
            "modelport_database_pool_max_connections {}\n",
            pool.max
        ));
        output.push_str("# HELP modelport_database_pool_utilization_ratio In-use PostgreSQL connections divided by the configured pool limit.\n");
        output.push_str("# TYPE modelport_database_pool_utilization_ratio gauge\n");
        output.push_str(&format!(
            "modelport_database_pool_utilization_ratio {utilization:.6}\n"
        ));
    }

    let scheduler = state.local_scheduler.snapshot();
    for (metric, field) in [
        ("running", "running"),
        ("interactive_queued", "interactiveQueued"),
        ("batch_queued", "batchQueued"),
        ("users_queued", "usersQueued"),
        ("estimated_service_ms", "estimatedServiceMs"),
        ("oldest_interactive_wait_ms", "oldestInteractiveWaitMs"),
        ("oldest_batch_wait_ms", "oldestBatchWaitMs"),
    ] {
        let value = scheduler
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        output.push_str(&format!(
            "# TYPE modelport_local_scheduler_{metric} gauge\n"
        ));
        output.push_str(&format!("modelport_local_scheduler_{metric} {value}\n"));
    }
    output.push_str(
        "# HELP modelport_stream_permits_available Inference stream permits currently available.\n",
    );
    output.push_str("# TYPE modelport_stream_permits_available gauge\n");
    output.push_str(&format!(
        "modelport_stream_permits_available {}\n",
        state.stream_permits.available_permits()
    ));

    output.push_str("# HELP modelport_provider_available Whether a configured Provider has usable credentials and is not cooling down.\n");
    output.push_str("# TYPE modelport_provider_available gauge\n");
    output.push_str(
        "# HELP modelport_provider_cooldown Whether routing has placed a Provider in cooldown.\n",
    );
    output.push_str("# TYPE modelport_provider_cooldown gauge\n");
    let config = effective_config(state);
    for provider_id in &config.provider_order {
        let Some(provider) = config.providers.get(provider_id) else {
            continue;
        };
        let credential_ready = state
            .control
            .provider_credential_route_available(provider_id)
            .unwrap_or(
                !provider.api_key_required
                    || provider
                        .api_key
                        .as_deref()
                        .is_some_and(|key| !key.trim().is_empty()),
            );
        let cooldown = state.control.provider_in_cooldown(provider_id);
        let provider = prometheus_label(provider_id);
        output.push_str(&format!(
            "modelport_provider_available{{provider=\"{provider}\"}} {}\n",
            u8::from(credential_ready && !cooldown)
        ));
        output.push_str(&format!(
            "modelport_provider_cooldown{{provider=\"{provider}\"}} {}\n",
            u8::from(cooldown)
        ));
    }
}

fn prometheus_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
