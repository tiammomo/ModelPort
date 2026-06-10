use std::{
    env,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    extract::State,
    http::{
        HeaderMap,
        header::{CONTENT_TYPE, HeaderName},
    },
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde_json::{Value, json};
use tower::{ServiceBuilder, limit::ConcurrencyLimitLayer};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::info;

use crate::{
    config::{AppConfig, ProviderProtocol},
    error::AppError,
    http::HttpTransport,
    metrics::Metrics,
    providers,
    types::AnthropicRequest,
};

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub transport: HttpTransport,
    pub metrics: Arc<Metrics>,
}

pub fn router(state: AppState) -> Router {
    let max_request_body_bytes = state.config.max_request_body_bytes;
    let max_concurrent_requests = state.config.max_concurrent_requests;

    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/models", get(models))
        .route("/v1/messages", post(messages))
        .route("/admin/dashboard", get(admin_dashboard))
        .route("/admin/providers", get(admin_providers))
        .route(
            "/admin/aliases",
            get(admin_aliases).post(admin_create_alias),
        )
        .route("/admin/aliases/{alias}", delete(admin_delete_alias))
        .route(
            "/admin/settings",
            get(admin_settings).put(admin_update_settings),
        )
        .route("/admin/settings/test-provider", post(admin_test_provider))
        .route("/admin/logs", get(admin_logs))
        .route("/admin/latency", get(admin_latency))
        .route("/admin/users", get(admin_users).post(admin_create_user))
        .route("/admin/users/{user_id}", delete(admin_delete_user))
        .route(
            "/admin/users/{user_id}/api-keys",
            get(admin_user_api_keys).post(admin_create_api_key),
        )
        .route("/admin/api-keys/{key_id}", delete(admin_revoke_api_key))
        .route("/admin/quotas", get(admin_quotas).post(admin_create_quota))
        .route(
            "/admin/quotas/{quota_id}",
            put(admin_update_quota).delete(admin_delete_quota),
        )
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::new(
                    X_REQUEST_ID.clone(),
                    MakeRequestUuid,
                ))
                .layer(PropagateRequestIdLayer::new(X_REQUEST_ID.clone()))
                .layer(TraceLayer::new_for_http())
                .layer(ConcurrencyLimitLayer::new(max_concurrent_requests)),
        )
        .layer(DefaultBodyLimit::max(max_request_body_bytes))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let started = Instant::now();
    state
        .metrics
        .record_route("health", true, started.elapsed());

    Json(json!({
        "status": "ok",
        "service": "model-port",
        "providers": state.config.provider_order.clone(),
    }))
}

async fn metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let started = Instant::now();

    if let Err(err) = state.config.validate_client_auth(&headers) {
        state
            .metrics
            .record_route("metrics", false, started.elapsed());
        return Err(err);
    }

    state
        .metrics
        .record_route("metrics", true, started.elapsed());
    Ok((
        [(CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        state.metrics.render_prometheus(),
    ))
}

async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let started = Instant::now();
    let result = (|| {
        state.config.validate_client_auth(&headers)?;
        let data = state
            .config
            .model_list()
            .into_iter()
            .map(|(id, display_name)| {
                json!({
                    "id": id,
                    "type": "model",
                    "display_name": display_name,
                })
            })
            .collect::<Vec<_>>();

        Ok(Json(json!({
            "data": data,
            "has_more": false,
            "first_id": data.first().and_then(|model| model.get("id")).cloned(),
            "last_id": data.last().and_then(|model| model.get("id")).cloned(),
        })))
    })();

    state
        .metrics
        .record_route("models", result.is_ok(), started.elapsed());
    result
}

