use std::{
    collections::BTreeSet,
    env,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{
        HeaderMap, HeaderValue,
        header::{CONTENT_TYPE, HeaderName, SET_COOKIE},
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
use tracing::{info, warn};

use crate::{
    auth::{AuthStore, CreateUserInput, LoginInput, PublicUser, UpdateUserInput},
    config::{AppConfig, ProviderConfig, ProviderProtocol},
    control::{
        ActivityInput, ClientIdentity, ControlStore, CreateApiKeyInput, UpdateApiKeyInput,
        UpsertQuotaInput, UsageEstimate, UsageEventInput,
    },
    error::AppError,
    http::{Header, HttpTransport},
    metrics::Metrics,
    pricing::{self, TokenUsageBreakdown},
    providers,
    types::AnthropicRequest,
};

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub auth: Arc<AuthStore>,
    pub control: Arc<ControlStore>,
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
        .route("/admin/auth/login", post(admin_login))
        .route("/admin/auth/logout", post(admin_logout))
        .route("/admin/auth/me", get(admin_me))
        .route("/admin/dashboard", get(admin_dashboard))
        .route("/admin/providers", get(admin_providers))
        .route(
            "/admin/providers/{provider_id}/models",
            get(admin_provider_models),
        )
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
        .route(
            "/admin/users/{user_id}",
            put(admin_update_user).delete(admin_delete_user),
        )
        .route(
            "/admin/api-keys",
            get(admin_api_keys).post(admin_create_api_key),
        )
        .route(
            "/admin/api-keys/{key_id}/disable",
            post(admin_revoke_api_key),
        )
        .route(
            "/admin/users/{user_id}/api-keys",
            get(admin_user_api_keys).post(admin_create_api_key),
        )
        .route(
            "/admin/api-keys/{key_id}",
            put(admin_update_api_key).delete(admin_delete_api_key),
        )
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
        authenticate_client(&state, &headers)?;
        let config = effective_config(&state);
        let data = config
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
    let identity = match authenticate_client(&state, &headers) {
        Ok(identity) => identity,
        Err(err) => {
            state
                .metrics
                .record_route("messages", false, started.elapsed());
            return Err(err);
        }
    };
    let estimate = estimate_usage(&request);
    if let Err(err) = state.control.check_quotas(&identity, estimate) {
        state
            .metrics
            .record_route("messages", false, started.elapsed());
        return Err(err);
    }

    let requested_model = request.model.clone();
    let config = effective_config(&state);
    let resolved = match config.resolve(&request.model) {
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
    let protocol = provider_protocol_value(resolved.provider.protocol).to_owned();
    let result = match resolved.provider.protocol {
        ProviderProtocol::Anthropic => {
            providers::anthropic::messages(state.clone(), resolved, request, &headers)
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
    let status_code = result
        .as_ref()
        .map(|response| response.status().as_u16())
        .unwrap_or(500);
    let error_message = result.as_ref().err().map(ToString::to_string);
    let actual_estimate = result
        .as_ref()
        .ok()
        .and_then(|response| pricing::usage_from_headers(response.headers()))
        .map(|charge| UsageEstimate {
            input_tokens: charge.input_tokens,
            output_tokens: charge.output_tokens,
            cache_write_tokens: charge.cache_write_tokens,
            cache_read_tokens: charge.cache_read_tokens,
            cost_estimate: charge.cost_estimate,
        })
        .unwrap_or(estimate);

    state.metrics.record_route("messages", success, duration);
    state
        .metrics
        .record_message(&provider_id, &upstream_model, stream, success, duration);
    state.control.record_usage(UsageEventInput {
        identity,
        model: requested_model,
        resolved_model: upstream_model,
        provider: provider_id,
        protocol,
        stream,
        success,
        status_code,
        estimate: actual_estimate,
        latency: duration,
        first_byte_latency: Some(duration),
        retry_count: 0,
        client_ip: client_ip(&headers),
        request_path: "/v1/messages".to_owned(),
        error_message,
    })?;
    result
}

async fn admin_login(
    State(state): State<AppState>,
    Json(input): Json<LoginInput>,
) -> Result<Response, AppError> {
    let login = state.auth.login(input)?;
    record_admin_activity(
        &state,
        &login.user,
        "config_change",
        format!("user:{}", login.user.id),
        format!("管理员 {} 登录控制台", login.user.username),
        "info",
    );
    let mut response = Json(json!({
        "user": login.user,
        "expiresAt": login.expires_at_ms.to_string(),
    }))
    .into_response();
    let cookie = state.auth.session_cookie(&login.session_token);
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|err| AppError::Config(format!("invalid admin session cookie: {err}")))?,
    );
    Ok(response)
}

async fn admin_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    state.auth.logout(&headers);
    let mut response = Json(json!({ "ok": true })).into_response();
    let cookie = state.auth.clear_cookie();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|err| AppError::Config(format!("invalid admin session cookie: {err}")))?,
    );
    Ok(response)
}

