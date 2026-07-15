use std::{collections::BTreeSet, net::SocketAddr};

use axum::{
    Json,
    body::Body,
    extract::{State, connect_info::ConnectInfo},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tracing::{error, info};

use crate::{
    config::{AppConfig, ProviderConfig, ProviderProtocol, ResolvedProvider},
    control::{UsageEstimate, UsageEventInput},
    domain::{AttemptId, RequestContext, RequestId},
    enterprise_ledger::{LedgerAttempt, LedgerLease, LedgerOutcome, LedgerRequest},
    exchange::{ClientRequest, ExchangeRequest, OpenAiChatRequest},
    pricing::{self, TokenUsageBreakdown},
    providers,
    stream_lifecycle::{StreamLifecycle, StreamTerminalOutcome, UpstreamStreamState},
    types::{AnthropicRequest, validate_anthropic_tooling},
};

use super::*;

pub(super) async fn models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let started = Instant::now();
    let result = (|| {
        authenticate_client(&state, &headers)?;
        let config = effective_config(&state);
        let data = public_model_rows(&config);

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

pub(super) fn public_model_rows(config: &AppConfig) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();

    for id in &config.provider_order {
        let Some(provider) = config.providers.get(id) else {
            continue;
        };
        if !provider_is_configured(provider) {
            continue;
        }

        for model in &provider.models {
            if seen.insert(model.clone()) {
                models.push(json!({
                    "id": model,
                    "type": "model",
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
        if !provider_is_configured(&resolved.provider) {
            continue;
        }
        if seen.insert(alias.clone()) {
            models.push(json!({
                "id": alias,
                "type": "model",
                "display_name": public_model_display_name(&resolved.provider_id, &resolved.provider, &resolved.model),
            }));
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
    credential_id: Option<String>,
    estimate: UsageEstimate,
    stream_lifecycle: StreamLifecycle,
    ledger_attempt: LedgerAttempt,
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
    matches!(
        provider_id,
        "ollama" | "local_sglang" | "local_vllm" | "local_llamacpp"
    ) || matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1")
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
    let identity = match authenticate_client(&state, &headers) {
        Ok(identity) => identity,
        Err(err) => {
            state
                .metrics
                .record_route(route_name, false, started.elapsed());
            return Err(err);
        }
    };
    let validation = match &request {
        ClientRequest::Anthropic(request) => validate_message_request(request),
        ClientRequest::OpenAiChat(request) => validate_openai_chat_request(request),
    };
    if let Err(err) = validation {
        state
            .metrics
            .record_route(route_name, false, started.elapsed());
        return Err(err);
    }
    let exchange = match ExchangeRequest::from_client(request) {
        Ok(exchange) => exchange,
        Err(err) => {
            state
                .metrics
                .record_route(route_name, false, started.elapsed());
            return Err(err);
        }
    };
    let idempotency_key = match request_idempotency_key(&headers) {
        Ok(key) => key,
        Err(err) => {
            state
                .metrics
                .record_route(route_name, false, started.elapsed());
            return Err(err);
        }
    };
    let request_fingerprint = match exchange.request_fingerprint() {
        Ok(fingerprint) => fingerprint,
        Err(err) => {
            state
                .metrics
                .record_route(route_name, false, started.elapsed());
            return Err(err);
        }
    };
    let request_client_ip = client_ip(&headers, Some(peer_addr), &state.trusted_proxies);
    let request_context = RequestContext::legacy(
        RequestId::from_external_or_new(
            headers
                .get(&X_REQUEST_ID)
                .and_then(|value| value.to_str().ok()),
        ),
        identity.user_id.clone(),
        exchange.client_protocol(),
    );
    let requested_model = exchange.requested_model.clone();
    let config = effective_config(&state);
    let resolved = match config.resolve(&requested_model) {
        Ok(resolved) => resolved,
        Err(err) => {
            state
                .metrics
                .record_route(route_name, false, started.elapsed());
            return Err(err);
        }
    };
    if let Err(err) = state.rate_limiter.check(RateLimitScope {
        identity: &identity,
        client_ip: request_client_ip.as_deref(),
        provider_id: None,
        model: None,
    }) {
        state
            .metrics
            .record_route(route_name, false, started.elapsed());
        return Err(err);
    }
    let stream = exchange.stream;
    let stream_permit = if stream {
        match state.stream_permits.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                state
                    .metrics
                    .record_route(route_name, false, started.elapsed());
                return Err(AppError::RateLimited {
                    message: "concurrent stream limit exceeded".to_owned(),
                    retry_after_secs: 1,
                });
            }
        }
    } else {
        None
    };
    let ledger_request = match state
        .ledger
        .begin_request(
            &request_context,
            &requested_model,
            stream,
            idempotency_key.as_deref(),
            &request_fingerprint,
        )
        .await
    {
        Ok(request) => request,
        Err(err) => {
            state
                .metrics
                .record_route(route_name, false, started.elapsed());
            return Err(err);
        }
    };
    let ledger_lease = state.ledger.maintain_lease(&ledger_request);
    info!(
        request_id = request_context.request_id.as_str(),
        organization_id = request_context.tenant.organization_id.as_str(),
        project_id = request_context.tenant.project_id.as_str(),
        environment_id = request_context.tenant.environment_id.as_str(),
        principal_id = request_context.principal_id.as_str(),
        client_protocol = request_context.protocol.as_str(),
        requested_model = exchange.requested_model.as_str(),
        provider = resolved.provider_id.as_str(),
        upstream_model = resolved.model.as_str(),
        stream,
        "routing inference request"
    );

    let attempts = route_attempts(&state, &config, &requested_model, resolved);
    let mut provider_id = String::new();
    let mut upstream_model = String::new();
    let mut protocol = String::new();
    let mut retry_count = 0u32;
    let mut fallback_from_provider = None;
    let mut result = Err(AppError::ProviderNotFound(requested_model.clone()));
    let mut first_sent_provider = None::<String>;
    let mut sent_attempts = 0u32;
    let mut last_sent = None::<SentAttempt>;

    for mut attempt in attempts {
        let attempt_id = AttemptId::new();
        provider_id = attempt.provider_id.clone();
        upstream_model = attempt.model.clone();
        protocol = provider_protocol_value(attempt.provider.protocol).to_owned();
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
        let estimate = estimate_usage(&exchange, &upstream_model);
        if let Err(err) = state.control.check_quotas(
            &identity,
            estimate,
            request_client_ip.as_deref(),
            &requested_model,
            &upstream_model,
            &provider_id,
        ) {
            result = Err(err);
            continue;
        }
        if let Err(err) = validate_inference_attempt(&state, &attempt, &exchange) {
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
            .begin_attempt(
                &ledger_request,
                &attempt_id,
                &provider_id,
                &upstream_model,
                &protocol,
                estimate,
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
            fallback_from_provider = first_sent_provider.clone();
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
        last_sent = Some(SentAttempt {
            attempt_id: attempt_id.clone(),
            provider_id: provider_id.clone(),
            model: upstream_model.clone(),
            protocol: protocol.clone(),
            credential_id: credential_id.clone(),
            estimate,
            stream_lifecycle: stream_lifecycle.clone(),
            ledger_attempt: ledger_attempt.clone(),
        });
        let attempt_result = send_inference_attempt(
            state.clone(),
            attempt,
            exchange.clone(),
            &headers,
            stream_lifecycle,
        )
        .await;
        let attempt_success = attempt_result.is_ok();
        let attempt_status = attempt_result
            .as_ref()
            .map(|response| response.status().as_u16())
            .unwrap_or_else(|error| error.http_status().as_u16());
        let attempt_error = attempt_result.as_ref().err().map(ToString::to_string);
        if !(stream && attempt_success) {
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
            let attempt_estimate = attempt_result
                .as_ref()
                .ok()
                .and_then(|response| pricing::usage_from_headers(response.headers()))
                .map(usage_estimate_from_charge)
                .unwrap_or(estimate);
            let ledger_outcome = LedgerOutcome::provider_attempt(
                attempt_success,
                attempt_status,
                attempt_error.clone(),
                attempt_estimate,
            );
            if let Err(err) = state
                .ledger
                .finalize_attempt(&ledger_attempt, &ledger_outcome)
                .await
            {
                error!(
                    error = %err,
                    request_id = request_context.request_id.as_str(),
                    attempt_id = attempt_id.as_str(),
                    "failed to finalize provider attempt ledger row"
                );
            }
        }
        result = attempt_result;
        if result.is_ok() || !is_retryable_message_error(result.as_ref().err()) {
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
    let error_message = result.as_ref().err().map(ToString::to_string);
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
    let actual_estimate = upstream_usage
        .map(|charge| UsageEstimate {
            input_tokens: charge.input_tokens,
            output_tokens: charge.output_tokens,
            cache_write_tokens: charge.cache_write_tokens,
            cache_read_tokens: charge.cache_read_tokens,
            cost_estimate: charge.cost_estimate,
        })
        .unwrap_or(local_estimate);
    let billing_mode = if upstream_usage.is_some() {
        "upstream-returned"
    } else {
        "local-estimate"
    };

    let usage = UsageEventInput {
        identity,
        request_id: Some(request_context.request_id.to_string()),
        attempt_id: last_sent.as_ref().map(|sent| sent.attempt_id.to_string()),
        model: requested_model,
        resolved_model: upstream_model,
        provider: provider_id,
        protocol,
        client_protocol: request_context.protocol.as_str().to_owned(),
        stream,
        success,
        timed_out,
        status_code,
        terminal_reason: if success {
            "completed"
        } else if timed_out {
            "timeout_before_response"
        } else {
            "failed_before_response"
        }
        .to_owned(),
        estimate: actual_estimate,
        billing_mode: billing_mode.to_owned(),
        chargeable,
        latency: duration,
        first_byte_latency: Some(duration),
        retry_count,
        fallback_from_provider,
        client_ip: request_client_ip,
        request_path: exchange.request_path().to_owned(),
        error_message,
    };

    if stream && success {
        let response = result.expect("successful stream result must contain a response");
        let permit = stream_permit.expect("stream request must hold a stream permit");
        let sent = last_sent.expect("successful stream must have a sent attempt");
        return Ok(response_with_stream_finalizer(
            response,
            permit,
            StreamFinalizationContext {
                state,
                usage,
                credential_id: sent.credential_id,
                lifecycle: sent.stream_lifecycle,
                ledger_request,
                ledger_attempt: sent.ledger_attempt,
                _ledger_lease: ledger_lease,
                started,
                route_name,
            },
        ));
    }

    state.metrics.record_route(route_name, success, duration);
    state.metrics.record_message(
        &usage.provider,
        &usage.resolved_model,
        stream,
        success,
        duration,
        actual_estimate,
    );
    let usage_for_ledger = usage.clone();
    if let Err(err) = state.control.record_usage(usage) {
        error!(
            error = %err,
            request_id = request_context.request_id.as_str(),
            "failed to persist usage after handling upstream response"
        );
    }
    let ledger_outcome = LedgerOutcome::from_usage(&usage_for_ledger);
    if let Err(err) = state
        .ledger
        .finalize_request(&ledger_request, &ledger_outcome)
        .await
    {
        error!(
            error = %err,
            request_id = request_context.request_id.as_str(),
            "failed to finalize request ledger row"
        );
    }
    result
}

fn usage_estimate_from_charge(charge: pricing::UsageCharge) -> UsageEstimate {
    UsageEstimate {
        input_tokens: charge.input_tokens,
        output_tokens: charge.output_tokens,
        cache_write_tokens: charge.cache_write_tokens,
        cache_read_tokens: charge.cache_read_tokens,
        cost_estimate: charge.cost_estimate,
    }
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

struct StreamFinalizationContext {
    state: AppState,
    usage: UsageEventInput,
    credential_id: Option<String>,
    lifecycle: StreamLifecycle,
    ledger_request: LedgerRequest,
    ledger_attempt: LedgerAttempt,
    _ledger_lease: LedgerLease,
    started: Instant,
    route_name: &'static str,
}

impl StreamFinalizationContext {
    fn finalize(mut self, outcome: StreamTerminalOutcome) {
        let duration = self.started.elapsed();
        if let Some(usage) = self.lifecycle.usage() {
            let charge = pricing::charge_for_model(&self.usage.resolved_model, usage);
            self.usage.estimate = UsageEstimate {
                input_tokens: charge.input_tokens,
                output_tokens: charge.output_tokens,
                cache_write_tokens: charge.cache_write_tokens,
                cache_read_tokens: charge.cache_read_tokens,
                cost_estimate: charge.cost_estimate,
            };
            self.usage.billing_mode = "upstream-returned".to_owned();
        }
        self.usage.success = outcome.success();
        self.usage.timed_out = outcome.timed_out();
        self.usage.status_code = outcome.status_code();
        self.usage.terminal_reason = outcome.terminal_reason().to_owned();
        self.usage.error_message = outcome.error_message().map(str::to_owned);
        self.usage.latency = duration;

        self.state
            .metrics
            .record_route(self.route_name, self.usage.success, duration);
        self.state.metrics.record_message(
            &self.usage.provider,
            &self.usage.resolved_model,
            true,
            self.usage.success,
            duration,
            self.usage.estimate,
        );

        if let Some((success, status_code, error_message)) =
            provider_terminal_outcome(&outcome, &self.lifecycle)
            && let Err(err) = self.state.control.record_provider_outcome_for_credential(
                &self.usage.provider,
                self.credential_id.as_deref(),
                success,
                status_code,
                error_message.as_deref(),
                false,
            )
        {
            error!(
                error = %err,
                request_id = self.usage.request_id.as_deref().unwrap_or("unknown"),
                attempt_id = self.usage.attempt_id.as_deref().unwrap_or("unknown"),
                "failed to record provider stream outcome"
            );
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
        let ledger_outcome = LedgerOutcome::from_usage(&self.usage);
        let ledger = self.state.ledger.clone();
        let ledger_request = self.ledger_request;
        let ledger_attempt = self.ledger_attempt;
        let request_id = self.usage.request_id.clone();
        if let Err(err) = self.state.control.record_usage(self.usage) {
            error!(
                error = %err,
                "failed to persist usage after finalizing message stream"
            );
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(err) = ledger
                    .finalize_attempt(&ledger_attempt, &ledger_outcome)
                    .await
                {
                    error!(
                        error = %err,
                        request_id = request_id.as_deref().unwrap_or("unknown"),
                        "failed to finalize streaming attempt ledger row"
                    );
                }
                if let Err(err) = ledger
                    .finalize_request(&ledger_request, &ledger_outcome)
                    .await
                {
                    error!(
                        error = %err,
                        request_id = request_id.as_deref().unwrap_or("unknown"),
                        "failed to finalize streaming request ledger row"
                    );
                }
            });
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
        StreamTerminalOutcome::UpstreamFailed(error) => {
            Some((false, upstream_failure_status(error), Some(error.clone())))
        }
        StreamTerminalOutcome::DeliveryFailed(_)
        | StreamTerminalOutcome::DownstreamCancelled { .. } => match lifecycle.state() {
            UpstreamStreamState::Completed => Some((true, 200, None)),
            UpstreamStreamState::Failed(error) => {
                Some((false, upstream_failure_status(&error), Some(error)))
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
        ProviderProtocol::OpenaiCompat => {
            providers::openai_compat::messages(state, resolved, request, headers, stream_lifecycle)
                .await
                .map(IntoResponse::into_response)
        }
    }
}

fn validate_inference_attempt(
    state: &AppState,
    resolved: &ResolvedProvider,
    request: &ExchangeRequest,
) -> Result<(), AppError> {
    crate::config::validate_provider_base_url_for_request(
        &resolved.provider_id,
        &resolved.provider.base_url,
        state.security.allow_private_provider_urls,
    )?;
    if resolved.provider.api_key_required {
        let _ = resolved.provider.api_key()?;
    }
    request.validate_provider(resolved)?;
    Ok(())
}

fn route_attempts(
    state: &AppState,
    config: &AppConfig,
    requested_model: &str,
    primary: ResolvedProvider,
) -> Vec<ResolvedProvider> {
    let mut attempts = Vec::new();
    if !state.control.provider_in_cooldown(&primary.provider_id) {
        attempts.push(primary.clone());
    }

    for provider_id in &config.provider_order {
        if provider_id == &primary.provider_id || state.control.provider_in_cooldown(provider_id) {
            continue;
        }
        let Some(provider) = config.providers.get(provider_id) else {
            continue;
        };
        let Some(model) = fallback_model_for_provider(provider, requested_model, &primary.model)
        else {
            continue;
        };
        attempts.push(ResolvedProvider {
            provider_id: provider_id.clone(),
            provider: provider.clone(),
            model,
        });
    }

    if attempts.is_empty() {
        attempts.push(primary);
    }
    attempts
}

fn fallback_model_for_provider(
    provider: &ProviderConfig,
    requested_model: &str,
    primary_model: &str,
) -> Option<String> {
    for model in [requested_model, primary_model] {
        if provider.models.iter().any(|configured| configured == model)
            || provider
                .model_prefixes
                .iter()
                .any(|prefix| model.starts_with(prefix))
            || provider.passthrough_unknown_models
        {
            return Some(model.to_owned());
        }
    }
    None
}

fn is_retryable_message_error(error: Option<&AppError>) -> bool {
    match error {
        Some(AppError::Transport(_) | AppError::UpstreamProtocol(_)) => true,
        Some(AppError::Upstream { status, .. }) => *status == 429 || *status >= 500,
        _ => false,
    }
}

fn validate_message_request(request: &AnthropicRequest) -> Result<(), AppError> {
    let max_model_name_chars = env_usize("MODELPORT_MAX_MODEL_NAME_CHARS", 240);
    let max_messages = env_usize("MODELPORT_MAX_MESSAGES", 200);
    let max_messages_json_chars = env_usize("MODELPORT_MAX_MESSAGES_JSON_CHARS", 2 * 1024 * 1024);
    let max_system_json_chars = env_usize("MODELPORT_MAX_SYSTEM_JSON_CHARS", 256 * 1024);
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

fn estimate_usage(request: &ExchangeRequest, resolved_model: &str) -> UsageEstimate {
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
        cost_estimate: pricing::cost_for_model(
            resolved_model,
            TokenUsageBreakdown {
                input_tokens,
                output_tokens,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
    }
}