async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AnthropicRequest>,
) -> Result<Response, AppError> {
    let started = Instant::now();
    if let Err(err) = state.config.validate_client_auth(&headers) {
        state
            .metrics
            .record_route("messages", false, started.elapsed());
        return Err(err);
    }

    let resolved = match state.config.resolve(&request.model) {
        Ok(resolved) => resolved,
        Err(err) => {
            state
                .metrics
                .record_route("messages", false, started.elapsed());
            return Err(err);
        }
    };
    let stream = request.stream.unwrap_or(false);
    info!(
        request_id = headers
            .get(&X_REQUEST_ID)
            .and_then(|value| value.to_str().ok())
            .unwrap_or(""),
        requested_model = request.model.as_str(),
        provider = resolved.provider_id.as_str(),
        upstream_model = resolved.model.as_str(),
        stream,
        "routing message request"
    );

    let provider_id = resolved.provider_id.clone();
    let upstream_model = resolved.model.clone();
    let result = match resolved.provider.protocol {
        ProviderProtocol::Anthropic => {
            providers::anthropic::messages(state.clone(), resolved, request)
                .await
                .map(IntoResponse::into_response)
        }
        ProviderProtocol::OpenaiCompat => {
            providers::openai_compat::messages(state.clone(), resolved, request)
                .await
                .map(IntoResponse::into_response)
        }
    };
    let success = result.is_ok();
    let duration = started.elapsed();

    state.metrics.record_route("messages", success, duration);
    state
        .metrics
        .record_message(&provider_id, &upstream_model, stream, success, duration);
    result
}

async fn admin_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;

    let snapshot = state.metrics.snapshot();
    let total_requests = snapshot
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    let total_successes = snapshot
        .messages
        .iter()
        .map(|message| message.successes_total)
        .sum::<u64>();
    let total_failures = snapshot
        .messages
        .iter()
        .map(|message| message.failures_total)
        .sum::<u64>();
    let total_duration = snapshot
        .messages
        .iter()
        .map(|message| message.duration_ms_total)
        .sum::<u64>();
    let success_rate = percent(total_successes, total_requests);
    let avg_latency_ms = average(total_duration, total_requests);
    let route_requests = snapshot
        .routes
        .iter()
        .map(|route| route.requests_total)
        .sum::<u64>();
    let route_successes = snapshot
        .routes
        .iter()
        .map(|route| route.successes_total)
        .sum::<u64>();
    let route_failures = snapshot
        .routes
        .iter()
        .map(|route| route.failures_total)
        .sum::<u64>();
    let route_duration = snapshot
        .routes
        .iter()
        .map(|route| route.duration_ms_total)
        .sum::<u64>();
    let busiest_route = snapshot
        .routes
        .iter()
        .max_by_key(|route| route.requests_total)
        .map(|route| route.route.as_str())
        .unwrap_or("none");
    let providers = provider_rows(&state);
    let active_providers = providers
        .iter()
        .filter(|provider| provider.get("status").and_then(Value::as_str) == Some("active"))
        .count();
    let now = now_millis_string();

    Ok(Json(json!({
        "uptimeSeconds": snapshot.uptime_seconds,
        "totalRequests": total_requests,
        "successRate": success_rate,
        "activeProviders": active_providers,
        "totalProviders": providers.len(),
        "activeUsers": 1,
        "totalModels": state.config.model_list().len(),
        "avgLatencyMs": avg_latency_ms,
        "requestTimeSeries": time_series(total_requests),
        "errorTimeSeries": time_series(total_failures),
        "topModels": snapshot.messages.iter().map(|message| json!({
            "model": message.model,
            "provider": message.provider,
            "requests": message.requests_total,
        })).collect::<Vec<_>>(),
        "providerHealth": providers.iter().map(|provider| {
            let id = provider.get("id").and_then(Value::as_str).unwrap_or("");
            let provider_messages = snapshot.messages.iter().filter(|message| message.provider == id).collect::<Vec<_>>();
            let requests = provider_messages.iter().map(|message| message.requests_total).sum::<u64>();
            let successes = provider_messages.iter().map(|message| message.successes_total).sum::<u64>();
            let duration = provider_messages.iter().map(|message| message.duration_ms_total).sum::<u64>();
            json!({
                "providerId": id,
                "displayName": provider.get("displayName").cloned().unwrap_or_else(|| json!(id)),
                "status": provider.get("status").cloned().unwrap_or_else(|| json!("inactive")),
                "requestsTotal": requests,
                "successRate": percent(successes, requests),
                "avgLatencyMs": average(duration, requests),
            })
        }).collect::<Vec<_>>(),
        "recentActivity": vec![
            json!({
                "id": "act_health",
                "timestamp": now,
                "type": "request",
                "message": format!("ModelPort gateway is healthy; busiest route: {busiest_route}"),
                "severity": "info",
            }),
            json!({
                "id": "act_messages",
                "timestamp": now_millis_string(),
                "type": if total_failures > 0 { "error" } else { "request" },
                "message": format!("{total_requests} model message request(s), {total_failures} failure(s) since startup"),
                "severity": if total_failures > 0 { "warning" } else { "info" },
            }),
            json!({
                "id": "act_routes",
                "timestamp": now_millis_string(),
                "type": if route_failures > 0 { "error" } else { "request" },
                "message": format!("{route_requests} route request(s), {route_successes} success(es), avg {} ms", average(route_duration, route_requests)),
                "severity": if route_failures > 0 { "warning" } else { "info" },
            }),
        ],
    })))
}