async fn admin_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let user = state.auth.require_session(&headers)?;
    Ok(Json(json!(user)))
}

fn require_admin_user(state: &AppState, headers: &HeaderMap) -> Result<PublicUser, AppError> {
    let user = state.auth.require_session(headers)?;
    if user.role == "admin" {
        Ok(user)
    } else {
        Err(AppError::Forbidden("admin role required".to_owned()))
    }
}

fn record_admin_activity(
    state: &AppState,
    actor: &PublicUser,
    activity_type: &str,
    target: impl Into<String>,
    message: impl Into<String>,
    severity: &str,
) {
    if let Err(err) = state.control.record_activity(ActivityInput {
        activity_type: activity_type.to_owned(),
        actor: actor.username.clone(),
        target: target.into(),
        message: message.into(),
        severity: severity.to_owned(),
    }) {
        warn!(error = %err, "failed to record admin activity");
    }
}

fn authenticate_client(state: &AppState, headers: &HeaderMap) -> Result<ClientIdentity, AppError> {
    if let Some(identity) = state.control.authenticate_headers(headers)? {
        return Ok(identity);
    }
    state.config.validate_client_auth(headers)?;
    Ok(ControlStore::legacy_identity())
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    for name in ["x-forwarded-for", "x-real-ip", "cf-connecting-ip"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            let ip = value.split(',').next().unwrap_or(value).trim();
            if !ip.is_empty() {
                return Some(ip.to_owned());
            }
        }
    }
    None
}

