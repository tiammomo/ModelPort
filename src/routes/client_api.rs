use std::{
    collections::{BTreeSet, VecDeque},
    net::SocketAddr,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{State, connect_info::ConnectInfo},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use rand_core::{OsRng, RngCore};
use serde_json::{Value, json};
use tracing::{error, info};

use crate::{
    config::{
        AppConfig, ProviderConfig, ProviderProtocol, ResolvedProvider, RoutingProfile,
        SmartRoutingMode, TokenCountingConfig, TokenCountingMode,
    },
    control::{UsageEstimate, UsageEventInput},
    domain::{AttemptId, RequestContext, RequestId},
    enterprise_ledger::{
        AttemptPricingEvidence, LedgerAttempt, LedgerLease, LedgerOutcome, LedgerRequest,
        LedgerRequestMetadata,
    },
    exchange::{ClientRequest, ExchangeRequest, OpenAiChatRequest},
    governance::{
        DataClassification, HybridMode, LocalAdmission, LocalLease, ProviderBoundary,
        WorkloadClass, order_attempts,
    },
    metrics::MessageMetricLabels,
    pricing::{self, ModelPricing, TokenUsageBreakdown},
    providers,
    smart_router::{RoutingRequest, hash_session_key},
    stream_lifecycle::{
        ResponseObservation, StreamLifecycle, StreamTerminalOutcome, UpstreamStreamState,
        audit_safe_stream_error,
    },
    types::{AnthropicCountTokensRequest, AnthropicRequest, validate_anthropic_tooling},
};

use super::*;

#[cfg(test)]
use super::route_contract::RouteContract;

#[cfg(test)]
pub(super) const ROUTES: &[RouteContract] = &[
    RouteContract::new("client-api", "/v1/models", &["GET"]),
    RouteContract::new("client-api", "/v1/messages", &["POST"]),
    RouteContract::new("client-api", "/v1/messages/count_tokens", &["POST"]),
    RouteContract::new("client-api", "/v1/chat/completions", &["POST"]),
    RouteContract::new("client-api", "/v1/effective-policy", &["GET"]),
];

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/effective-policy", get(effective_policy))
}