async fn admin_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(Value::Array(provider_rows(&state))))
}

async fn admin_aliases(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(Value::Array(alias_rows(&state))))
}

async fn admin_create_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    let alias = body.get("alias").and_then(Value::as_str).unwrap_or("");
    let target = body.get("target").and_then(Value::as_str).unwrap_or("");
    Ok(Json(alias_row(&state, alias, target)))
}

async fn admin_delete_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!({ "ok": true })))
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(settings_row(&state)))
}

async fn admin_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(if body.is_object() {
        body
    } else {
        settings_row(&state)
    }))
}

async fn admin_test_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    let provider_id = body
        .get("providerId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(provider) = state.config.providers.get(provider_id) else {
        return Ok(Json(json!({
            "success": false,
            "message": "provider not found",
        })));
    };

    Ok(Json(json!({
        "success": provider.api_key().is_ok_and(|key| key.is_some() || !provider.api_key_required),
        "message": if provider.api_key().is_ok_and(|key| key.is_some() || !provider.api_key_required) {
            "configured"
        } else {
            "missing API key"
        },
    })))
}

async fn admin_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    let logs = log_rows(&state);
    Ok(Json(json!({
        "logs": logs,
        "total": logs.len(),
    })))
}

async fn admin_latency(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    let snapshot = state.metrics.snapshot();
    let total_requests = snapshot
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    let total_duration = snapshot
        .messages
        .iter()
        .map(|message| message.duration_ms_total)
        .sum::<u64>();
    let avg = average(total_duration, total_requests);

    Ok(Json(json!({
        "p50": avg,
        "p90": avg,
        "p95": avg,
        "p99": avg,
        "avg": avg,
        "max": avg,
        "byModel": {},
        "byProvider": {},
    })))
}

async fn admin_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    let requests = state
        .metrics
        .snapshot()
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    Ok(Json(json!([admin_user_row(requests)])))
}

async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!({
        "id": format!("usr_{}", now_millis_string()),
        "username": body.get("username").and_then(Value::as_str).unwrap_or("local-user"),
        "email": body.get("email").and_then(Value::as_str).unwrap_or("local@modelport"),
        "role": body.get("role").and_then(Value::as_str).unwrap_or("user"),
        "status": body.get("status").and_then(Value::as_str).unwrap_or("active"),
        "createdAt": now_millis_string(),
        "lastLoginAt": now_millis_string(),
        "apiKeyCount": 0,
        "requestCount24h": 0,
    })))
}

async fn admin_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!({ "ok": true })))
}

async fn admin_user_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!([{
        "id": "key_modelport_local",
        "userId": "usr_local_admin",
        "name": "MODELPORT_AUTH_TOKEN",
        "keyPrefix": "local-token",
        "createdAt": now_millis_string(),
        "lastUsedAt": null,
        "expiresAt": null,
        "status": "active",
    }])))
}

async fn admin_create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!({
        "id": format!("key_{}", now_millis_string()),
        "userId": body.get("userId").and_then(Value::as_str).unwrap_or("usr_local_admin"),
        "name": body.get("name").and_then(Value::as_str).unwrap_or("local key"),
        "keyPrefix": "mp-local-",
        "createdAt": now_millis_string(),
        "lastUsedAt": null,
        "expiresAt": null,
        "status": "active",
    })))
}

async fn admin_revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!({ "ok": true })))
}

async fn admin_quotas(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!([])))
}

async fn admin_create_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(body))
}

async fn admin_update_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(body))
}

async fn admin_delete_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    state.config.validate_client_auth(&headers)?;
    Ok(Json(json!({ "ok": true })))
}