fn estimate_usage(request: &AnthropicRequest) -> UsageEstimate {
    let input_chars = serde_json::to_string(&request.messages)
        .map(|value| value.chars().count())
        .unwrap_or(0)
        + request
            .system
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok())
            .map(|value| value.chars().count())
            .unwrap_or(0);
    let input_tokens = u64::try_from(input_chars.div_ceil(4)).unwrap_or(u64::MAX);
    let output_tokens = request.max_tokens.unwrap_or(0);
    UsageEstimate {
        input_tokens,
        output_tokens,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        cost_estimate: pricing::cost_for_model(
            &request.model,
            TokenUsageBreakdown {
                input_tokens,
                output_tokens,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
    }
}

async fn admin_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;

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
    let active_users = state.auth.active_user_count();
    let usage_summary = state.control.usage_summary_today();
    let (usage_request_series, usage_error_series) = state.control.usage_time_series_24h();
    let usage_series_has_data = usage_request_series
        .iter()
        .any(|point| point.get("value").and_then(Value::as_u64).unwrap_or(0) > 0);
    let metric_top_models = snapshot
        .messages
        .iter()
        .map(|message| {
            json!({
                "model": message.model,
                "provider": message.provider,
                "requests": message.requests_total,
            })
        })
        .collect::<Vec<_>>();
    let usage_top_models = state.control.usage_top_models_today(8);
    let persisted_provider_usage = state.control.provider_usage_today();
    let now = now_millis_string();
    let recent_activity = state.control.activity_rows(8);
    let fallback_activity = vec![
        json!({
            "id": "act_health",
            "timestamp": now.clone(),
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
    ];

    Ok(Json(json!({
        "uptimeSeconds": snapshot.uptime_seconds,
        "totalRequests": total_requests,
        "successRate": success_rate,
        "activeProviders": active_providers,
        "totalProviders": providers.len(),
        "activeUsers": active_users,
        "totalModels": state.config.model_list().len(),
        "avgLatencyMs": avg_latency_ms,
        "apiKeysTotal": usage_summary.api_keys_total,
        "apiKeysActive": usage_summary.api_keys_active,
        "todayRequests": usage_summary.total_requests,
        "todayInputTokens": usage_summary.total_input_tokens,
        "todayOutputTokens": usage_summary.total_output_tokens,
        "todayCacheWriteTokens": usage_summary.total_cache_write_tokens,
        "todayCacheReadTokens": usage_summary.total_cache_read_tokens,
        "todayCostEstimate": usage_summary.total_cost_estimate,
        "requestTimeSeries": if usage_series_has_data { usage_request_series } else { time_series(total_requests) },
        "errorTimeSeries": if usage_series_has_data { usage_error_series } else { time_series(total_failures) },
        "topModels": if metric_top_models.is_empty() { usage_top_models } else { metric_top_models },
        "providerHealth": providers.iter().map(|provider| {
            let id = provider.get("id").and_then(Value::as_str).unwrap_or("");
            let provider_messages = snapshot.messages.iter().filter(|message| message.provider == id).collect::<Vec<_>>();
            let metric_requests = provider_messages.iter().map(|message| message.requests_total).sum::<u64>();
            let metric_successes = provider_messages.iter().map(|message| message.successes_total).sum::<u64>();
            let metric_duration = provider_messages.iter().map(|message| message.duration_ms_total).sum::<u64>();
            let persisted = persisted_provider_usage.get(id);
            let requests = if metric_requests > 0 { metric_requests } else { persisted.map(|stats| stats.requests_total).unwrap_or(0) };
            let successes = if metric_requests > 0 { metric_successes } else { persisted.map(|stats| stats.successes_total).unwrap_or(0) };
            let duration = if metric_requests > 0 { metric_duration } else { persisted.map(|stats| stats.duration_ms_total).unwrap_or(0) };
            let success_rate = percent(successes, requests);
            let provider_status = provider.get("status").and_then(Value::as_str).unwrap_or("inactive");
            let health_status = if provider_status != "active" {
                "down"
            } else if requests > 0 && success_rate < 99.0 {
                "degraded"
            } else {
                "healthy"
            };
            json!({
                "providerId": id,
                "displayName": provider.get("displayName").cloned().unwrap_or_else(|| json!(id)),
                "status": health_status,
                "requestsTotal": requests,
                "successRate": success_rate,
                "avgLatencyMs": average(duration, requests),
            })
        }).collect::<Vec<_>>(),
        "recentActivity": if recent_activity.is_empty() { fallback_activity } else { recent_activity },
    })))
}

async fn admin_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(Value::Array(provider_rows(&state))))
}

async fn admin_aliases(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(Value::Array(alias_rows(&state))))
}

async fn admin_create_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let alias = body.get("alias").and_then(Value::as_str).unwrap_or("");
    let target = body.get("target").and_then(Value::as_str).unwrap_or("");
    validate_alias_target(&state, alias, target)?;
    state
        .control
        .upsert_alias(alias.to_owned(), target.to_owned())?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("alias:{alias}"),
        format!("创建模型别名 {alias} -> {target}"),
        "info",
    );
    let config = effective_config(&state);
    Ok(Json(alias_row(&config, alias, target)))
}

async fn admin_delete_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let tombstone = state.config.aliases.contains_key(&alias);
    state.control.delete_alias(&alias, tombstone)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("alias:{alias}"),
        format!("删除模型别名 {alias}"),
        "warning",
    );
    Ok(Json(json!({ "ok": true })))
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(settings_row(&state)))
}