pub(super) async fn models(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let started = Instant::now();
    let result = (|| {
        let identity = authenticate_inference_client(&state, &headers)?;
        let request_client_ip = client_ip(&headers, Some(peer_addr), &state.trusted_proxies);
        identity
            .api_key_policy
            .enforce_client_ip(request_client_ip.as_deref())?;
        let tenant = state.control.tenant_scope(&identity)?;
        let policy = state.governance.effective_policy(&tenant);
        let config = effective_config(&state);
        let api_key_policy = identity
            .api_key_id
            .as_ref()
            .map(|_| &identity.api_key_policy);
        let data = catalog_model_rows(&config)
            .into_iter()
            .filter(|row| {
                let Some(model) = row.get("id").and_then(Value::as_str) else {
                    return false;
                };
                if config.smart_route_group(model).is_some() {
                    return !super::provider_view::catalog_smart_alias_candidates(
                        &state,
                        &config,
                        model,
                        api_key_policy,
                        &policy,
                    )
                    .is_empty();
                }
                config.resolve(model).is_ok_and(|resolved| {
                    super::provider_view::catalog_candidate_is_allowed(
                        &state,
                        api_key_policy,
                        &policy,
                        model,
                        &resolved,
                    )
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

pub(super) async fn effective_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let identity = authenticate_inference_client(&state, &headers)?;
    let tenant = state.control.tenant_scope(&identity)?;
    Ok(Json(json!({
        "principalType": identity.principal_type,
        "tenant": {
            "organizationId": tenant.organization_id.as_str(),
            "projectId": tenant.project_id.as_str(),
            "environmentId": tenant.environment_id.as_str(),
        },
        "policy": state.governance.effective_policy(&tenant),
        "scheduler": state.local_scheduler.snapshot(),
    })))
}

#[cfg(test)]
pub(super) fn public_model_rows(config: &AppConfig) -> Vec<Value> {
    build_public_model_rows(config, true)
}

fn catalog_model_rows(config: &AppConfig) -> Vec<Value> {
    build_public_model_rows(config, false)
}

fn build_public_model_rows(config: &AppConfig, require_static_credential: bool) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();

    for id in &config.provider_order {
        let Some(provider) = config.providers.get(id) else {
            continue;
        };
        if require_static_credential && !provider_is_configured(provider) {
            continue;
        }

        for model in &provider.models {
            let public_id = format!("{id}:{model}");
            if seen.insert(public_id.clone()) {
                models.push(json!({
                    "id": public_id,
                    "type": "model",
                    "owned_by": id,
                    "display_name": public_model_display_name(id, provider, model),
                }));
            }
        }
    }

    for alias in config.aliases.keys() {
        if seen.contains(alias) {
            continue;
        }
        let Ok(resolved) = config.resolve(alias) else {
            continue;
        };
        if require_static_credential && !provider_is_configured(&resolved.provider) {
            continue;
        }
        if seen.insert(alias.clone()) {
            models.push(json!({
                "id": alias,
                "type": "model",
                "owned_by": resolved.provider_id,
                "display_name": public_model_display_name(&resolved.provider_id, &resolved.provider, &resolved.model),
            }));
        }
    }

    if config.smart_routing.mode != SmartRoutingMode::Off {
        for group in config.smart_routing.groups.values() {
            for alias in &group.aliases {
                if seen.insert(alias.clone()) {
                    models.push(json!({
                        "id": alias,
                        "type": "model",
                        "owned_by": "modelport-router",
                        "display_name": format!("{} (Smart Router)", alias),
                    }));
                }
            }
        }
    }

    models
}

fn provider_is_configured(provider: &ProviderConfig) -> bool {
    !provider.api_key_required || provider.api_key().ok().flatten().is_some()
}

#[derive(Debug, Clone)]
struct SentAttempt {
    attempt_id: AttemptId,
    provider_id: String,
    model: String,
    protocol: String,
    boundary: ProviderBoundary,
    credential_id: Option<String>,
    estimate: UsageEstimate,
    pricing: ModelPricing,
    pricing_card: Option<pricing::ModelPricingCard>,
    trust_upstream_cost: bool,
    stream_lifecycle: StreamLifecycle,
    ledger_attempt: LedgerAttempt,
    started: Instant,
}

fn public_model_display_name(provider_id: &str, provider: &ProviderConfig, model: &str) -> String {
    format!(
        "{} · {}",
        provider_origin_label(provider_id, provider),
        model_owner_label(model)
    )
}

fn provider_origin_label(provider_id: &str, provider: &ProviderConfig) -> &'static str {
    let host = provider_host(&provider.base_url);
    if is_local_provider(provider_id, &host) {
        return "本地";
    }
    if provider_id == "custom" {
        return "自定义";
    }
    if provider_id == "openrouter" {
        return "聚合平台";
    }
    if official_provider_host(provider_id, &host) {
        return "官方";
    }
    "第三方"
}

fn model_owner_label(model: &str) -> &'static str {
    let value = model.to_ascii_lowercase();
    if value.starts_with("gpt-")
        || value.starts_with("o1")
        || value.starts_with("o3")
        || value.starts_with("o4")
        || value.starts_with("o5")
        || value.starts_with("chatgpt-")
        || value.starts_with("codex-")
        || value.contains("-codex")
        || value.starts_with("openai/")
    {
        return "OpenAI";
    }
    if value.contains("mimo") {
        return "小米 MiMo";
    }
    if value.contains("deepseek") {
        return "DeepSeek";
    }
    if value.contains("claude") || value.starts_with("anthropic/") {
        return "Anthropic Claude";
    }
    if value.contains("gemini") || value.starts_with("google/") {
        return "Google Gemini";
    }
    if value.contains("qwen") || value.starts_with("qwq-") || value.starts_with("qvq-") {
        return "Qwen";
    }
    if value.contains("kimi") || value.contains("moonshot") {
        return "Moonshot Kimi";
    }
    if value.starts_with("glm-") || value.contains("z-ai/") {
        return "智谱 GLM";
    }
    if value.contains("grok") || value.contains("x-ai/") {
        return "xAI Grok";
    }
    if value.contains("llama") || value.contains("meta-llama/") {
        return "Llama";
    }
    if value.contains("mistral") || value.contains("codestral") {
        return "Mistral AI";
    }
    if value.contains("doubao") {
        return "Doubao";
    }
    "自定义模型"
}

fn provider_host(base_url: &str) -> String {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}

fn is_local_provider(provider_id: &str, host: &str) -> bool {
    provider_id.starts_with("local_")
        || provider_id.starts_with("cpa_")
        || provider_id == "ollama"
        || matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1")
}

fn official_provider_host(provider_id: &str, host: &str) -> bool {
    let expected = match provider_id {
        "deepseek" | "deepseek_openai" => "api.deepseek.com",
        "mimo" => "api.xiaomimimo.com",
        "openai" => "api.openai.com",
        "anthropic" => "api.anthropic.com",
        "gemini" => "generativelanguage.googleapis.com",
        "dashscope" => "dashscope.aliyuncs.com",
        "kimi" => "api.moonshot.cn",
        "zhipu" => "open.bigmodel.cn",
        "xai" => "api.x.ai",
        "groq" => "api.groq.com",
        "mistral" => "api.mistral.ai",
        "ark" => "ark.cn-beijing.volces.com",
        _ => return false,
    };
    host == expected || host.ends_with(&format!(".{expected}"))
}

pub(super) async fn messages(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<AnthropicRequest>,
) -> Result<Response, AppError> {
    handle_inference(state, peer_addr, headers, ClientRequest::Anthropic(request)).await
}

pub(super) async fn count_tokens(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<AnthropicCountTokensRequest>,
) -> Result<Json<Value>, AppError> {
    let started = Instant::now();
    let result = count_tokens_inner(state.clone(), peer_addr, &headers, request).await;
    state
        .metrics
        .record_route("count_tokens", result.is_ok(), started.elapsed());
    result
}

async fn count_tokens_inner(
    state: AppState,
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    request: AnthropicCountTokensRequest,
) -> Result<Json<Value>, AppError> {
    ensure_accepting_inference(&state)?;
    let identity = authenticate_inference_client(&state, headers)?;
    validate_count_tokens_request(&request)?;
    let requested_model = request.model.clone();
    let config = effective_config(&state);
    let mut resolved = config.resolve(&requested_model)?;
    let tenant = state.control.tenant_scope(&identity)?;
    let policy = state.governance.effective_policy(&tenant);
    let classification = request_routing_header(
        headers,
        &DATA_CLASSIFICATION,
        "x-modelport-data-classification",
        32,
    )?
    .map(|value| {
        DataClassification::parse(&value).ok_or_else(|| {
            AppError::InvalidRequest(
                "x-modelport-data-classification must be unknown, sensitive, internal, or public"
                    .to_owned(),
            )
        })
    })
    .transpose()?
    .unwrap_or(policy.default_classification);
    let requested_mode = request_routing_header(
        headers,
        &HYBRID_MODE,
        "x-modelport-hybrid-mode",
        32,
    )?
    .map(|value| {
        HybridMode::parse(&value).ok_or_else(|| {
            AppError::InvalidRequest(
                "x-modelport-hybrid-mode must be local_strict, local_first, balanced, or cloud_first"
                    .to_owned(),
            )
        })
    })
    .transpose()?;
    let mode = policy.effective_mode(requested_mode, classification)?;
    policy.enforce_attempt(&resolved)?;
    if mode == HybridMode::LocalStrict
        && ProviderBoundary::for_resolved(&resolved) == ProviderBoundary::Cloud
    {
        return Err(AppError::Forbidden(
            "local_strict token counting cannot use a cloud provider".to_owned(),
        ));
    }
    if resolved.provider.token_counting.mode != TokenCountingMode::Anthropic {
        return Err(AppError::InvalidRequest(format!(
            "provider `{}` does not enable Anthropic token counting",
            resolved.provider_id
        )));
    }

    let request_client_ip = client_ip(headers, Some(peer_addr), &state.trusted_proxies);
    state.rate_limiter.check(RateLimitScope {
        identity: &identity,
        client_ip: request_client_ip.as_deref(),
        provider_id: None,
        model: None,
    })?;
    let usage_policy = state.control.check_quotas(
        &identity,
        request_client_ip.as_deref(),
        &requested_model,
        &resolved.model,
        &resolved.provider_id,
    )?;
    state
        .ledger
        .check_usage_policy(&usage_policy, UsageEstimate::default(), true)
        .await?;
    state
        .control
        .apply_selected_provider_credential_for_request(
            &resolved.provider_id,
            &mut resolved.provider,
        )?;
    crate::config::validate_provider_base_url_dns_for_request(
        &resolved.provider_id,
        &resolved.provider.base_url,
        state.security.allow_private_provider_urls,
    )
    .await?;
    if resolved.provider.api_key_required {
        let _ = resolved.provider.api_key()?;
    }
    state
        .rate_limiter
        .check_provider_attempt(&resolved.provider_id, &resolved.model)?;

    providers::token_counting::count_tokens(state, resolved, request, headers).await
}

pub(super) async fn chat_completions(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<OpenAiChatRequest>,
) -> Result<Response, AppError> {
    handle_inference(
        state,
        peer_addr,
        headers,
        ClientRequest::OpenAiChat(request),
    )
    .await
}

async fn handle_inference(
    state: AppState,
    peer_addr: SocketAddr,
    headers: HeaderMap,
    request: ClientRequest,
) -> Result<Response, AppError> {
    let started = Instant::now();
    let route_name = request.route_name();
    if let Err(error) = ensure_accepting_inference(&state) {
        return Err(record_inference_rejection(
            &state, route_name, "draining", error, started,
        ));
    }
    let identity = match authenticate_inference_client(&state, &headers) {
        Ok(identity) => identity,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "authentication",
                err,
                started,
            ));
        }
    };
    let validation = match &request {
        ClientRequest::Anthropic(request) => validate_message_request(request),
        ClientRequest::OpenAiChat(request) => validate_openai_chat_request(request),
    };
    if let Err(err) = validation {
        return Err(record_inference_rejection(
            &state,
            route_name,
            "validation",
            err,
            started,
        ));
    }
    let exchange = match ExchangeRequest::from_client(request) {
        Ok(exchange) => exchange,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "validation",
                err,
                started,
            ));
        }
    };
    let idempotency_key = match request_idempotency_key(&headers) {
        Ok(key) => key,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "validation",
                err,
                started,
            ));
        }
    };
    let request_fingerprint = match exchange.request_fingerprint() {
        Ok(fingerprint) => fingerprint,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "validation",
                err,
                started,
            ));
        }
    };
    let request_client_ip = client_ip(&headers, Some(peer_addr), &state.trusted_proxies);
    let traffic_class = match request_traffic_class(&headers) {
        Ok(traffic_class) => traffic_class,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "validation",
                err,
                started,
            ));
        }
    };
    let bound_tenant = match state.control.tenant_scope(&identity) {
        Ok(tenant) => tenant,
        Err(err) => {
            return Err(record_inference_rejection(
                &state, route_name, "identity", err, started,
            ));
        }
    };
    let tenant = match request_tenant_scope(&headers, &bound_tenant) {
        Ok(tenant) => tenant,
        Err(err) => {
            return Err(record_inference_rejection(
                &state, route_name, "identity", err, started,
            ));
        }
    };
    let request_context = RequestContext::scoped(
        RequestId::from_external_or_new(
            headers
                .get(&X_REQUEST_ID)
                .and_then(|value| value.to_str().ok()),
        ),
        tenant,
        identity.user_id.clone(),
        exchange.client_protocol(),
    );
    let project_policy = state.governance.effective_policy(&request_context.tenant);
    let classification = match request_routing_header(
        &headers,
        &DATA_CLASSIFICATION,
        "x-modelport-data-classification",
        32,
    ) {
        Ok(Some(value)) => DataClassification::parse(&value).ok_or_else(|| {
            record_inference_rejection(
                &state,
                route_name,
                "governance",
                AppError::InvalidRequest(
                    "x-modelport-data-classification must be unknown, sensitive, internal, or public"
                        .to_owned(),
                ),
                started,
            )
        })?,
        Ok(None) => project_policy.default_classification,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "governance",
                err,
                started,
            ));
        }
    };
    let requested_hybrid_mode = match request_routing_header(
        &headers,
        &HYBRID_MODE,
        "x-modelport-hybrid-mode",
        32,
    ) {
        Ok(Some(value)) => Some(HybridMode::parse(&value).ok_or_else(|| {
            record_inference_rejection(
                &state,
                route_name,
                "governance",
                AppError::InvalidRequest(
                    "x-modelport-hybrid-mode must be local_strict, local_first, balanced, or cloud_first"
                        .to_owned(),
                ),
                started,
            )
        })?),
        Ok(None) => None,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "governance",
                err,
                started,
            ));
        }
    };
    let hybrid_mode = match project_policy.effective_mode(requested_hybrid_mode, classification) {
        Ok(mode) => mode,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "governance",
                err,
                started,
            ));
        }
    };
    let workload_class = WorkloadClass::parse(&traffic_class).unwrap_or_default();
    let requested_model = exchange.requested_model.clone();
    let config = effective_config(&state);
    let routing_profile = match request_routing_header(
        &headers,
        &ROUTING_PROFILE,
        "x-modelport-routing-profile",
        32,
    ) {
        Ok(value) => value,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "validation",
                err,
                started,
            ));
        }
    };
    let routing_session_id = match request_routing_header(
        &headers,
        &ROUTING_SESSION_ID,
        "x-modelport-session-id",
        128,
    ) {
        Ok(value) => value,
        Err(err) => {
            return Err(record_inference_rejection(
                &state,
                route_name,
                "validation",
                err,
                started,
            ));
        }
    };
    let routing_profile = match routing_profile.as_deref() {
        Some(value) => match RoutingProfile::parse(value) {
            Some(profile) => Some(profile),
            None => {
                return Err(record_inference_rejection(
                    &state,
                    route_name,
                    "validation",
                    AppError::InvalidRequest(
                        "x-modelport-routing-profile must be quality, balanced, economy, or latency"
                            .to_owned(),
                    ),
                    started,
                ));
            }
        },
        None => None,
    };
    let routing_session_hash = routing_session_id
        .as_deref()
        .map(|session_id| hash_session_key(&identity.user_id, session_id));
    let routing_activation_key = routing_session_hash.clone().unwrap_or_else(|| {
        hash_session_key(&identity.user_id, request_context.request_id.as_str())
    });
    let routing_plan = match state.smart_router.plan(RoutingRequest {
        config: &config,
        control: &state.control,
        identity: &identity,
        client_ip: request_client_ip.as_deref(),
        exchange: &exchange,
        profile_override: routing_profile,
        session_hash: routing_session_hash.as_deref(),
        activation_key: &routing_activation_key,
    }) {
        Ok(plan) => plan,
        Err(err) => {
            return Err(record_inference_rejection(
                &state, route_name, "routing", err, started,
            ));
        }
    };
    let ordered_attempts = order_attempts(routing_plan.attempts.clone(), hybrid_mode);
    let mut first_policy_error = None;
    let authorized_attempts = ordered_attempts
        .into_iter()
        .filter(|attempt| match project_policy.enforce_attempt(attempt) {
            Ok(()) => true,
            Err(error) => {
                if first_policy_error.is_none() {
                    first_policy_error = Some(error);
                }
                false
            }
        })
        .collect::<Vec<_>>();
    let resolved = authorized_attempts.first().cloned().ok_or_else(|| {
        first_policy_error.unwrap_or_else(|| {
            AppError::ProviderNotFound(format!(
                "no {} provider is authorized for this project",
                hybrid_mode.as_str()
            ))
        })
    })?;
    if let Err(err) = state.rate_limiter.check(RateLimitScope {
        identity: &identity,
        client_ip: request_client_ip.as_deref(),
        provider_id: None,
        model: None,
    }) {
        return Err(record_inference_rejection(
            &state,
            route_name,
            "rate_limit",
            err,
            started,
        ));
    }
    if let Err(err) = enforce_context_admission(&state, &headers, &exchange, &resolved).await {
        return Err(record_inference_rejection(
            &state,
            route_name,
            "admission",
            err,
            started,
        ));
    }
    let stream = exchange.stream;
    let stream_permit = if stream {
        match state.stream_permits.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                let err = AppError::RateLimited {
                    message: "concurrent stream limit exceeded".to_owned(),
                    retry_after_secs: 1,
                };
                return Err(record_inference_rejection(
                    &state,
                    route_name,
                    "concurrency",
                    err,
                    started,
                ));
            }
        }
    } else {
        None
    };
    let ledger_request = match state
        .ledger
        .begin_request_with_metadata(
            &request_context,
            &requested_model,
            stream,
            idempotency_key.as_deref(),
            &request_fingerprint,
            &LedgerRequestMetadata {
                request_path: exchange.request_path().to_owned(),
                traffic_class: traffic_class.clone(),
                tool_use_requested: exchange.uses_tools(),
                username: identity.username.clone(),
                api_key_id: identity.api_key_id.clone(),
                quota_subject_id: identity.quota_subject_id.clone(),
                api_key_name: identity.api_key_name.clone(),
                api_key_group: identity.api_key_group.clone(),
                team_id: identity.team_id.clone(),
                team_name: identity.team_name.clone(),
                client_ip: request_client_ip.clone(),
                routing_decision: Some(routing_plan.evidence.clone()),
            },
        )
        .await
    {
        Ok(request) => request,
        Err(err) => {
            return Err(record_inference_rejection(
                &state, route_name, "ledger", err, started,
            ));
        }
    };
    let ledger_lease = state
        .ledger
        .maintain_lease(&ledger_request, state.metrics.clone());
    info!(
        request_id = request_context.request_id.as_str(),
        organization_id = request_context.tenant.organization_id.as_str(),
        project_id = request_context.tenant.project_id.as_str(),
        environment_id = request_context.tenant.environment_id.as_str(),
        principal_id = request_context.principal_id.as_str(),
        principal_type = identity.principal_type.as_str(),
        client_protocol = request_context.protocol.as_str(),
        requested_model = exchange.requested_model.as_str(),
        provider = resolved.provider_id.as_str(),
        upstream_model = resolved.model.as_str(),
        routing_mode = routing_plan.evidence.mode.as_str(),
        routing_profile = routing_plan.evidence.profile.as_str(),
        routing_decision_id = routing_plan.evidence.decision_id.as_str(),
        recommended_provider = routing_plan.evidence.recommended_provider.as_str(),
        recommended_model = routing_plan.evidence.recommended_model.as_str(),
        stream,
        "routing inference request"
    );
    state.metrics.record_routing_decision(
        &routing_plan.evidence.mode,
        &routing_plan.evidence.profile,
        &routing_plan.evidence.selected_provider,
        routing_plan.evidence.shadow_disagreement,
    );
    let routing_decision_id = routing_plan.evidence.decision_id.clone();
    let routing_mode = routing_plan.evidence.mode.clone();

    let mut attempts = authorized_attempts
        .into_iter()
        .map(|resolved| (resolved, None, 0u32))
        .collect::<VecDeque<_>>();
    let mut provider_id = String::new();
    let mut upstream_model = String::new();
    let mut protocol = String::new();
    let mut retry_count = 0u32;
    let mut fallback_from_provider = None;
    let mut result = Err(AppError::ProviderNotFound(requested_model.clone()));
    let mut first_sent_provider = None::<String>;
    let mut sent_attempts = 0u32;
    let mut last_sent = None::<SentAttempt>;
    let mut tool_repair_attempted = false;
    let mut tool_repair_recovered = false;
    let mut prior_attempt_usage = UsageEstimate::default();
    let mut prior_pricing_evidence = None::<pricing::PricingEvidence>;
    let mut successful_local_lease = None::<LocalLease>;

    while let Some((mut attempt, repair, provider_retry_ordinal)) = attempts.pop_front() {
        let attempt_id = AttemptId::new();
        provider_id = attempt.provider_id.clone();
        upstream_model = attempt.model.clone();
        protocol = provider_protocol_value(attempt.provider.protocol).to_owned();
        let repair_template = attempt.clone();
        let repair_enabled = exchange.is_anthropic_client()
            && !stream
            && attempt.provider.protocol == ProviderProtocol::OpenaiCompat
            && attempt.provider.tool_use.repair_invalid_arguments;
        let attempt_boundary = ProviderBoundary::for_resolved(&attempt);
        let cloud_available = attempts.iter().any(|(candidate, _, _)| {
            ProviderBoundary::for_resolved(candidate) == ProviderBoundary::Cloud
        });
        let attempt_local_lease = if attempt_boundary == ProviderBoundary::Local {
            match state
                .local_scheduler
                .acquire(
                    &identity.user_id,
                    hybrid_mode,
                    workload_class,
                    cloud_available,
                )
                .await
            {
                Ok((LocalAdmission::Acquired, lease)) => lease,
                Ok((LocalAdmission::OverflowToCloud, _)) => continue,
                Err(err) => {
                    result = Err(err);
                    break;
                }
            }
        } else {
            None
        };
        let credential_id = match state
            .control
            .apply_selected_provider_credential_for_request(&provider_id, &mut attempt.provider)
        {
            Ok(credential_id) => credential_id,
            Err(err) => {
                result = Err(err);
                continue;
            }
        };
        let pricing_card = attempt
            .provider
            .model_pricing_card(&upstream_model)
            .cloned();
        let pricing_verified = pricing_card.is_some() || attempt.provider.trust_upstream_cost;
        let applied_pricing = attempt
            .provider
            .effective_pricing(&upstream_model)
            .unwrap_or_else(|| pricing::pricing_for_model(&upstream_model));
        let estimate = estimate_usage(&exchange, &upstream_model, Some(applied_pricing));
        let usage_policy = match state.control.check_quotas(
            &identity,
            request_client_ip.as_deref(),
            &requested_model,
            &upstream_model,
            &provider_id,
        ) {
            Ok(policy) => policy,
            Err(err) => {
                result = Err(err);
                continue;
            }
        };
        if let Err(err) = validate_inference_attempt(&state, &attempt, &exchange).await {
            result = Err(err);
            continue;
        }
        if let Err(err) = state
            .rate_limiter
            .check_provider_attempt(&provider_id, &upstream_model)
        {
            result = Err(err);
            continue;
        }
        let ledger_attempt = match state
            .ledger
            .begin_attempt_with_pricing(
                &ledger_request,
                &attempt_id,
                &provider_id,
                &upstream_model,
                &protocol,
                AttemptPricingEvidence {
                    estimate,
                    verified: pricing_verified,
                    usage_policy: &usage_policy,
                },
            )
            .await
        {
            Ok(attempt) => attempt,
            Err(err) => {
                result = Err(err);
                break;
            }
        };
        if sent_attempts > 0 {
            retry_count = retry_count.saturating_add(1);
            if first_sent_provider.as_deref() != Some(provider_id.as_str()) {
                fallback_from_provider = first_sent_provider.clone();
            }
        } else {
            first_sent_provider = Some(provider_id.clone());
        }
        sent_attempts = sent_attempts.saturating_add(1);
        info!(
            request_id = request_context.request_id.as_str(),
            attempt_id = attempt_id.as_str(),
            provider = provider_id.as_str(),
            upstream_model = upstream_model.as_str(),
            "starting provider attempt"
        );
        let stream_lifecycle = StreamLifecycle::new();
        let attempt_started = Instant::now();
        last_sent = Some(SentAttempt {
            attempt_id: attempt_id.clone(),
            provider_id: provider_id.clone(),
            model: upstream_model.clone(),
            protocol: protocol.clone(),
            boundary: attempt_boundary,
            credential_id: credential_id.clone(),
            estimate,
            pricing: applied_pricing,
            pricing_card,
            trust_upstream_cost: attempt.provider.trust_upstream_cost,
            stream_lifecycle: stream_lifecycle.clone(),
            ledger_attempt: ledger_attempt.clone(),
            started: attempt_started,
        });
        let attempt_result = send_inference_attempt(
            state.clone(),
            attempt,
            exchange.clone(),
            &headers,
            stream_lifecycle,
            repair,
        )
        .await;
        let attempt_success = attempt_result.is_ok();
        let attempt_status = attempt_result
            .as_ref()
            .map(|response| response.status().as_u16())
            .unwrap_or_else(|error| error.http_status().as_u16());
        let attempt_error = attempt_result.as_ref().err().map(AppError::audit_message);
        let mut finalized_attempt_estimate = None;
        if !(stream && attempt_success) {
            state.smart_router.record_outcome(
                &provider_id,
                &upstream_model,
                attempt_success,
                attempt_started.elapsed(),
            );
            if let Err(err) = state.control.record_provider_outcome_for_credential(
                &provider_id,
                credential_id.as_deref(),
                attempt_success,
                attempt_status,
                attempt_error.as_deref(),
                false,
            ) {
                error!(
                    error = %err,
                    request_id = request_context.request_id.as_str(),
                    attempt_id = attempt_id.as_str(),
                    "failed to persist provider attempt outcome"
                );
            }
            let provider_charge = attempt_result
                .as_ref()
                .ok()
                .and_then(|response| pricing::usage_from_headers(response.headers()));
            let rejected_tool_charge = attempt_result
                .as_ref()
                .err()
                .and_then(AppError::tool_argument_usage)
                .map(|usage| {
                    pricing::charge_with_evidence(
                        &provider_id,
                        &upstream_model,
                        usage,
                        Some(applied_pricing),
                        repair_template.provider.model_pricing_card(&upstream_model),
                        None,
                    )
                });
            let attempt_charge = provider_charge.or(rejected_tool_charge);
            let pricing_evidence = attempt_charge
                .as_ref()
                .and_then(|charge| charge.pricing_evidence.clone());
            if pricing_evidence.is_some() {
                prior_pricing_evidence.clone_from(&pricing_evidence);
            }
            let billing_mode = if attempt_charge.is_some() {
                "upstream-returned"
            } else {
                "local-estimate"
            };
            let attempt_estimate = attempt_charge
                .map(usage_estimate_from_charge)
                .unwrap_or(estimate);
            finalized_attempt_estimate = Some(attempt_estimate);
            let ledger_outcome = LedgerOutcome::provider_attempt_with_evidence(
                attempt_success,
                attempt_status,
                attempt_error.clone(),
                attempt_estimate,
                pricing_evidence,
                billing_mode,
                attempt_started.elapsed(),
            );
            let finalization = state
                .ledger
                .finalize_attempt(&ledger_attempt, &ledger_outcome)
                .await;
            state
                .metrics
                .record_ledger_operation("attempt_finalization", finalization.is_ok());
            if let Err(err) = finalization {
                error!(
                    error = %err,
                    request_id = request_context.request_id.as_str(),
                    attempt_id = attempt_id.as_str(),
                    "failed to finalize provider attempt ledger row"
                );
            }
        }
        result = attempt_result;
        if result.is_ok() {
            tool_repair_recovered = repair.is_some();
            successful_local_lease = attempt_local_lease;
            break;
        }

        let repair_candidate = match result.as_ref().err() {
            Some(AppError::ToolArgumentsInvalid { .. })
                if repair_enabled && !tool_repair_attempted =>
            {
                Some(providers::openai_compat::ToolArgumentRepair)
            }
            _ => None,
        };
        if let Some(repair_candidate) = repair_candidate {
            tool_repair_attempted = true;
            prior_attempt_usage = merge_usage_estimates(
                prior_attempt_usage,
                finalized_attempt_estimate.unwrap_or(estimate),
            );
            attempts.push_front((
                repair_template,
                Some(repair_candidate),
                provider_retry_ordinal,
            ));
            continue;
        }

        if !is_retryable_message_error(result.as_ref().err()) {
            break;
        }
        prior_attempt_usage = merge_usage_estimates(
            prior_attempt_usage,
            finalized_attempt_estimate.unwrap_or(estimate),
        );
        if provider_retry_ordinal + 1 < repair_template.provider.retry.max_attempts {
            let delay = provider_retry_delay(
                &repair_template.provider.retry,
                provider_retry_ordinal,
                result
                    .as_ref()
                    .err()
                    .and_then(AppError::upstream_retry_after_secs),
            );
            attempts.push_front((repair_template, repair, provider_retry_ordinal + 1));
            tokio::time::sleep(delay).await;
            continue;
        }
        if attempts.is_empty() {
            break;
        }
    }
    let success = result.is_ok();
    let duration = started.elapsed();
    let status_code = result
        .as_ref()
        .map(|response| response.status().as_u16())
        .unwrap_or_else(|error| error.http_status().as_u16());
    let timed_out = result.as_ref().err().is_some_and(
        |error| matches!(error, AppError::Transport(message) if message.contains("timed out")),
    );
    let error_message = result.as_ref().err().map(AppError::audit_message);
    let upstream_usage = result
        .as_ref()
        .ok()
        .and_then(|response| pricing::usage_from_headers(response.headers()));
    let chargeable = last_sent.is_some();
    if let Some(sent) = &last_sent {
        provider_id.clone_from(&sent.provider_id);
        upstream_model.clone_from(&sent.model);
        protocol.clone_from(&sent.protocol);
    }
    let local_estimate = last_sent
        .as_ref()
        .map(|sent| sent.estimate)
        .unwrap_or_default();
    let applied_pricing = last_sent.as_ref().map(|sent| sent.pricing);
    let pricing_evidence = upstream_usage
        .as_ref()
        .and_then(|charge| charge.pricing_evidence.clone())
        .or_else(|| prior_pricing_evidence.clone());
    let actual_estimate = merge_usage_estimates(
        prior_attempt_usage,
        upstream_usage
            .as_ref()
            .map(|charge| UsageEstimate {
                input_tokens: charge.input_tokens,
                output_tokens: charge.output_tokens,
                cache_write_tokens: charge.cache_write_tokens,
                cache_read_tokens: charge.cache_read_tokens,
                cost_estimate: charge.cost_estimate,
                actual_cost: charge.actual_cost,
                billable_cost: charge.billable_cost,
            })
            .unwrap_or(local_estimate),
    );
    let billing_mode = if tool_repair_attempted && upstream_usage.is_some() {
        "upstream-returned+tool-repair"
    } else if retry_count > 0 && upstream_usage.is_some() {
        "mixed-attempts"
    } else if retry_count > 0 {
        "local-estimate+retry"
    } else if upstream_usage.is_some() {
        "upstream-returned"
    } else {
        "local-estimate"
    };
    let tool_use_requested = exchange.uses_tools();
    let tool_continuation = exchange.has_tool_results();
    let response_observation = last_sent
        .as_ref()
        .map(|sent| sent.stream_lifecycle.response_observation())
        .unwrap_or_default();
    let terminal_reason = if success {
        "completed"
    } else if timed_out {
        "timeout_before_response"
    } else {
        "failed_before_response"
    };
    let tool_outcome = classify_tool_outcome(
        tool_use_requested,
        tool_continuation,
        success,
        timed_out,
        terminal_reason,
        error_message.as_deref(),
        &response_observation,
    );

    let usage = UsageEventInput {
        request_id: Some(request_context.request_id.to_string()),
        attempt_id: last_sent.as_ref().map(|sent| sent.attempt_id.to_string()),
        resolved_model: upstream_model,
        provider: provider_id,
        protocol,
        tool_use_requested,
        tool_outcome,
        traffic_class,
        tool_repair_attempted,
        tool_repair_recovered,
        success,
        timed_out,
        status_code,
        terminal_reason: terminal_reason.to_owned(),
        estimate: actual_estimate,
        model_pricing: applied_pricing,
        pricing_evidence,
        billing_mode: billing_mode.to_owned(),
        chargeable,
        latency: duration,
        first_byte_latency: stream
            .then(|| {
                last_sent
                    .as_ref()
                    .and_then(|sent| sent.stream_lifecycle.first_semantic_latency())
            })
            .flatten(),
        retry_count,
        fallback_from_provider,
        error_message,
    };
    let cloud_egress = last_sent
        .as_ref()
        .is_some_and(|sent| sent.boundary == ProviderBoundary::Cloud);

    if stream && success {
        let mut response = result.expect("successful stream result must contain a response");
        response.headers_mut().remove(pricing::USAGE_HEADER);
        attach_routing_response_headers(
            &mut response,
            &RoutingResponseEvidence {
                request_id: request_context.request_id.as_str(),
                logical_model: &requested_model,
                provider_id: &usage.provider,
                resolved_model: &usage.resolved_model,
                decision_id: &routing_decision_id,
                mode: &routing_mode,
                hybrid_mode,
                cloud_egress,
            },
        )?;
        let permit = stream_permit.expect("stream request must hold a stream permit");
        let sent = last_sent.expect("successful stream must have a sent attempt");
        return Ok(response_with_stream_finalizer(
            response,
            permit,
            StreamFinalizationContext {
                state,
                usage,
                tool_continuation,
                credential_id: sent.credential_id,
                pricing: sent.pricing,
                pricing_card: sent.pricing_card,
                trust_upstream_cost: sent.trust_upstream_cost,
                lifecycle: sent.stream_lifecycle,
                ledger_request,
                ledger_attempt: sent.ledger_attempt,
                attempt_started: sent.started,
                prior_attempt_usage,
                _ledger_lease: ledger_lease,
                _local_lease: successful_local_lease,
                started,
                route_name,
            },
        ));
    }

    state.metrics.record_route(route_name, success, duration);
    state.metrics.record_message(
        MessageMetricLabels {
            provider: &usage.provider,
            model: &usage.resolved_model,
            traffic_class: &usage.traffic_class,
            stream,
        },
        success,
        duration,
        actual_estimate,
    );
    let finalization = state
        .ledger
        .finalize_request_usage(&ledger_request, &usage)
        .await;
    state
        .metrics
        .record_ledger_operation("request_finalization", finalization.is_ok());
    if let Err(err) = finalization {
        error!(
            error = %err,
            request_id = request_context.request_id.as_str(),
            "failed to finalize request ledger row"
        );
    }
    let mut response = match result {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    response.headers_mut().remove(pricing::USAGE_HEADER);
    attach_routing_response_headers(
        &mut response,
        &RoutingResponseEvidence {
            request_id: request_context.request_id.as_str(),
            logical_model: &requested_model,
            provider_id: &usage.provider,
            resolved_model: &usage.resolved_model,
            decision_id: &routing_decision_id,
            mode: &routing_mode,
            hybrid_mode,
            cloud_egress,
        },
    )?;
    Ok(response)
}