fn provider_rows(state: &AppState) -> Vec<Value> {
    state
        .config
        .provider_order
        .iter()
        .filter_map(|id| {
            let provider = state.config.providers.get(id)?;
            let has_api_key = provider.api_key().ok().flatten().is_some();
            Some(json!({
                "id": id,
                "displayName": provider.display_name,
                "protocol": provider_protocol_value(provider.protocol),
                "baseUrl": provider.base_url,
                "apiKeyEnv": provider.api_key_env,
                "apiKeyRequired": provider.api_key_required,
                "defaultModel": provider.default_model,
                "models": provider.models,
                "modelPrefixes": provider.model_prefixes,
                "passthroughUnknownModels": provider.passthrough_unknown_models,
                "maxTokensField": max_tokens_field_value(provider.max_tokens_field),
                "deduplicateStreamText": provider.deduplicate_stream_text,
                "bufferStreamText": provider.buffer_stream_text,
                "status": if has_api_key || !provider.api_key_required { "active" } else { "inactive" },
                "hasApiKey": has_api_key,
            }))
        })
        .collect()
}

fn alias_rows(state: &AppState) -> Vec<Value> {
    state
        .config
        .aliases
        .iter()
        .map(|(alias, target)| alias_row(state, alias, target))
        .collect()
}

fn alias_row(state: &AppState, alias: &str, target: &str) -> Value {
    let resolved = state.config.resolve(alias).ok();
    json!({
        "alias": alias,
        "target": target,
        "resolvedProvider": resolved.as_ref().map(|value| value.provider_id.as_str()).unwrap_or(""),
        "resolvedModel": resolved.as_ref().map(|value| value.model.as_str()).unwrap_or(""),
    })
}

fn settings_row(state: &AppState) -> Value {
    json!({
        "server": {
            "bindAddress": state.config.bind_addr.to_string(),
            "maxRequestBodyBytes": state.config.max_request_body_bytes,
            "maxConcurrentRequests": state.config.max_concurrent_requests,
        },
        "auth": {
            "enabled": state.config.auth_token.is_some(),
            "tokenEnvVar": "MODELPORT_AUTH_TOKEN",
            "allowNoAuth": state.config.auth_token.is_none(),
        },
        "gateway": {
            "defaultProvider": state.config.default_provider,
            "providerOrder": state.config.provider_order,
        },
        "rateLimits": {
            "maxConcurrentRequests": state.config.max_concurrent_requests,
            "maxRequestBodyBytes": state.config.max_request_body_bytes,
            "requestTimeoutSecs": env_u64("MODELPORT_HTTP_REQUEST_TIMEOUT_SECS", 600),
            "streamIdleTimeoutSecs": env_u64("MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS", 300),
        },
    })
}

fn log_rows(state: &AppState) -> Vec<Value> {
    state
        .metrics
        .snapshot()
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let protocol = state
                .config
                .providers
                .get(&message.provider)
                .map(|provider| provider_protocol_value(provider.protocol))
                .unwrap_or("openai-compat");
            let requests = message.requests_total.max(1);
            json!({
                "id": format!("log_{}_{}_{}", message.provider, message.model.replace('/', "_"), if message.stream { "stream" } else { "nonstream" }),
                "timestamp": now_millis_string(),
                "userId": "usr_local_admin",
                "username": "local-admin",
                "model": message.model,
                "resolvedModel": message.model,
                "provider": message.provider,
                "protocol": protocol,
                "stream": if message.stream { "stream" } else { "non-stream" },
                "status": if message.failures_total > 0 { "error" } else { "success" },
                "statusCode": if message.failures_total > 0 { 502 } else { 200 },
                "inputTokens": 0,
                "outputTokens": 0,
                "latencyMs": average(message.duration_ms_total, requests),
                "errorMessage": if message.failures_total > 0 { Some(format!("{} failure(s) recorded", message.failures_total)) } else { None },
                "sortIndex": index,
            })
        })
        .collect()
}

fn admin_user_row(requests: u64) -> Value {
    json!({
        "id": "usr_local_admin",
        "username": "local-admin",
        "email": "local@modelport",
        "role": "admin",
        "status": "active",
        "createdAt": now_millis_string(),
        "lastLoginAt": now_millis_string(),
        "apiKeyCount": 1,
        "requestCount24h": requests,
    })
}