async fn admin_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let mut changes = Vec::new();
    if let Some(gateway) = body.get("gateway") {
        if let Some(provider_id) = gateway.get("defaultProvider").and_then(Value::as_str) {
            if !state.config.providers.contains_key(provider_id) {
                return Err(AppError::ProviderNotFound(provider_id.to_owned()));
            }
            state.control.set_default_provider(provider_id.to_owned())?;
            changes.push(format!("默认供应商设为 {provider_id}"));
        }

        if let Some(order) = gateway.get("providerOrder") {
            let provider_order = parse_provider_order(&state.config, order)?;
            let provider_count = provider_order.len();
            state.control.set_provider_order(provider_order)?;
            changes.push(format!("供应商路由顺序更新为 {provider_count} 个节点"));
        }
    }

    if !changes.is_empty() {
        record_admin_activity(
            &state,
            &actor,
            "config_change",
            "gateway",
            changes.join("；"),
            "info",
        );
    }

    Ok(Json(settings_row(&state)))
}

async fn admin_test_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let provider_id = body
        .get("providerId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let config = effective_config(&state);
    let Some(provider) = config.providers.get(provider_id).cloned() else {
        return Ok(Json(json!({
            "success": false,
            "message": "provider not found",
        })));
    };

    let (success, message, models) = match discover_provider_models(&state, &provider).await {
        Ok(models) => {
            let message = if provider.protocol == ProviderProtocol::OpenaiCompat {
                format!("connected; discovered {} model(s)", models.len())
            } else {
                "configured".to_owned()
            };
            (true, message, models)
        }
        Err(err) => (false, err.to_string(), Vec::new()),
    };
    let tested_at = state.control.record_provider_test(
        provider_id.to_owned(),
        success,
        message.to_owned(),
        models.clone(),
    )?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}"),
        format!("测试供应商 {provider_id}: {message}"),
        if success { "info" } else { "warning" },
    );

    Ok(Json(json!({
        "success": success,
        "message": message,
        "models": models,
        "modelCount": models.len(),
        "testedAt": tested_at.to_string(),
    })))
}

async fn admin_provider_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let config = effective_config(&state);
    let Some(provider) = config.providers.get(&provider_id).cloned() else {
        return Err(AppError::ProviderNotFound(provider_id));
    };

    let (success, message, models) = match discover_provider_models(&state, &provider).await {
        Ok(models) => {
            let message = if provider.protocol == ProviderProtocol::OpenaiCompat {
                format!("discovered {} model(s)", models.len())
            } else {
                "model discovery is not available for this protocol; returned configured models"
                    .to_owned()
            };
            (true, message, models)
        }
        Err(err) => (false, err.to_string(), Vec::new()),
    };
    let tested_at = state.control.record_provider_test(
        provider_id.clone(),
        success,
        message.clone(),
        models.clone(),
    )?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}"),
        format!("发现供应商 {provider_id} 模型: {message}"),
        if success { "info" } else { "warning" },
    );

    Ok(Json(json!({
        "providerId": provider_id,
        "success": success,
        "message": message,
        "models": models,
        "modelCount": models.len(),
        "discoveredAt": tested_at.to_string(),
    })))
}

async fn discover_provider_models(
    state: &AppState,
    provider: &ProviderConfig,
) -> Result<Vec<String>, AppError> {
    if provider.protocol != ProviderProtocol::OpenaiCompat {
        provider.api_key()?;
        return Ok(configured_provider_models(provider));
    }

    let url = provider.endpoint("/models");
    let body = state
        .transport
        .get_json(&url, &openai_compatible_headers(provider)?)
        .await?;
    let models = parse_model_ids(&body);

    if models.is_empty() {
        Ok(configured_provider_models(provider))
    } else {
        Ok(models)
    }
}