fn ensure_accepting_inference(state: &AppState) -> Result<(), AppError> {
    if state.is_draining() {
        Err(AppError::NotReady(
            "gateway is draining and is not accepting new inference requests".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn record_inference_rejection(
    state: &AppState,
    route_name: &str,
    phase: &'static str,
    error: AppError,
    started: Instant,
) -> AppError {
    state
        .metrics
        .record_route(route_name, false, started.elapsed());
    state
        .metrics
        .record_rejection(route_name, phase, error.telemetry_code());
    error
}

fn request_tenant_scope(
    headers: &HeaderMap,
    bound_tenant: &crate::domain::TenantScope,
) -> Result<crate::domain::TenantScope, AppError> {
    let requested = [&ORGANIZATION_ID, &PROJECT_ID, &ENVIRONMENT_ID]
        .map(|name| headers.get(name).map(|value| value.to_str()))
        .into_iter()
        .map(|value| match value {
            None => Ok(None),
            Some(Ok(value)) => {
                let value = value.trim();
                if !crate::domain::valid_tenant_identifier(value) {
                    return Err(AppError::InvalidRequest(
                        "ModelPort tenant scope headers contain an invalid identifier".to_owned(),
                    ));
                }
                Ok(Some(value))
            }
            Some(Err(_)) => Err(AppError::InvalidRequest(
                "ModelPort tenant scope headers must be ASCII".to_owned(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    match requested.as_slice() {
        [None, None, None] => Ok(bound_tenant.clone()),
        [Some(organization_id), Some(project_id), Some(environment_id)]
            if *organization_id == bound_tenant.organization_id.as_str()
                && *project_id == bound_tenant.project_id.as_str()
                && *environment_id == bound_tenant.environment_id.as_str() =>
        {
            Ok(bound_tenant.clone())
        }
        [Some(_), Some(_), Some(_)] => Err(AppError::Forbidden(
            "requested ModelPort tenant scope is not bound to this API key".to_owned(),
        )),
        _ => Err(AppError::InvalidRequest(
            "x-modelport-organization-id, x-modelport-project-id, and x-modelport-environment-id must be supplied together"
                .to_owned(),
        )),
    }
}

async fn enforce_context_admission(
    state: &AppState,
    headers: &HeaderMap,
    exchange: &ExchangeRequest,
    resolved: &ResolvedProvider,
) -> Result<(), AppError> {
    let admission = &resolved.provider.token_counting;
    validate_output_budget(
        &exchange.requested_model,
        exchange.estimated_output_tokens(),
        admission,
    )?;
    let Some(context_tokens) = admission.context_tokens else {
        return Ok(());
    };
    let Some(count_request) = exchange.anthropic_count_tokens_request() else {
        // OpenAI Chat Completions has no lossless Anthropic Count Tokens body.
        // Keep this capability scoped to the Anthropic edge rather than using
        // the local characters/4 estimate as a hard admission decision.
        return Ok(());
    };
    if admission.mode != TokenCountingMode::Anthropic {
        return Err(AppError::Config(format!(
            "provider `{}` context admission requires exact Anthropic token counting",
            resolved.provider_id
        )));
    }
    let mut counting_provider = resolved.clone();
    state
        .control
        .apply_selected_provider_credential_for_request(
            &counting_provider.provider_id,
            &mut counting_provider.provider,
        )?;
    crate::config::validate_provider_base_url_dns_for_request(
        &counting_provider.provider_id,
        &counting_provider.provider.base_url,
        state.security.allow_private_provider_urls,
    )
    .await?;
    if counting_provider.provider.api_key_required {
        let _ = counting_provider.provider.api_key()?;
    }
    let input_tokens = providers::token_counting::input_tokens(
        state.clone(),
        counting_provider,
        count_request,
        headers,
    )
    .await?;
    // A repair is a distinct provider attempt with a short gateway-generated
    // instruction. Reserve a bounded margin during the original exact-token
    // admission so retrying cannot push a near-limit request over context.
    const TOOL_REPAIR_CONTEXT_RESERVE: u64 = 256;
    let repair_reserve = if exchange.is_anthropic_client()
        && !exchange.stream
        && exchange.uses_tools()
        && resolved.provider.protocol == ProviderProtocol::OpenaiCompat
        && resolved.provider.tool_use.repair_invalid_arguments
    {
        TOOL_REPAIR_CONTEXT_RESERVE
    } else {
        0
    };
    let output_tokens = exchange
        .estimated_output_tokens()
        .saturating_add(repair_reserve);
    let recommended_input_tokens = recommended_input_limit(&exchange.requested_model, admission);
    validate_context_budget(
        input_tokens,
        output_tokens,
        context_tokens,
        recommended_input_tokens,
        exchange.thinking_disabled(),
    )
}

fn recommended_input_limit(requested_model: &str, admission: &TokenCountingConfig) -> Option<u64> {
    admission
        .model_recommended_input_tokens
        .get(requested_model)
        .copied()
        .or(admission.recommended_reasoning_input_tokens)
}

fn validate_output_budget(
    requested_model: &str,
    output_tokens: u64,
    admission: &TokenCountingConfig,
) -> Result<(), AppError> {
    let model_limit = admission
        .model_max_output_tokens
        .get(requested_model)
        .copied();
    let Some(limit) = model_limit.or(admission.max_output_tokens) else {
        return Ok(());
    };
    if output_tokens > limit {
        let scope = if model_limit.is_some() {
            format!("logical model `{requested_model}`")
        } else {
            "selected provider".to_owned()
        };
        return Err(AppError::InvalidRequest(format!(
            "output admission rejected request: max_output_tokens={output_tokens} exceeds \
             configured limit={limit} for {scope}; reduce max_tokens"
        )));
    }
    Ok(())
}

fn validate_context_budget(
    input_tokens: u64,
    output_tokens: u64,
    context_tokens: u64,
    recommended_reasoning_input_tokens: Option<u64>,
    thinking_disabled: bool,
) -> Result<(), AppError> {
    let requested_total = input_tokens.saturating_add(output_tokens);
    if requested_total > context_tokens {
        let excess = requested_total - context_tokens;
        return Err(AppError::InvalidRequest(format!(
            "context admission rejected request: input_tokens={input_tokens} + \
             max_output_tokens={output_tokens} exceeds context_tokens={context_tokens}; \
             reduce input or max_tokens by at least {excess} tokens; input is never silently truncated"
        )));
    }
    if !thinking_disabled
        && let Some(recommended) = recommended_reasoning_input_tokens
        && input_tokens > recommended
    {
        let excess = input_tokens - recommended;
        return Err(AppError::InvalidRequest(format!(
            "reasoning context admission rejected request: input_tokens={input_tokens} exceeds \
             recommended_reasoning_input_tokens={recommended}; reduce input by at least \
             {excess} tokens or explicitly disable thinking for a non-reasoning task"
        )));
    }
    Ok(())
}

fn classify_tool_outcome(
    requested: bool,
    continuation: bool,
    success: bool,
    timed_out: bool,
    terminal_reason: &str,
    error_message: Option<&str>,
    response: &ResponseObservation,
) -> String {
    if !requested {
        return "not_requested".to_owned();
    }
    if success {
        if response.tool_call_count > 0 {
            return if continuation {
                "continuation_tool_called"
            } else {
                "tool_called"
            }
            .to_owned();
        }
        if continuation {
            return "final_answer".to_owned();
        }
        if response.text_present {
            return "answered_without_tool".to_owned();
        }
        return "completed_unobserved".to_owned();
    }
    if terminal_reason == "downstream_cancelled" {
        return "client_cancelled".to_owned();
    }
    if timed_out || terminal_reason.contains("timeout") {
        return "timeout".to_owned();
    }
    let error = error_message.unwrap_or_default().to_ascii_lowercase();
    if ["tool", "function", "input_json", "tool_use", "tool_result"]
        .iter()
        .any(|marker| error.contains(marker))
    {
        return "protocol_error".to_owned();
    }
    "upstream_or_delivery_error".to_owned()
}

fn usage_estimate_from_charge(charge: pricing::UsageCharge) -> UsageEstimate {
    UsageEstimate {
        input_tokens: charge.input_tokens,
        output_tokens: charge.output_tokens,
        cache_write_tokens: charge.cache_write_tokens,
        cache_read_tokens: charge.cache_read_tokens,
        cost_estimate: charge.cost_estimate,
        actual_cost: charge.actual_cost,
        billable_cost: charge.billable_cost,
    }
}

fn merge_usage_estimates(left: UsageEstimate, right: UsageEstimate) -> UsageEstimate {
    UsageEstimate {
        input_tokens: left.input_tokens.saturating_add(right.input_tokens),
        output_tokens: left.output_tokens.saturating_add(right.output_tokens),
        cache_write_tokens: left
            .cache_write_tokens
            .saturating_add(right.cache_write_tokens),
        cache_read_tokens: left
            .cache_read_tokens
            .saturating_add(right.cache_read_tokens),
        cost_estimate: left.cost_estimate + right.cost_estimate,
        actual_cost: merge_complete_cost(left, right),
        billable_cost: merge_billable_cost(left.billable_cost, right.billable_cost),
    }
}

fn merge_complete_cost(left: UsageEstimate, right: UsageEstimate) -> Option<f64> {
    let left_empty = estimate_total_tokens_for_merge(left) == 0 && left.cost_estimate == 0.0;
    let right_empty = estimate_total_tokens_for_merge(right) == 0 && right.cost_estimate == 0.0;
    match (left.actual_cost, right.actual_cost) {
        (Some(left), Some(right)) => Some(left + right),
        (None, Some(right)) if left_empty => Some(right),
        (Some(left), None) if right_empty => Some(left),
        _ => None,
    }
}

fn merge_billable_cost(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left + right),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn estimate_total_tokens_for_merge(estimate: UsageEstimate) -> u64 {
    estimate
        .input_tokens
        .saturating_add(estimate.output_tokens)
        .saturating_add(estimate.cache_write_tokens)
        .saturating_add(estimate.cache_read_tokens)
}

fn request_idempotency_key(headers: &HeaderMap) -> Result<Option<String>, AppError> {
    let Some(value) = headers.get(&IDEMPOTENCY_KEY) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| AppError::InvalidRequest("Idempotency-Key must be ASCII".to_owned()))?
        .trim();
    if value.is_empty() || value.len() > 200 {
        return Err(AppError::InvalidRequest(
            "Idempotency-Key must contain 1 to 200 visible ASCII characters".to_owned(),
        ));
    }
    if !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(AppError::InvalidRequest(
            "Idempotency-Key must contain only visible ASCII characters without whitespace"
                .to_owned(),
        ));
    }
    Ok(Some(value.to_owned()))
}

fn request_traffic_class(headers: &HeaderMap) -> Result<String, AppError> {
    let Some(value) = headers.get(&TRAFFIC_CLASS) else {
        return Ok("business".to_owned());
    };
    let value = value
        .to_str()
        .map_err(|_| AppError::InvalidRequest("traffic class must be ASCII".to_owned()))?;
    match value.trim().to_ascii_lowercase().as_str() {
        "business" => Ok("business".to_owned()),
        "batch" => Ok("batch".to_owned()),
        "synthetic" => Ok("synthetic".to_owned()),
        "diagnostic" => Ok("diagnostic".to_owned()),
        _ => Err(AppError::InvalidRequest(
            "x-modelport-traffic-class must be business, batch, synthetic, or diagnostic"
                .to_owned(),
        )),
    }
}

struct StreamFinalizationContext {
    state: AppState,
    usage: UsageEventInput,
    tool_continuation: bool,
    credential_id: Option<String>,
    pricing: ModelPricing,
    pricing_card: Option<pricing::ModelPricingCard>,
    trust_upstream_cost: bool,
    lifecycle: StreamLifecycle,
    ledger_request: LedgerRequest,
    ledger_attempt: LedgerAttempt,
    attempt_started: Instant,
    prior_attempt_usage: UsageEstimate,
    _ledger_lease: LedgerLease,
    _local_lease: Option<LocalLease>,
    started: Instant,
    route_name: &'static str,
}

impl StreamFinalizationContext {
    fn finalize(mut self, outcome: StreamTerminalOutcome) {
        let duration = self.started.elapsed();
        let provider_reported_cost = self
            .trust_upstream_cost
            .then(|| self.lifecycle.provider_reported_cost())
            .flatten();
        if let Some(usage) = self
            .lifecycle
            .usage()
            .or_else(|| provider_reported_cost.map(|_| TokenUsageBreakdown::default()))
        {
            let charge = pricing::charge_with_evidence(
                &self.usage.provider,
                &self.usage.resolved_model,
                usage,
                Some(self.pricing),
                self.pricing_card.as_ref(),
                provider_reported_cost,
            );
            self.usage.estimate = merge_usage_estimates(
                self.prior_attempt_usage,
                UsageEstimate {
                    input_tokens: charge.input_tokens,
                    output_tokens: charge.output_tokens,
                    cache_write_tokens: charge.cache_write_tokens,
                    cache_read_tokens: charge.cache_read_tokens,
                    cost_estimate: charge.cost_estimate,
                    actual_cost: charge.actual_cost,
                    billable_cost: charge.billable_cost,
                },
            );
            if charge.pricing_evidence.is_some() {
                self.usage.pricing_evidence = charge.pricing_evidence;
            }
            self.usage.billing_mode = if self.usage.retry_count > 0 {
                "mixed-attempts".to_owned()
            } else {
                "upstream-returned".to_owned()
            };
        }
        self.usage.success = outcome.success();
        self.usage.timed_out = outcome.timed_out();
        self.usage.status_code = outcome.status_code();
        self.usage.terminal_reason = outcome.terminal_reason().to_owned();
        self.usage.error_message = outcome.audit_error_message();
        self.usage.tool_outcome = classify_tool_outcome(
            self.usage.tool_use_requested,
            self.tool_continuation,
            self.usage.success,
            self.usage.timed_out,
            &self.usage.terminal_reason,
            self.usage.error_message.as_deref(),
            &self.lifecycle.response_observation(),
        );
        self.usage.latency = duration;
        self.usage.first_byte_latency = self.lifecycle.first_semantic_latency();

        self.state
            .metrics
            .record_route(self.route_name, self.usage.success, duration);
        self.state.metrics.record_message(
            MessageMetricLabels {
                provider: &self.usage.provider,
                model: &self.usage.resolved_model,
                traffic_class: &self.usage.traffic_class,
                stream: true,
            },
            self.usage.success,
            duration,
            self.usage.estimate,
        );

        if let Some((success, status_code, error_message)) =
            provider_terminal_outcome(&outcome, &self.lifecycle)
        {
            self.state.smart_router.record_outcome(
                &self.usage.provider,
                &self.usage.resolved_model,
                success,
                self.attempt_started.elapsed(),
            );
            if let Err(err) = self.state.control.record_provider_outcome_for_credential(
                &self.usage.provider,
                self.credential_id.as_deref(),
                success,
                status_code,
                error_message.as_deref(),
                false,
            ) {
                error!(
                    error = %err,
                    request_id = self.usage.request_id.as_deref().unwrap_or("unknown"),
                    attempt_id = self.usage.attempt_id.as_deref().unwrap_or("unknown"),
                    "failed to record provider stream outcome"
                );
            }
        }

        info!(
            request_id = self.usage.request_id.as_deref().unwrap_or("unknown"),
            attempt_id = self.usage.attempt_id.as_deref().unwrap_or("unknown"),
            provider = self.usage.provider.as_str(),
            status_code = self.usage.status_code,
            terminal_reason = self.usage.terminal_reason.as_str(),
            duration_ms = duration.as_millis(),
            "finalized message stream"
        );
        let attempt_ledger_outcome =
            LedgerOutcome::from_usage_with_latency(&self.usage, self.attempt_started.elapsed());
        let ledger = self.state.ledger.clone();
        let ledger_request = self.ledger_request;
        let ledger_attempt = self.ledger_attempt;
        let request_id = self.usage.request_id.clone();
        let request_usage = self.usage;
        let ledger_lease = self._ledger_lease;
        let finalizers = self.state.finalizers.clone();
        let metrics = self.state.metrics.clone();
        let finalizer_request_id = request_id.clone();
        if !finalizers.spawn(async move {
            let attempt_finalization = ledger
                .finalize_attempt(&ledger_attempt, &attempt_ledger_outcome)
                .await;
            metrics.record_ledger_operation("attempt_finalization", attempt_finalization.is_ok());
            if let Err(err) = attempt_finalization {
                error!(
                    error = %err,
                    request_id = request_id.as_deref().unwrap_or("unknown"),
                    "failed to finalize streaming attempt ledger row"
                );
            }
            let request_finalization = ledger
                .finalize_request_usage(&ledger_request, &request_usage)
                .await;
            metrics.record_ledger_operation("request_finalization", request_finalization.is_ok());
            if let Err(err) = request_finalization {
                error!(
                    error = %err,
                    request_id = request_id.as_deref().unwrap_or("unknown"),
                    "failed to finalize streaming request ledger row"
                );
            }
            drop(ledger_lease);
        }) {
            self.state
                .metrics
                .record_ledger_operation("finalizer_spawn", false);
            error!(
                request_id = finalizer_request_id.as_deref().unwrap_or("unknown"),
                "streaming ledger finalizer could not start outside a Tokio runtime"
            );
        } else {
            self.state
                .metrics
                .record_ledger_operation("finalizer_spawn", true);
        }
    }
}

struct StreamFinalizationGuard(Option<StreamFinalizationContext>);

impl StreamFinalizationGuard {
    fn new(context: StreamFinalizationContext) -> Self {
        Self(Some(context))
    }

    fn finish(&mut self, outcome: StreamTerminalOutcome) {
        if let Some(context) = self.0.take() {
            context.finalize(outcome);
        }
    }
}

impl Drop for StreamFinalizationGuard {
    fn drop(&mut self) {
        let Some(context) = self.0.take() else {
            return;
        };
        let outcome = StreamTerminalOutcome::after_drop(&context.lifecycle);
        context.finalize(outcome);
    }
}

fn provider_terminal_outcome(
    outcome: &StreamTerminalOutcome,
    lifecycle: &StreamLifecycle,
) -> Option<(bool, u16, Option<String>)> {
    match outcome {
        StreamTerminalOutcome::Completed => Some((true, 200, None)),
        StreamTerminalOutcome::UpstreamFailed(error) => Some((
            false,
            upstream_failure_status(error),
            Some(audit_safe_stream_error(error)),
        )),
        StreamTerminalOutcome::DeliveryFailed(_)
        | StreamTerminalOutcome::DownstreamCancelled { .. } => match lifecycle.state() {
            UpstreamStreamState::Completed => Some((true, 200, None)),
            UpstreamStreamState::Failed(error) => {
                let status = upstream_failure_status(&error);
                Some((false, status, Some(audit_safe_stream_error(&error))))
            }
            UpstreamStreamState::Pending => None,
        },
    }
}

fn upstream_failure_status(error: &str) -> u16 {
    if error.to_ascii_lowercase().contains("timed out") {
        504
    } else {
        502
    }
}

fn response_with_stream_finalizer(
    response: Response,
    permit: tokio::sync::OwnedSemaphorePermit,
    context: StreamFinalizationContext,
) -> Response {
    let (parts, body) = response.into_parts();
    let lifecycle = context.lifecycle.clone();
    let guard = StreamFinalizationGuard::new(context);
    let stream = async_stream::stream! {
        let _permit = permit;
        let mut guard = guard;
        let mut body = body.into_data_stream();
        while let Some(chunk) = body.next().await {
            match chunk {
                Ok(bytes) => yield Ok::<_, axum::Error>(bytes),
                Err(err) => {
                    guard.finish(StreamTerminalOutcome::after_body_error(
                        &lifecycle,
                        err.to_string(),
                    ));
                    yield Err(err);
                    return;
                }
            }
        }
        guard.finish(StreamTerminalOutcome::after_eof(&lifecycle));
    };
    Response::from_parts(parts, Body::from_stream(stream))
}

async fn send_inference_attempt(
    state: AppState,
    resolved: ResolvedProvider,
    request: ExchangeRequest,
    headers: &HeaderMap,
    stream_lifecycle: StreamLifecycle,
    repair: Option<providers::openai_compat::ToolArgumentRepair>,
) -> Result<Response, AppError> {
    if !request.is_anthropic_client() {
        return match resolved.provider.protocol {
            ProviderProtocol::Anthropic => providers::anthropic::chat_completions(
                state,
                resolved,
                request,
                headers,
                stream_lifecycle,
            )
            .await
            .map(IntoResponse::into_response),
            ProviderProtocol::OpenaiCompat => providers::openai_compat::chat_completions(
                state,
                resolved,
                request,
                headers,
                stream_lifecycle,
            )
            .await
            .map(IntoResponse::into_response),
        };
    }

    let ClientRequest::Anthropic(request) = request.into_source() else {
        unreachable!("Anthropic client exchange must retain its source request");
    };
    match resolved.provider.protocol {
        ProviderProtocol::Anthropic => {
            providers::anthropic::messages(state, resolved, request, headers, stream_lifecycle)
                .await
                .map(IntoResponse::into_response)
        }
        ProviderProtocol::OpenaiCompat => providers::openai_compat::messages(
            state,
            resolved,
            request,
            headers,
            stream_lifecycle,
            repair,
        )
        .await
        .map(IntoResponse::into_response),
    }
}

async fn validate_inference_attempt(
    state: &AppState,
    resolved: &ResolvedProvider,
    request: &ExchangeRequest,
) -> Result<(), AppError> {
    crate::config::validate_provider_base_url_dns_for_request(
        &resolved.provider_id,
        &resolved.provider.base_url,
        state.security.allow_private_provider_urls,
    )
    .await?;
    if resolved.provider.api_key_required {
        let _ = resolved.provider.api_key()?;
    }
    request.validate_provider(resolved)?;
    Ok(())
}

fn is_retryable_message_error(error: Option<&AppError>) -> bool {
    match error {
        Some(AppError::Transport(_)) => true,
        Some(AppError::Upstream { status, .. }) => *status == 429 || *status >= 500,
        _ => false,
    }
}

fn provider_retry_delay(
    retry: &crate::config::ProviderRetryConfig,
    retry_ordinal: u32,
    retry_after_secs: Option<u64>,
) -> std::time::Duration {
    let multiplier = 1u64.checked_shl(retry_ordinal.min(20)).unwrap_or(u64::MAX);
    let policy_ms = retry
        .initial_delay_ms
        .saturating_mul(multiplier)
        .min(retry.max_delay_ms);
    let jitter_span = (policy_ms as f64 * retry.jitter_ratio) as u64;
    let jittered_ms = if jitter_span == 0 {
        policy_ms
    } else {
        let width = jitter_span.saturating_mul(2).saturating_add(1);
        let sample = OsRng.next_u64() % width;
        policy_ms.saturating_sub(jitter_span).saturating_add(sample)
    };
    let retry_after_ms = retry_after_secs.unwrap_or(0).saturating_mul(1_000);
    std::time::Duration::from_millis(jittered_ms.max(retry_after_ms).min(retry.max_delay_ms))
}

fn request_routing_header(
    headers: &HeaderMap,
    name: &axum::http::HeaderName,
    label: &str,
    max_len: usize,
) -> Result<Option<String>, AppError> {
    let Some(value) = headers.get(name) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| AppError::InvalidRequest(format!("{label} must be ASCII")))?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_len || value.chars().any(char::is_control) {
        return Err(AppError::InvalidRequest(format!(
            "{label} must contain at most {max_len} non-control bytes"
        )));
    }
    Ok(Some(value.to_owned()))
}

struct RoutingResponseEvidence<'a> {
    request_id: &'a str,
    logical_model: &'a str,
    provider_id: &'a str,
    resolved_model: &'a str,
    decision_id: &'a str,
    mode: &'a str,
    hybrid_mode: HybridMode,
    cloud_egress: bool,
}

fn attach_routing_response_headers(
    response: &mut Response,
    evidence: &RoutingResponseEvidence<'_>,
) -> Result<(), AppError> {
    insert_safe_response_header(response, &X_REQUEST_ID, evidence.request_id, "request ID")?;
    insert_safe_response_header(
        response,
        &LOGICAL_MODEL,
        evidence.logical_model,
        "logical model",
    )?;
    insert_safe_response_header(
        response,
        &RESOLVED_PROVIDER,
        evidence.provider_id,
        "resolved provider",
    )?;
    insert_safe_response_header(
        response,
        &RESOLVED_MODEL,
        evidence.resolved_model,
        "resolved model",
    )?;
    response.headers_mut().insert(
        ROUTING_DECISION_ID.clone(),
        axum::http::HeaderValue::from_str(evidence.decision_id)
            .map_err(|_| AppError::Config("invalid routing decision ID".to_owned()))?,
    );
    response.headers_mut().insert(
        ROUTING_MODE.clone(),
        axum::http::HeaderValue::from_str(evidence.mode)
            .map_err(|_| AppError::Config("invalid routing mode".to_owned()))?,
    );
    response.headers_mut().insert(
        EXECUTION_MODE.clone(),
        axum::http::HeaderValue::from_static(evidence.hybrid_mode.as_str()),
    );
    response.headers_mut().insert(
        ROUTING_POLICY.clone(),
        axum::http::HeaderValue::from_static(evidence.hybrid_mode.as_str()),
    );
    response.headers_mut().insert(
        CLOUD_EGRESS.clone(),
        axum::http::HeaderValue::from_static(if evidence.cloud_egress {
            "true"
        } else {
            "false"
        }),
    );
    Ok(())
}

fn insert_safe_response_header(
    response: &mut Response,
    name: &axum::http::HeaderName,
    value: &str,
    label: &str,
) -> Result<(), AppError> {
    let value = axum::http::HeaderValue::from_str(value)
        .map_err(|_| AppError::Config(format!("invalid {label} response metadata")))?;
    response.headers_mut().insert(name.clone(), value);
    Ok(())
}

fn validate_message_request(request: &AnthropicRequest) -> Result<(), AppError> {
    validate_anthropic_input(request)?;
    let max_output_tokens = env_u64("MODELPORT_MAX_OUTPUT_TOKENS", 131_072);
    let max_tokens = request
        .max_tokens
        .ok_or_else(|| AppError::InvalidRequest("max_tokens is required".to_owned()))?;
    if max_tokens == 0 {
        return Err(AppError::InvalidRequest(
            "max_tokens must be greater than 0".to_owned(),
        ));
    }
    if max_tokens > max_output_tokens {
        return Err(AppError::InvalidRequest(format!(
            "max_tokens exceeds configured limit; max={max_output_tokens}"
        )));
    }
    Ok(())
}

fn validate_count_tokens_request(request: &AnthropicCountTokensRequest) -> Result<(), AppError> {
    validate_anthropic_input(&request.as_message_request())
}

fn validate_anthropic_input(request: &AnthropicRequest) -> Result<(), AppError> {
    let max_model_name_chars = env_usize("MODELPORT_MAX_MODEL_NAME_CHARS", 240);
    let max_messages = env_usize("MODELPORT_MAX_MESSAGES", 200);
    let max_messages_json_chars = env_usize("MODELPORT_MAX_MESSAGES_JSON_CHARS", 2 * 1024 * 1024);
    let max_system_json_chars = env_usize("MODELPORT_MAX_SYSTEM_JSON_CHARS", 256 * 1024);
    let max_tools = env_usize("MODELPORT_MAX_TOOLS", 256);
    let max_tools_json_chars = env_usize("MODELPORT_MAX_TOOLS_JSON_CHARS", 1024 * 1024);

    if request.model.trim().is_empty() {
        return Err(AppError::InvalidRequest("model is required".to_owned()));
    }
    if request.model.chars().count() > max_model_name_chars {
        return Err(AppError::InvalidRequest(format!(
            "model is too long; max={max_model_name_chars} chars"
        )));
    }

    if request.messages.is_empty() {
        return Err(AppError::InvalidRequest(
            "messages must not be empty".to_owned(),
        ));
    }
    if request.messages.len() > max_messages {
        return Err(AppError::InvalidRequest(format!(
            "too many messages; max={max_messages}"
        )));
    }

    let messages_json_chars = serde_json::to_string(&request.messages)
        .map(|value| value.chars().count())
        .unwrap_or(0);
    if messages_json_chars > max_messages_json_chars {
        return Err(AppError::InvalidRequest(format!(
            "messages JSON is too large; max={max_messages_json_chars} chars"
        )));
    }

    if let Some(system) = &request.system {
        let system_json_chars = serde_json::to_string(system)
            .map(|value| value.chars().count())
            .unwrap_or(0);
        if system_json_chars > max_system_json_chars {
            return Err(AppError::InvalidRequest(format!(
                "system JSON is too large; max={max_system_json_chars} chars"
            )));
        }
    }

    if let Some(tools) = request.extra.get("tools") {
        let Some(tools_array) = tools.as_array() else {
            return Err(AppError::InvalidRequest(
                "tools must be an array".to_owned(),
            ));
        };
        if tools_array.len() > max_tools {
            return Err(AppError::InvalidRequest(format!(
                "too many tools; max={max_tools}"
            )));
        }
        let tools_json_chars = serde_json::to_string(tools)
            .map(|value| value.chars().count())
            .unwrap_or(0);
        if tools_json_chars > max_tools_json_chars {
            return Err(AppError::InvalidRequest(format!(
                "tools JSON is too large; max={max_tools_json_chars} chars"
            )));
        }
    }

    for (index, message) in request.messages.iter().enumerate() {
        let Some(object) = message.as_object() else {
            return Err(AppError::InvalidRequest(format!(
                "messages[{index}] must be an object"
            )));
        };
        let role = object.get("role").and_then(Value::as_str).ok_or_else(|| {
            AppError::InvalidRequest(format!("messages[{index}].role is required"))
        })?;
        if !matches!(role, "user" | "assistant") {
            return Err(AppError::InvalidRequest(format!(
                "messages[{index}].role must be user or assistant"
            )));
        }
        let Some(content) = object.get("content") else {
            return Err(AppError::InvalidRequest(format!(
                "messages[{index}].content is required"
            )));
        };
        if !content.is_string() && !content.is_array() {
            return Err(AppError::InvalidRequest(format!(
                "messages[{index}].content must be a string or array"
            )));
        }
    }

    validate_anthropic_tooling(request)?;

    Ok(())
}

fn validate_openai_chat_request(request: &OpenAiChatRequest) -> Result<(), AppError> {
    let max_model_name_chars = env_usize("MODELPORT_MAX_MODEL_NAME_CHARS", 240);
    let max_messages = env_usize("MODELPORT_MAX_MESSAGES", 200);
    let max_messages_json_chars = env_usize("MODELPORT_MAX_MESSAGES_JSON_CHARS", 2 * 1024 * 1024);
    let max_tools = env_usize("MODELPORT_MAX_TOOLS", 256);
    let max_tools_json_chars = env_usize("MODELPORT_MAX_TOOLS_JSON_CHARS", 1024 * 1024);
    let max_output_tokens = env_u64("MODELPORT_MAX_OUTPUT_TOKENS", 131_072);

    if request.model.trim().is_empty() {
        return Err(AppError::InvalidRequest("model is required".to_owned()));
    }
    if request.model.chars().count() > max_model_name_chars {
        return Err(AppError::InvalidRequest(format!(
            "model is too long; max={max_model_name_chars} chars"
        )));
    }
    if request.messages.is_empty() {
        return Err(AppError::InvalidRequest(
            "messages must not be empty".to_owned(),
        ));
    }
    if request.messages.len() > max_messages {
        return Err(AppError::InvalidRequest(format!(
            "too many messages; max={max_messages}"
        )));
    }
    if request
        .max_completion_tokens
        .or(request.max_tokens)
        .is_some_and(|value| value > max_output_tokens)
    {
        return Err(AppError::InvalidRequest(format!(
            "max_completion_tokens/max_tokens exceeds configured limit; max={max_output_tokens}"
        )));
    }
    let messages_json_chars = serde_json::to_string(&request.messages)
        .map(|value| value.chars().count())
        .unwrap_or(0);
    if messages_json_chars > max_messages_json_chars {
        return Err(AppError::InvalidRequest(format!(
            "messages JSON is too large; max={max_messages_json_chars} chars"
        )));
    }
    if let Some(tools) = request.extra.get("tools") {
        let tools = tools
            .as_array()
            .ok_or_else(|| AppError::InvalidRequest("tools must be an array".to_owned()))?;
        if tools.len() > max_tools {
            return Err(AppError::InvalidRequest(format!(
                "too many tools; max={max_tools}"
            )));
        }
        let tools_json_chars = serde_json::to_string(tools)
            .map(|value| value.chars().count())
            .unwrap_or(0);
        if tools_json_chars > max_tools_json_chars {
            return Err(AppError::InvalidRequest(format!(
                "tools JSON is too large; max={max_tools_json_chars} chars"
            )));
        }
    }
    Ok(())
}

fn estimate_usage(
    request: &ExchangeRequest,
    resolved_model: &str,
    configured_pricing: Option<ModelPricing>,
) -> UsageEstimate {
    // Estimate the complete input payload, including tool schemas and flattened
    // protocol fields. The heuristic is conservative and the provider-reported
    // usage replaces it whenever a completed response exposes usage metadata.
    let input_chars = request.serialized_input_chars();
    let input_tokens = u64::try_from(input_chars.div_ceil(4)).unwrap_or(u64::MAX);
    let output_tokens = request.estimated_output_tokens();
    UsageEstimate {
        input_tokens,
        output_tokens,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        cost_estimate: pricing::cost_for_model_with_pricing(
            resolved_model,
            TokenUsageBreakdown {
                input_tokens,
                output_tokens,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
            configured_pricing,
        ),
        actual_cost: None,
        billable_cost: None,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use crate::config::{TokenCountingConfig, TokenCountingMode};

    use super::{
        ENVIRONMENT_ID, ORGANIZATION_ID, PROJECT_ID, ResponseObservation, classify_tool_outcome,
        provider_retry_delay, recommended_input_limit, request_tenant_scope, request_traffic_class,
        validate_context_budget, validate_output_budget,
    };

    #[test]
    fn tenant_scope_headers_are_assertions_not_authority() {
        let bound =
            crate::domain::TenantScope::from_strings("org_dave", "prj_quantpilot", "env_test");
        let mut matching = HeaderMap::new();
        matching.insert(&ORGANIZATION_ID, HeaderValue::from_static("org_dave"));
        matching.insert(&PROJECT_ID, HeaderValue::from_static("prj_quantpilot"));
        matching.insert(&ENVIRONMENT_ID, HeaderValue::from_static("env_test"));
        assert_eq!(request_tenant_scope(&matching, &bound).unwrap(), bound);

        let mut forged = matching;
        forged.insert(&PROJECT_ID, HeaderValue::from_static("prj_other"));
        assert!(matches!(
            request_tenant_scope(&forged, &bound),
            Err(crate::error::AppError::Forbidden(_))
        ));

        let mut partial = HeaderMap::new();
        partial.insert(&ORGANIZATION_ID, HeaderValue::from_static("org_dave"));
        assert!(matches!(
            request_tenant_scope(&partial, &bound),
            Err(crate::error::AppError::InvalidRequest(_))
        ));

        let mut invalid = HeaderMap::new();
        invalid.insert(&ORGANIZATION_ID, HeaderValue::from_static("org dave"));
        invalid.insert(&PROJECT_ID, HeaderValue::from_static("prj_quantpilot"));
        invalid.insert(&ENVIRONMENT_ID, HeaderValue::from_static("env_test"));
        assert!(matches!(
            request_tenant_scope(&invalid, &bound),
            Err(crate::error::AppError::InvalidRequest(_))
        ));

        assert_eq!(
            request_tenant_scope(&HeaderMap::new(), &bound).unwrap(),
            bound
        );
    }

    #[test]
    fn traffic_class_is_bounded_and_defaults_to_business() {
        assert_eq!(
            request_traffic_class(&HeaderMap::new()).unwrap(),
            "business"
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-modelport-traffic-class",
            HeaderValue::from_static("synthetic"),
        );
        assert_eq!(request_traffic_class(&headers).unwrap(), "synthetic");
        headers.insert(
            "x-modelport-traffic-class",
            HeaderValue::from_static("arbitrary-cardinality"),
        );
        assert!(request_traffic_class(&headers).is_err());
    }

    #[test]
    fn provider_retry_delay_is_exponential_and_caps_retry_after() {
        let policy = crate::config::ProviderRetryConfig {
            max_attempts: 3,
            initial_delay_ms: 250,
            max_delay_ms: 5_000,
            jitter_ratio: 0.0,
        };
        assert_eq!(provider_retry_delay(&policy, 0, None).as_millis(), 250);
        assert_eq!(provider_retry_delay(&policy, 1, None).as_millis(), 500);
        assert_eq!(
            provider_retry_delay(&policy, 1, Some(60)).as_millis(),
            5_000
        );
    }

    #[test]
    fn context_budget_accepts_reasoning_within_recommended_limit() {
        validate_context_budget(92_000, 32_768, 131_072, Some(94_208), false)
            .expect("92K reasoning input should remain admissible");
    }

    #[test]
    fn output_budget_prefers_logical_model_limit() {
        let admission = TokenCountingConfig {
            mode: TokenCountingMode::Anthropic,
            context_tokens: Some(131_072),
            recommended_reasoning_input_tokens: Some(94_208),
            model_recommended_input_tokens: Default::default(),
            max_output_tokens: Some(32_768),
            model_max_output_tokens: std::collections::HashMap::from([
                ("qwen3.5-fast".to_owned(), 4_096),
                ("qwen3.5-deep".to_owned(), 32_768),
            ]),
        };

        validate_output_budget("qwen3.5-fast", 4_096, &admission)
            .expect("the logical-model boundary is inclusive");
        let error = validate_output_budget("qwen3.5-fast", 4_097, &admission)
            .expect_err("the logical-model limit must be enforced");
        assert!(error.to_string().contains("logical model `qwen3.5-fast`"));
        assert!(error.to_string().contains("configured limit=4096"));
        validate_output_budget("local_qwen:qwen3.5-9b-q5km", 32_768, &admission)
            .expect("direct selectors use the provider-wide limit");
    }

    #[test]
    fn recommended_input_prefers_logical_model_working_set() {
        let admission = TokenCountingConfig {
            mode: TokenCountingMode::Anthropic,
            context_tokens: Some(131_072),
            recommended_reasoning_input_tokens: Some(94_208),
            model_recommended_input_tokens: std::collections::HashMap::from([(
                "qwen3.5-code".to_owned(),
                57_344,
            )]),
            max_output_tokens: None,
            model_max_output_tokens: Default::default(),
        };

        assert_eq!(
            recommended_input_limit("qwen3.5-code", &admission),
            Some(57_344)
        );
        assert_eq!(
            recommended_input_limit("local_qwen:qwen3.5-9b-q5km", &admission),
            Some(94_208)
        );
    }

    #[test]
    fn context_budget_rejects_hard_overflow_without_truncation() {
        let error = validate_context_budget(120_000, 16_384, 131_072, Some(94_208), true)
            .expect_err("hard context overflow must be rejected");
        let message = error.to_string();
        assert!(message.contains("exceeds context_tokens=131072"));
        assert!(message.contains("never silently truncated"));
    }

    #[test]
    fn context_budget_requires_explicit_no_thinking_above_recommended_limit() {
        let error = validate_context_budget(100_000, 8_192, 131_072, Some(94_208), false)
            .expect_err("reasoning above the production input ceiling must be rejected");
        assert!(
            error
                .to_string()
                .contains("recommended_reasoning_input_tokens=94208")
        );
        validate_context_budget(100_000, 8_192, 131_072, Some(94_208), true)
            .expect("explicitly disabled thinking may use the remaining hard context");
    }

    #[test]
    fn tool_outcome_distinguishes_lifecycle_and_protocol_failures() {
        let empty = ResponseObservation::default();
        assert_eq!(
            classify_tool_outcome(false, false, true, false, "completed", None, &empty),
            "not_requested"
        );
        assert_eq!(
            classify_tool_outcome(
                true,
                false,
                true,
                false,
                "completed",
                None,
                &ResponseObservation {
                    tool_call_count: 1,
                    ..ResponseObservation::default()
                },
            ),
            "tool_called"
        );
        assert_eq!(
            classify_tool_outcome(
                true,
                true,
                true,
                false,
                "completed",
                None,
                &ResponseObservation {
                    text_present: true,
                    ..ResponseObservation::default()
                },
            ),
            "final_answer"
        );
        assert_eq!(
            classify_tool_outcome(
                true,
                false,
                false,
                false,
                "downstream_cancelled",
                None,
                &empty,
            ),
            "client_cancelled"
        );
        assert_eq!(
            classify_tool_outcome(true, false, false, true, "upstream_timeout", None, &empty,),
            "timeout"
        );
        assert_eq!(
            classify_tool_outcome(
                true,
                false,
                false,
                false,
                "upstream_protocol_error",
                Some("tool_use input_json is invalid"),
                &empty,
            ),
            "protocol_error"
        );
        assert_eq!(
            classify_tool_outcome(
                true,
                false,
                false,
                false,
                "upstream_error",
                Some("HTTP 502"),
                &empty,
            ),
            "upstream_or_delivery_error"
        );
    }
}