fn provider_protocol_value(protocol: ProviderProtocol) -> &'static str {
    match protocol {
        ProviderProtocol::Anthropic => "anthropic",
        ProviderProtocol::OpenaiCompat => "openai-compat",
    }
}

fn max_tokens_field_value(field: crate::config::MaxTokensField) -> &'static str {
    match field {
        crate::config::MaxTokensField::MaxCompletionTokens => "max_completion_tokens",
        crate::config::MaxTokensField::MaxTokens => "max_tokens",
        crate::config::MaxTokensField::Both => "both",
    }
}

fn time_series(value: u64) -> Vec<Value> {
    let now = now_millis();
    (0..24)
        .map(|offset| {
            json!({
                "timestamp": (now.saturating_sub((23 - offset) * 3_600_000)).to_string(),
                "value": if offset == 23 { value } else { 0 },
            })
        })
        .collect()
}

fn percent(successes: u64, total: u64) -> f64 {
    if total == 0 {
        100.0
    } else {
        (successes as f64 / total as f64) * 100.0
    }
}

fn average(total: u64, count: u64) -> u64 {
    if count == 0 { 0 } else { total / count }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn now_millis_string() -> String {
    now_millis().to_string()
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use axum::{
        body::{Body, to_bytes},
        http::{
            Request, StatusCode,
            header::{CONTENT_TYPE, HeaderValue},
        },
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        config::{MaxTokensField, ProviderConfig},
        metrics::Metrics,
    };

    const CLIENT_TOKEN: &str = "client-token";

    #[tokio::test]
    async fn routes_non_stream_openai_compatible_response() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl_test",
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "hello from upstream"
                        },
                        "finish_reason": "stop"
                    }
                ],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 4
                }
            }"#,
            "application/json",
        )
        .await;
        let app = router(test_state(upstream, 1024 * 1024));

        let (status, body) = post_message(app, message_body(false)).await;

        assert_eq!(status, StatusCode::OK);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["content"][0]["text"], "hello from upstream");
        assert_eq!(body["usage"]["input_tokens"], 3);
        assert_eq!(body["usage"]["output_tokens"], 4);
    }

    #[tokio::test]
    async fn propagates_non_stream_upstream_status() {
        let upstream = spawn_openai_upstream(
            StatusCode::UNAUTHORIZED,
            r#"{"code":"INVALID_API_KEY","message":"Invalid API key"}"#,
            "application/json",
        )
        .await;
        let app = router(test_state(upstream, 1024 * 1024));

        let (status, body) = post_message(app, message_body(false)).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("upstream returned HTTP 401"));
        assert!(body.contains("INVALID_API_KEY"));
    }

    #[tokio::test]
    async fn maps_stream_upstream_status_to_anthropic_error_event() {
        let upstream = spawn_openai_upstream(
            StatusCode::UNAUTHORIZED,
            r#"{"code":"INVALID_API_KEY","message":"Invalid API key"}"#,
            "application/json",
        )
        .await;
        let app = router(test_state(upstream, 1024 * 1024));

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("event: error"));
        assert!(body.contains("upstream returned HTTP 401"));
        assert!(body.contains("INVALID_API_KEY"));
    }

    #[tokio::test]
    async fn supports_multiple_openai_data_lines_in_one_sse_frame() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"data: {"choices":[{"delta":{"role":"assistant","content":""},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"content":"hel"},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"content":"hello"},"finish_reason":null,"index":0}]}

data: [DONE]

"#,
            "text/event-stream",
        )
        .await;
        let app = router(test_state(upstream, 1024 * 1024));

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains(r#""text":"hel""#));
        assert!(body.contains(r#""text":"lo""#));
        assert!(!body.contains("event: error"));
    }

    #[tokio::test]
    async fn deduplicates_cumulative_stream_tool_arguments() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"Agent","arguments":""}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"description\": "}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"description\": "}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\""}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"scan"}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"description\": \"scan\", \"prompt\": "}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\""}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"list project files"}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"description\": \"scan\", \"prompt\": \"list project files\"}"}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\""}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"}"}}]},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}
data: [DONE]