fn openai_compatible_headers(provider: &ProviderConfig) -> Result<Vec<Header>, AppError> {
    let mut headers = Vec::new();
    if let Some(api_key) = provider.api_key()? {
        headers.push(("Authorization".to_owned(), format!("Bearer {api_key}")));
    }
    Ok(headers)
}

fn configured_provider_models(provider: &ProviderConfig) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();

    for model in provider
        .models
        .iter()
        .chain(std::iter::once(&provider.default_model))
    {
        push_model_id(model, &mut models, &mut seen);
    }

    models
}

fn parse_model_ids(value: &Value) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    let root = value
        .get("data")
        .or_else(|| value.get("models"))
        .unwrap_or(value);

    collect_model_ids(root, &mut models, &mut seen);
    models
}

fn collect_model_ids(value: &Value, models: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_model_ids(item, models, seen);
            }
        }
        Value::Object(map) => {
            if let Some(id) = map
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| map.get("name").and_then(Value::as_str))
                .or_else(|| map.get("model").and_then(Value::as_str))
            {
                push_model_id(id, models, seen);
                return;
            }

            for key in ["data", "models"] {
                if let Some(nested) = map.get(key) {
                    collect_model_ids(nested, models, seen);
                }
            }
        }
        Value::String(id) => push_model_id(id, models, seen),
        _ => {}
    }
}

fn push_model_id(id: &str, models: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    let id = id.trim();
    if !id.is_empty() && seen.insert(id.to_owned()) {
        models.push(id.to_owned());
    }
}

async fn admin_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    let mut logs = state.control.usage_rows();
    if logs.is_empty() {
        logs = log_rows(&state);
    }
    Ok(Json(json!({
        "logs": logs,
        "total": logs.len(),
    })))
}

async fn admin_latency(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
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
    require_admin_user(&state, &headers)?;
    let requests = state
        .metrics
        .snapshot()
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    let mut users = state.auth.list_users(requests);
    for user in &mut users {
        user.api_key_count = state.control.active_api_key_count(&user.id);
    }
    Ok(Json(json!(users)))
}

async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateUserInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let user = state.auth.create_user(body)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("user:{}", user.id),
        format!("创建用户 {}", user.username),
        "info",
    );
    Ok(Json(json!(user)))
}

async fn admin_update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<UpdateUserInput>,
) -> Result<Json<Value>, AppError> {
    let current_user = require_admin_user(&state, &headers)?;
    let user = state.auth.update_user(&user_id, &current_user.id, body)?;
    record_admin_activity(
        &state,
        &current_user,
        "config_change",
        format!("user:{user_id}"),
        format!("更新用户 {} ({})", user.username, user.role),
        "info",
    );
    Ok(Json(json!(user)))
}

async fn admin_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let current_user = require_admin_user(&state, &headers)?;
    state.auth.delete_user(&user_id, &current_user.id)?;
    state.control.delete_user_resources(&user_id)?;
    record_admin_activity(
        &state,
        &current_user,
        "config_change",
        format!("user:{user_id}"),
        format!("删除用户 {user_id} 并回收相关资源"),
        "warning",
    );
    Ok(Json(json!({ "ok": true })))
}

async fn admin_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(json!(state.control.list_api_keys())))
}

async fn admin_user_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(json!(state.control.list_user_api_keys(&user_id))))
}

async fn admin_create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<CreateApiKeyInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    if body.username.is_none()
        && let Some(user) = state
            .auth
            .list_users(0)
            .into_iter()
            .find(|user| user.id == body.user_id)
    {
        body.username = Some(user.username);
    }
    let created = state.control.create_api_key(body)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("api_key:{}", created.public.id),
        format!(
            "为用户 {} 创建 API Key {}",
            created.public.username, created.public.name
        ),
        "info",
    );
    Ok(Json(json!(created)))
}

async fn admin_revoke_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    state.control.revoke_api_key(&key_id)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("api_key:{key_id}"),
        format!("吊销 API Key {key_id}"),
        "warning",
    );
    Ok(Json(json!({ "ok": true })))
}

async fn admin_update_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(body): Json<UpdateApiKeyInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let updated = state.control.update_api_key(&key_id, body)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("api_key:{key_id}"),
        format!("更新 API Key {} ({})", updated.name, updated.status),
        "info",
    );
    Ok(Json(json!(updated)))
}

async fn admin_delete_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    state.control.delete_api_key(&key_id)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("api_key:{key_id}"),
        format!("删除 API Key {key_id}"),
        "warning",
    );
    Ok(Json(json!({ "ok": true })))
}

async fn admin_quotas(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(json!(state.control.list_quotas()?)))
}

async fn admin_create_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertQuotaInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    let quota = state.control.upsert_quota(body)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("quota:{}", quota.id),
        format!(
            "为用户 {} 配置 {} 配额 {} / {}",
            quota.username, quota.quota_type, quota.limit, quota.period
        ),
        "info",
    );
    Ok(Json(json!(quota)))
}

async fn admin_update_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(quota_id): Path<String>,
    Json(mut body): Json<UpsertQuotaInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    body.id = Some(quota_id);
    let quota = state.control.upsert_quota(body)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("quota:{}", quota.id),
        format!(
            "更新用户 {} 的 {} 配额为 {} / {}",
            quota.username, quota.quota_type, quota.limit, quota.period
        ),
        "info",
    );
    Ok(Json(json!(quota)))
}

async fn admin_delete_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(quota_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    state.control.delete_quota(&quota_id)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("quota:{quota_id}"),
        format!("删除配额 {quota_id}"),
        "warning",
    );
    Ok(Json(json!({ "ok": true })))
}

fn provider_rows(state: &AppState) -> Vec<Value> {
    let config = effective_config(state);
    let provider_tests = state.control.provider_test_rows();
    config
        .provider_order
        .iter()
        .filter_map(|id| {
            let provider = config.providers.get(id)?;
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
                "fidelityMode": fidelity_mode_value(provider.fidelity_mode),
                "status": if has_api_key || !provider.api_key_required { "active" } else { "inactive" },
                "hasApiKey": has_api_key,
                "lastTest": provider_tests.get(id).cloned(),
            }))
        })
        .collect()
}

fn alias_rows(state: &AppState) -> Vec<Value> {
    let config = effective_config(state);
    config
        .aliases
        .iter()
        .map(|(alias, target)| alias_row(&config, alias, target))
        .collect()
}

fn alias_row(config: &AppConfig, alias: &str, target: &str) -> Value {
    let resolved = config.resolve(alias).ok();
    json!({
        "alias": alias,
        "target": target,
        "resolvedProvider": resolved.as_ref().map(|value| value.provider_id.as_str()).unwrap_or(""),
        "resolvedModel": resolved.as_ref().map(|value| value.model.as_str()).unwrap_or(""),
    })
}

fn settings_row(state: &AppState) -> Value {
    let config = effective_config(state);
    json!({
        "server": {
            "bindAddress": config.bind_addr.to_string(),
            "maxRequestBodyBytes": config.max_request_body_bytes,
            "maxConcurrentRequests": config.max_concurrent_requests,
        },
        "auth": {
            "enabled": config.auth_token.is_some(),
            "tokenEnvVar": "MODELPORT_AUTH_TOKEN",
            "allowNoAuth": config.auth_token.is_none(),
        },
        "gateway": {
            "defaultProvider": config.default_provider,
            "providerOrder": config.provider_order,
        },
        "rateLimits": {
            "maxConcurrentRequests": config.max_concurrent_requests,
            "maxRequestBodyBytes": config.max_request_body_bytes,
            "requestTimeoutSecs": env_u64("MODELPORT_HTTP_REQUEST_TIMEOUT_SECS", 600),
            "streamIdleTimeoutSecs": env_u64("MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS", 300),
        },
    })
}