"#,
            "text/event-stream",
        )
        .await;
        let app = router(test_state(upstream, 1024 * 1024));

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""name":"Agent""#));
        assert!(!body.contains(r#""partial_json":"""#));
        assert_eq!(body.matches(r#""partial_json":"#).count(), 1);
        assert!(body.contains(
            r#""partial_json":"{\"description\": \"scan\", \"prompt\": \"list project files\"}""#
        ));
        assert_eq!(body.matches(r#""stop_reason":"tool_use""#).count(), 1);
        assert!(!body.contains("event: error"));
    }

    #[tokio::test]
    async fn buffers_stream_text_from_non_stream_openai_response() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl_buffered",
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "| 项目 | 状态 |\n|------|------|\n| 前端 | 正常 |"
                        },
                        "finish_reason": "stop"
                    }
                ],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 4
                }
            }"#,
            "application/json",
        )
        .await;
        let app = router(test_state_with_flags(upstream, 1024 * 1024, true, true));

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("event: message_start"));
        assert!(body.contains(r#""text":"| 项目 | 状态 |\n""#));
        assert!(body.contains(r#""text":"|------|------|\n""#));
        assert!(body.contains(r#""text":"| 前端 | 正常 |""#));
        assert!(body.contains(r#""output_tokens":4"#));
        assert!(body.contains("event: message_stop"));
        assert!(!body.contains("event: error"));
    }

    #[tokio::test]
    async fn rejects_oversized_message_request_body() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 16));

        let (status, _body) = post_message(app, message_body(false)).await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn metrics_endpoint_requires_auth() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn metrics_endpoint_records_message_requests() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl_test",
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "hello from upstream"
                        },
                        "finish_reason": "stop"
                    }
                ]
            }"#,
            "application/json",
        )
        .await;
        let app = router(test_state(upstream, 1024 * 1024));

        let (message_status, _) = post_message(app.clone(), message_body(false)).await;
        assert_eq!(message_status, StatusCode::OK);

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/metrics")
                    .header("x-api-key", CLIENT_TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains(r#"modelport_route_requests_total{route="messages"} 1"#));
        assert!(body.contains(
            r#"modelport_message_requests_total{provider="mimo",model="mimo-v2.5-pro",stream="false"} 1"#
        ));
    }

    async fn post_message(app: Router, body: Value) -> (StatusCode, String) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("x-api-key", CLIENT_TOKEN)
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    async fn spawn_openai_upstream(
        status: StatusCode,
        body: &'static str,
        content_type: &'static str,
    ) -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || async move { (status, [(CONTENT_TYPE, content_type)], body) }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{addr}/v1")
    }

    fn test_state(base_url: String, max_request_body_bytes: usize) -> AppState {
        test_state_with_flags(base_url, max_request_body_bytes, true, false)
    }

    fn test_state_with_flags(
        base_url: String,
        max_request_body_bytes: usize,
        deduplicate_stream_text: bool,
        buffer_stream_text: bool,
    ) -> AppState {
        let provider = ProviderConfig {
            display_name: "Mimo".to_owned(),
            protocol: ProviderProtocol::OpenaiCompat,
            base_url,
            api_key_env: None,
            api_key: Some("upstream-key".to_owned()),
            api_key_required: true,
            default_model: "mimo-v2.5-pro".to_owned(),
            models: vec!["mimo-v2.5-pro".to_owned()],
            model_prefixes: vec!["mimo-".to_owned()],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            deduplicate_stream_text,
            buffer_stream_text,
        };

        AppState {
            config: Arc::new(AppConfig {
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                max_request_body_bytes,
                max_concurrent_requests: 16,
                auth_token: Some(CLIENT_TOKEN.to_owned()),
                default_provider: "mimo".to_owned(),
                provider_order: vec!["mimo".to_owned()],
                providers: HashMap::from([("mimo".to_owned(), provider)]),
                aliases: HashMap::new(),
            }),
            transport: HttpTransport::new().unwrap(),
            metrics: Arc::new(Metrics::new()),
        }
    }

    fn message_body(stream: bool) -> Value {
        json!({
            "model": "mimo-v2.5-pro",
            "max_tokens": 32,
            "stream": stream,
            "messages": [
                {
                    "role": "user",
                    "content": "hello"
                }
            ]
        })
    }
}