fn effective_config(state: &AppState) -> AppConfig {
    let mut config = state.config.as_ref().clone();
    let snapshot = state.control.routing_config();
    config.aliases = state.control.effective_aliases(&config.aliases);

    if let Some(provider_id) = snapshot.default_provider
        && config.providers.contains_key(&provider_id)
    {
        config.default_provider = provider_id;
    }

    if let Some(provider_order) = snapshot.provider_order {
        let filtered = provider_order
            .into_iter()
            .filter(|provider_id| config.providers.contains_key(provider_id))
            .collect::<Vec<_>>();
        if !filtered.is_empty() {
            config.provider_order = filtered;
        }
    }

    config
}

fn validate_alias_target(state: &AppState, alias: &str, target: &str) -> Result<(), AppError> {
    let alias = alias.trim();
    let target = target.trim();
    if alias.is_empty() || target.is_empty() {
        return Err(AppError::InvalidRequest(
            "alias and target are required".to_owned(),
        ));
    }

    let mut config = effective_config(state);
    config.aliases.insert(alias.to_owned(), target.to_owned());
    config.resolve(alias)?;
    Ok(())
}

fn parse_provider_order(config: &AppConfig, value: &Value) -> Result<Vec<String>, AppError> {
    let Some(values) = value.as_array() else {
        return Err(AppError::InvalidRequest(
            "gateway.providerOrder must be an array".to_owned(),
        ));
    };

    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for value in values {
        let Some(provider_id) = value.as_str().map(str::trim) else {
            return Err(AppError::InvalidRequest(
                "gateway.providerOrder values must be strings".to_owned(),
            ));
        };
        if provider_id.is_empty() {
            continue;
        }
        if !config.providers.contains_key(provider_id) {
            return Err(AppError::ProviderNotFound(provider_id.to_owned()));
        }
        if seen.insert(provider_id.to_owned()) {
            order.push(provider_id.to_owned());
        }
    }

    if order.is_empty() {
        return Err(AppError::InvalidRequest(
            "gateway.providerOrder cannot be empty".to_owned(),
        ));
    }

    Ok(order)
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
                "apiKeyId": null,
                "apiKeyName": "MODELPORT_AUTH_TOKEN",
                "apiKeyGroup": "legacy",
                "tokenName": "MODELPORT_AUTH_TOKEN",
                "group": "legacy",
                "channelId": message.provider,
                "channelName": message.provider,
                "model": message.model,
                "resolvedModel": message.model,
                "provider": message.provider,
                "protocol": protocol,
                "requestType": if message.failures_total > 0 { "error" } else { "consume" },
                "stream": if message.stream { "stream" } else { "non-stream" },
                "status": if message.failures_total > 0 { "error" } else { "success" },
                "statusCode": if message.failures_total > 0 { 502 } else { 200 },
                "inputTokens": 0,
                "outputTokens": 0,
                "cacheWriteTokens": 0,
                "cacheReadTokens": 0,
                "billedInputTokens": 0,
                "totalTokens": 0,
                "cacheHitRate": 0.0,
                "costEstimate": 0.0,
                "costBreakdown": {
                    "inputCost": 0.0,
                    "outputCost": 0.0,
                    "cacheWriteCost": 0.0,
                    "cacheReadCost": 0.0,
                    "totalCost": 0.0,
                },
                "latencyMs": average(message.duration_ms_total, requests),
                "firstByteLatencyMs": average(message.duration_ms_total, requests),
                "retryCount": 0,
                "clientIp": null,
                "requestPath": "/v1/messages",
                "billingMode": "metrics-fallback",
                "detail": format!("进程内指标回退日志 · provider={} · model={}", message.provider, message.model),
                "errorMessage": if message.failures_total > 0 { Some(format!("{} failure(s) recorded", message.failures_total)) } else { None },
                "sortIndex": index,
            })
        })
        .collect()
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

fn fidelity_mode_value(mode: crate::config::FidelityMode) -> &'static str {
    match mode {
        crate::config::FidelityMode::Strict => "strict",
        crate::config::FidelityMode::BestEffort => "best_effort",
        crate::config::FidelityMode::Stability => "stability",
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
    total.checked_div(count).unwrap_or(0)
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
            header::{CONTENT_TYPE, COOKIE, HeaderValue, SET_COOKIE},
        },
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        auth::{AuthStore, CreateUserInput},
        config::{FidelityMode, MaxTokensField, ProviderConfig},
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

    #[tokio::test]
    async fn admin_dashboard_requires_admin_session_not_router_token() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state_with_admin(upstream, 1024 * 1024));

        let token_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/dashboard")
                    .header("x-api-key", CLIENT_TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(token_response.status(), StatusCode::UNAUTHORIZED);

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/auth/login")
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(
                        json!({
                            "username": "admin",
                            "password": "strong-password-123",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login_response.status(), StatusCode::OK);
        let session_cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .expect("login should set a session cookie")
            .clone();

        let dashboard_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/dashboard")
                    .header(COOKIE, session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(dashboard_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn admin_alias_updates_runtime_message_routing() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","model":"mimo-v2.5-pro","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
            "application/json",
        )
        .await;
        let app = router(test_state_with_admin(upstream, 1024 * 1024));

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/auth/login")
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(
                        json!({
                            "username": "admin",
                            "password": "strong-password-123",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login_response.status(), StatusCode::OK);
        let session_cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .expect("login should set a session cookie")
            .clone();

        let alias_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/aliases")
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .header(COOKIE, session_cookie)
                    .body(Body::from(
                        json!({
                            "alias": "fast",
                            "target": "mimo:mimo-v2.5-pro",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(alias_response.status(), StatusCode::OK);

        let (message_status, _) = post_message(
            app.clone(),
            json!({
                "model": "fast",
                "max_tokens": 32,
                "messages": [
                    {
                        "role": "user",
                        "content": "hello"
                    }
                ]
            }),
        )
        .await;
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
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains(
            r#"modelport_message_requests_total{provider="mimo",model="mimo-v2.5-pro",stream="false"} 1"#
        ));
    }

    #[test]
    fn parse_model_ids_accepts_common_local_runtime_shapes() {
        assert_eq!(
            parse_model_ids(&json!({
                "data": [
                    { "id": "qwen2.5-coder-ft" },
                    { "id": "qwen2.5-coder-ft" },
                    { "name": "deepseek-coder-lora" }
                ]
            })),
            vec!["qwen2.5-coder-ft", "deepseek-coder-lora"]
        );

        assert_eq!(
            parse_model_ids(&json!({
                "models": [
                    "local-model",
                    { "model": "my-org/my-code-model" }
                ]
            })),
            vec!["local-model", "my-org/my-code-model"]
        );
    }

    #[tokio::test]
    async fn discover_anthropic_models_checks_required_api_key() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let provider = ProviderConfig {
            display_name: "Anthropic".to_owned(),
            protocol: ProviderProtocol::Anthropic,
            base_url: "https://api.anthropic.com".to_owned(),
            api_key_env: Some("ANTHROPIC_API_KEY".to_owned()),
            api_key: None,
            api_key_required: true,
            default_model: "claude-sonnet-4-6".to_owned(),
            models: vec!["claude-sonnet-4-6".to_owned()],
            model_prefixes: vec!["claude-".to_owned()],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::Strict,
        };

        let err = discover_provider_models(&state, &provider)
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::MissingSecret(name) if name == "ANTHROPIC_API_KEY"));
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

    fn test_state_with_admin(base_url: String, max_request_body_bytes: usize) -> AppState {
        let state = test_state(base_url, max_request_body_bytes);
        state
            .auth
            .create_user(CreateUserInput {
                username: "admin".to_owned(),
                email: "admin@modelport.local".to_owned(),
                password: "strong-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        state
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
            fidelity_mode: if deduplicate_stream_text || buffer_stream_text {
                FidelityMode::Stability
            } else {
                FidelityMode::BestEffort
            },
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
            auth: Arc::new(AuthStore::for_tests()),
            control: Arc::new(ControlStore::for_tests()),
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
