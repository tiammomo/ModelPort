use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, HeaderName, LOCATION, SET_COOKIE},
    },
    middleware,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tower::{ServiceBuilder, limit::ConcurrencyLimitLayer};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{info_span, warn};
use uuid::Uuid;

use crate::{
    auth::{AuthStore, FederatedLoginInput, LoginInput, PublicUser},
    config::{
        AppConfig, FidelityMode, MaxTokensField, ProviderConfig, ProviderProtocol, RuntimeConfig,
        TokenCountingMode, ToolResponseValidation, ToolUseConfig,
    },
    control::{
        ClientIdentity, ControlStore, ProviderCredentialRecord, ProviderModelOverrideRecord,
        ProviderOverrideRecord, UpsertQuotaInput, UpsertTeamInput,
    },
    control_view::provider_credential_row,
    enterprise_ledger::{
        AuditEventInput, EnterpriseBudgetAdjustmentInput, EnterpriseBudgetScopeQuery,
        EnterpriseBudgetUpdate, EnterpriseLedger, EnterpriseLedgerQuery, RetentionPolicy,
    },
    error::AppError,
    finalization::FinalizationTracker,
    governance::{GovernanceStore, LocalScheduler},
    http::{Header, HttpTransport},
    metrics::Metrics,
    model_catalog::{ModelProfileOverride, ModelProfileSource},
    oidc::{OIDC_FLOW_COOKIE, OidcService},
    smart_router::SmartRouter,
};

mod admin_api_keys;
mod admin_auth;
mod admin_control;
mod admin_evidence;
mod admin_identity;
mod admin_providers;
mod admin_users;
mod client_api;
mod dashboard_view;
#[path = "routes/governance.rs"]
mod governance_routes;
mod logs_view;
mod ops;
mod ops_agent;
mod provider_view;
#[cfg(test)]
mod route_contract;
mod settings_view;

use dashboard_view::{DashboardQuery, dashboard_body};
use logs_view::{LogsQuery, latency_body, log_belongs_to_user, log_body, logs_body};
use provider_view::{
    catalog_alias_rows, catalog_provider_rows, provider_model_row, provider_row_by_id,
    provider_rows,
};
use settings_view::{alias_row, alias_rows, config_issues_json, settings_row};

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");
const TRAFFIC_CLASS: HeaderName = HeaderName::from_static("x-modelport-traffic-class");
const ROUTING_PROFILE: HeaderName = HeaderName::from_static("x-modelport-routing-profile");
const ROUTING_SESSION_ID: HeaderName = HeaderName::from_static("x-modelport-session-id");
const ROUTING_DECISION_ID: HeaderName = HeaderName::from_static("x-modelport-routing-decision-id");
const ROUTING_MODE: HeaderName = HeaderName::from_static("x-modelport-routing-mode");
const LOGICAL_MODEL: HeaderName = HeaderName::from_static("x-modelport-logical-model");
const RESOLVED_PROVIDER: HeaderName = HeaderName::from_static("x-modelport-resolved-provider");
const RESOLVED_MODEL: HeaderName = HeaderName::from_static("x-modelport-resolved-model");
const ROUTING_POLICY: HeaderName = HeaderName::from_static("x-modelport-routing-policy");
const CLOUD_EGRESS: HeaderName = HeaderName::from_static("x-modelport-cloud-egress");
const HYBRID_MODE: HeaderName = HeaderName::from_static("x-modelport-hybrid-mode");
const DATA_CLASSIFICATION: HeaderName = HeaderName::from_static("x-modelport-data-classification");
const EXECUTION_MODE: HeaderName = HeaderName::from_static("x-modelport-execution-mode");
const CHANGE_REQUEST_ID: HeaderName = HeaderName::from_static("x-modelport-change-request-id");
const ORGANIZATION_ID: HeaderName = HeaderName::from_static("x-modelport-organization-id");
const PROJECT_ID: HeaderName = HeaderName::from_static("x-modelport-project-id");
const ENVIRONMENT_ID: HeaderName = HeaderName::from_static("x-modelport-environment-id");
const CSRF_HEADER: HeaderName = HeaderName::from_static("x-modelport-csrf");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
const ERROR_CONTRACT: HeaderName = HeaderName::from_static("x-modelport-error-contract");
static ADMIN_LOGIN_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
const RETENTION_PREVIEW_TTL_MS: u64 = 5 * 60 * 1_000;

#[derive(Debug, Clone)]
struct RetentionPreview {
    actor_id: String,
    policy: RetentionPolicy,
    evaluated_at_ms: u64,
    expires_at_ms: u64,
}

#[derive(Debug)]
pub(crate) struct RetentionPreviewStore {
    ttl_ms: u64,
    inner: Mutex<BTreeMap<String, RetentionPreview>>,
}

impl Default for RetentionPreviewStore {
    fn default() -> Self {
        Self {
            ttl_ms: RETENTION_PREVIEW_TTL_MS,
            inner: Mutex::new(BTreeMap::new()),
        }
    }
}

impl RetentionPreviewStore {
    fn issue(
        &self,
        actor_id: &str,
        policy: RetentionPolicy,
        evaluated_at_ms: u64,
        now_ms: u64,
    ) -> (String, u64) {
        let expires_at_ms = now_ms.saturating_add(self.ttl_ms);
        let token = format!("rtp_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let preview = RetentionPreview {
            actor_id: actor_id.to_owned(),
            policy,
            evaluated_at_ms,
            expires_at_ms,
        };
        let mut inner = self
            .inner
            .lock()
            .expect("retention preview store lock poisoned");
        // A new preview supersedes the actor's older preview and opportunistically
        // removes expired entries, bounding normal single-instance Beta usage.
        inner
            .retain(|_, existing| existing.expires_at_ms > now_ms && existing.actor_id != actor_id);
        inner.insert(token.clone(), preview);
        (token, expires_at_ms)
    }

    fn consume(
        &self,
        actor_id: &str,
        token: &str,
        now_ms: u64,
    ) -> Result<RetentionPreview, AppError> {
        let mut inner = self
            .inner
            .lock()
            .expect("retention preview store lock poisoned");
        let Some(preview) = inner.get(token).cloned() else {
            return Err(retention_preview_unavailable());
        };
        if preview.expires_at_ms <= now_ms {
            inner.remove(token);
            return Err(retention_preview_unavailable());
        }
        if preview.actor_id != actor_id {
            return Err(retention_preview_unavailable());
        }
        // Consume before the destructive operation. An uncertain storage error,
        // legal hold, or successful apply must all require a fresh preview.
        inner.remove(token);
        Ok(preview)
    }

    #[cfg(test)]
    fn with_ttl_ms(ttl_ms: u64) -> Self {
        Self {
            ttl_ms,
            inner: Mutex::new(BTreeMap::new()),
        }
    }
}

fn retention_preview_unavailable() -> AppError {
    AppError::StateConflict(
        "retention preview is missing, expired, already used, or belongs to another administrator; run a new preview"
            .to_owned(),
    )
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RuntimeConfig>,
    pub auth: Arc<AuthStore>,
    pub oidc: Arc<OidcService>,
    pub control: Arc<ControlStore>,
    pub security: Arc<GatewaySecurityPolicy>,
    pub rate_limiter: Arc<RateLimiter>,
    pub stream_permits: Arc<tokio::sync::Semaphore>,
    pub trusted_proxies: Arc<TrustedProxyConfig>,
    pub transport: HttpTransport,
    pub metrics: Arc<Metrics>,
    pub(crate) smart_router: Arc<SmartRouter>,
    pub(crate) governance: Arc<GovernanceStore>,
    pub(crate) local_scheduler: Arc<LocalScheduler>,
    pub(crate) ledger: Arc<EnterpriseLedger>,
    pub(crate) finalizers: Arc<FinalizationTracker>,
    pub(crate) draining: Arc<AtomicBool>,
    pub(crate) retention_previews: Arc<RetentionPreviewStore>,
}

impl AppState {
    pub(crate) fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct GatewaySecurityPolicy {
    allow_legacy_client_auth: bool,
    expose_detailed_public_health: bool,
    allow_private_provider_urls: bool,
    require_dual_approval: bool,
}

#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    inner: Mutex<RateLimitState>,
}

#[derive(Debug, Clone)]
struct RateLimitConfig {
    enabled: bool,
    window_ms: u64,
    global_per_minute: u32,
    api_key_per_minute: u32,
    ip_per_minute: u32,
    provider_per_minute: u32,
    model_per_minute: u32,
}

#[derive(Debug, Default)]
struct RateLimitState {
    windows: BTreeMap<String, VecDeque<u64>>,
}

#[derive(Debug)]
struct RateLimitScope<'a> {
    identity: &'a ClientIdentity,
    client_ip: Option<&'a str>,
    provider_id: Option<&'a str>,
    model: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct TrustedProxyConfig {
    rules: Vec<IpRule>,
}

#[derive(Debug, Clone)]
enum IpRule {
    Exact(IpAddr),
    Cidr { base: IpAddr, prefix: u8 },
}

impl TrustedProxyConfig {
    pub fn from_env() -> Result<Self, AppError> {
        let value = env::var("MODELPORT_TRUSTED_PROXIES").ok();
        Self::from_value(value.as_deref())
    }

    pub(crate) fn from_value(value: Option<&str>) -> Result<Self, AppError> {
        let mut rules = vec![
            IpRule::Exact(IpAddr::from([127, 0, 0, 1])),
            IpRule::Exact(IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])),
        ];

        if let Some(value) = value {
            for item in value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
            {
                rules.push(parse_ip_rule(item).map_err(|_| {
                    AppError::Config(format!("invalid MODELPORT_TRUSTED_PROXIES entry: {item}"))
                })?);
            }
        }

        Ok(Self { rules })
    }

    #[cfg(test)]
    fn for_tests() -> Self {
        Self {
            rules: vec![
                IpRule::Exact(IpAddr::from([127, 0, 0, 1])),
                IpRule::Exact(IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])),
            ],
        }
    }

    fn is_trusted(&self, ip: IpAddr) -> bool {
        ip.is_loopback() || self.rules.iter().any(|rule| ip_rule_matches(rule, ip))
    }
}

pub(crate) fn validate_allowed_origins_from_env() -> Result<(), AppError> {
    let value = env::var("MODELPORT_ALLOWED_ORIGINS").ok();
    validate_allowed_origins(value.as_deref())
}

fn validate_allowed_origins(value: Option<&str>) -> Result<(), AppError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    for origin in value.split(',').map(str::trim) {
        let authority = origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"))
            .filter(|authority| !authority.is_empty())
            .ok_or_else(|| {
                AppError::Config(
                    "MODELPORT_ALLOWED_ORIGINS entries must be absolute http:// or https:// origins"
                        .to_owned(),
                )
            })?;
        if authority.contains(['/', '?', '#', '@'])
            || authority.parse::<axum::http::uri::Authority>().is_err()
        {
            return Err(AppError::Config(
                "MODELPORT_ALLOWED_ORIGINS entries must contain only scheme, host, and optional port"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

impl GatewaySecurityPolicy {
    pub fn from_env() -> Self {
        Self {
            allow_legacy_client_auth: !env_flag("MODELPORT_REQUIRE_CONTROL_API_KEYS"),
            expose_detailed_public_health: env_flag("MODELPORT_EXPOSE_DETAILED_HEALTH"),
            allow_private_provider_urls: env_flag("MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS"),
            require_dual_approval: env_flag("MODELPORT_ENTERPRISE_MODE")
                || env_flag("MODELPORT_REQUIRE_DUAL_APPROVAL"),
        }
    }

    pub(crate) fn allow_private_provider_urls(&self) -> bool {
        self.allow_private_provider_urls
    }

    #[cfg(test)]
    fn for_tests() -> Self {
        Self {
            allow_legacy_client_auth: true,
            expose_detailed_public_health: false,
            allow_private_provider_urls: true,
            require_dual_approval: false,
        }
    }

    #[cfg(test)]
    fn require_control_api_keys_for_tests() -> Self {
        Self {
            allow_legacy_client_auth: false,
            expose_detailed_public_health: false,
            allow_private_provider_urls: true,
            require_dual_approval: false,
        }
    }
}

impl RateLimiter {
    pub fn from_env() -> Self {
        Self {
            config: RateLimitConfig {
                enabled: !env_flag("MODELPORT_RATE_LIMIT_DISABLED"),
                window_ms: env_u64("MODELPORT_RATE_LIMIT_WINDOW_SECONDS", 60).saturating_mul(1_000),
                global_per_minute: env_u32("MODELPORT_RATE_LIMIT_GLOBAL_PER_MINUTE", 6_000),
                api_key_per_minute: env_u32("MODELPORT_RATE_LIMIT_API_KEY_PER_MINUTE", 600),
                ip_per_minute: env_u32("MODELPORT_RATE_LIMIT_IP_PER_MINUTE", 1_200),
                provider_per_minute: env_u32("MODELPORT_RATE_LIMIT_PROVIDER_PER_MINUTE", 3_000),
                model_per_minute: env_u32("MODELPORT_RATE_LIMIT_MODEL_PER_MINUTE", 1_200),
            },
            inner: Mutex::new(RateLimitState::default()),
        }
    }

    #[cfg(test)]
    fn disabled() -> Self {
        Self {
            config: RateLimitConfig {
                enabled: false,
                window_ms: 60_000,
                global_per_minute: 0,
                api_key_per_minute: 0,
                ip_per_minute: 0,
                provider_per_minute: 0,
                model_per_minute: 0,
            },
            inner: Mutex::new(RateLimitState::default()),
        }
    }

    #[cfg(test)]
    fn for_tests(
        api_key_per_minute: u32,
        ip_per_minute: u32,
        provider_per_minute: u32,
        model_per_minute: u32,
    ) -> Self {
        Self {
            config: RateLimitConfig {
                enabled: true,
                window_ms: 60_000,
                global_per_minute: 0,
                api_key_per_minute,
                ip_per_minute,
                provider_per_minute,
                model_per_minute,
            },
            inner: Mutex::new(RateLimitState::default()),
        }
    }

    fn check(&self, scope: RateLimitScope<'_>) -> Result<(), AppError> {
        if !self.config.enabled {
            return Ok(());
        }

        let mut rules = vec![
            (
                "global:inference".to_owned(),
                self.config.global_per_minute,
                "global request rate limit exceeded",
            ),
            (
                format!(
                    "api-key:{}",
                    scope
                        .identity
                        .quota_subject_id
                        .as_deref()
                        .unwrap_or("legacy-router-token")
                ),
                self.config.api_key_per_minute,
                "API key request rate limit exceeded",
            ),
        ];

        if let Some(client_ip) = scope.client_ip {
            rules.push((
                format!("ip:{client_ip}"),
                self.config.ip_per_minute,
                "client IP request rate limit exceeded",
            ));
        }

        if let Some(provider_id) = scope.provider_id {
            rules.push((
                format!("provider:{provider_id}"),
                self.config.provider_per_minute,
                "provider request rate limit exceeded",
            ));
        }

        if let Some(model) = scope.model {
            rules.push((
                format!("model:{model}"),
                self.config.model_per_minute,
                "model request rate limit exceeded",
            ));
        }

        self.check_rules(rules)
    }

    fn check_provider_attempt(&self, provider_id: &str, model: &str) -> Result<(), AppError> {
        if !self.config.enabled {
            return Ok(());
        }
        self.check_rules(vec![
            (
                format!("provider:{provider_id}"),
                self.config.provider_per_minute,
                "provider request rate limit exceeded",
            ),
            (
                format!("model:{model}"),
                self.config.model_per_minute,
                "model request rate limit exceeded",
            ),
        ])
    }

    fn check_rules(&self, rules: Vec<(String, u32, &'static str)>) -> Result<(), AppError> {
        let now = now_millis();
        let window_start = now.saturating_sub(self.config.window_ms);
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");

        for (key, limit, message) in &rules {
            prune_rate_window(&mut inner, key, window_start);
            if *limit > 0
                && inner
                    .windows
                    .get(key)
                    .is_some_and(|timestamps| timestamps.len() >= *limit as usize)
            {
                let retry_after_secs = inner
                    .windows
                    .get(key)
                    .and_then(|timestamps| timestamps.front().copied())
                    .map(|oldest| {
                        oldest
                            .saturating_add(self.config.window_ms)
                            .saturating_sub(now)
                            .div_ceil(1_000)
                            .max(1)
                    })
                    .unwrap_or(1);
                let window_seconds = self.config.window_ms.div_ceil(1_000).max(1);
                return Err(AppError::RateLimited {
                    message: format!("{message}; limit={limit}/{window_seconds}s"),
                    retry_after_secs,
                });
            }
        }

        for (key, limit, _) in rules {
            if limit == 0 {
                continue;
            }
            inner.windows.entry(key).or_default().push_back(now);
        }

        inner.windows.retain(|_, timestamps| {
            while timestamps
                .front()
                .is_some_and(|timestamp| *timestamp < window_start)
            {
                timestamps.pop_front();
            }
            !timestamps.is_empty()
        });

        Ok(())
    }
}

fn prune_rate_window(state: &mut RateLimitState, key: &str, window_start: u64) {
    if let Some(timestamps) = state.windows.get_mut(key) {
        while timestamps
            .front()
            .is_some_and(|timestamp| *timestamp < window_start)
        {
            timestamps.pop_front();
        }
    }
}

pub fn router(state: AppState) -> Router {
    let config = state.config.snapshot();
    let max_request_body_bytes = config.max_request_body_bytes;
    let max_concurrent_requests = config.max_concurrent_requests;

    Router::new()
        .merge(ops::router())
        .merge(client_api::router())
        .merge(ops_agent::internal_router())
        .merge(admin_auth::router())
        .merge(governance_routes::router())
        .merge(ops_agent::admin_router())
        .merge(admin_providers::router())
        .merge(admin_control::router())
        .merge(admin_evidence::router())
        .merge(admin_identity::router())
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::new(
                    X_REQUEST_ID.clone(),
                    MakeRequestUuid,
                ))
                .layer(PropagateRequestIdLayer::new(X_REQUEST_ID.clone()))
                .layer(TraceLayer::new_for_http().make_span_with(
                    |request: &axum::extract::Request| {
                        // Query strings can contain short-lived authorization codes on
                        // authentication callbacks. Keep correlation metadata while
                        // ensuring those credentials never enter tracing output.
                        info_span!(
                            "http_request",
                            method = %request.method(),
                            path = %request.uri().path(),
                        )
                    },
                ))
                .layer(ConcurrencyLimitLayer::new(max_concurrent_requests)),
        )
        .layer(middleware::from_fn(add_security_headers))
        .layer(DefaultBodyLimit::max(max_request_body_bytes))
        .with_state(state)
}

async fn add_security_headers(request: axum::extract::Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        X_CONTENT_TYPE_OPTIONS.clone(),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(X_FRAME_OPTIONS.clone(), HeaderValue::from_static("DENY"));
    headers.insert(
        REFERRER_POLICY.clone(),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        PERMISSIONS_POLICY.clone(),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    attach_error_request_id(response).await
}

async fn attach_error_request_id(mut response: Response) -> Response {
    if response.headers().get(&ERROR_CONTRACT).is_none() {
        return response;
    }
    response.headers_mut().remove(&ERROR_CONTRACT);
    let request_id = response
        .headers()
        .get(&X_REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };
    let Some(request_id) = request_id else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let Some(error) = value.get_mut("error").and_then(Value::as_object_mut) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    error.insert("request_id".to_owned(), Value::String(request_id));
    let Ok(bytes) = serde_json::to_vec(&value) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(bytes))
}

async fn add_no_store_header(request: axum::extract::Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    set_no_store(&mut response);
    response
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OidcStartQuery {
    return_to: Option<String>,
}

#[derive(Deserialize)]
struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn admin_auth_methods(State(state): State<AppState>) -> Response {
    let mut response = Json(state.oidc.methods()).into_response();
    set_no_store(&mut response);
    response
}

async fn admin_oidc_start(
    State(state): State<AppState>,
    Query(query): Query<OidcStartQuery>,
) -> Response {
    match state.oidc.start(query.return_to.as_deref()).await {
        Ok(start) => redirect_no_store(&start.authorization_url, &[start.flow_cookie]),
        Err(error) => oidc_error_redirect(error.code(), &state.oidc.clear_flow_cookie()),
    }
}

async fn admin_oidc_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    query: Result<Query<OidcCallbackQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let browser_flow = cookie_value(&headers, OIDC_FLOW_COOKIE).unwrap_or("");
    let clear_flow_cookie = state.oidc.clear_flow_cookie();
    let Query(mut query) = match query {
        Ok(query) => query,
        Err(_) => return oidc_error_redirect("invalid_callback", &clear_flow_cookie),
    };

    if query.error.take().is_some() {
        let result = query
            .state
            .as_deref()
            .ok_or(crate::oidc::OidcFlowError::InvalidCallback)
            .and_then(|state_value| state.oidc.consume_provider_error(state_value, browser_flow));
        let code = result.map_or_else(|error| error.code(), |_| "provider_error");
        return oidc_error_redirect(code, &clear_flow_cookie);
    }

    let code = query.code.take();
    let callback_state = query.state.take();
    let (code, state_value) = match (code, callback_state) {
        (Some(code), Some(state_value)) => (code, state_value),
        (_, callback_state) => {
            if let Some(state_value) = callback_state.as_deref() {
                let _ = state.oidc.consume_provider_error(state_value, browser_flow);
            }
            return oidc_error_redirect("invalid_callback", &clear_flow_cookie);
        }
    };
    let completed = match state.oidc.complete(&code, &state_value, browser_flow).await {
        Ok(completed) => completed,
        Err(error) => return oidc_error_redirect(error.code(), &clear_flow_cookie),
    };

    let auth = state.auth.clone();
    let input = FederatedLoginInput {
        issuer: completed.issuer,
        subject: completed.subject,
        username: completed.username,
        email: completed.email,
        email_verified: completed.email_verified,
        auto_provision: completed.auto_provision,
    };
    let _permit = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ADMIN_LOGIN_WORKERS.acquire(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => {
            return oidc_error_redirect("oidc_unavailable", &clear_flow_cookie);
        }
        Err(_) => {
            return oidc_error_redirect("oidc_unavailable", &clear_flow_cookie);
        }
    };
    let login = match tokio::task::spawn_blocking(move || auth.login_federated(input)).await {
        Ok(Ok(login)) => login,
        Ok(Err(_)) | Err(_) => {
            return oidc_error_redirect("account_not_authorized", &clear_flow_cookie);
        }
    };
    record_admin_activity(
        &state,
        &login.user,
        "config_change",
        format!("user:{}", login.user.id),
        format!("用户 {} 通过 OIDC 登录控制台", login.user.username),
        "info",
    )
    .await;
    let session_cookie = state.auth.session_cookie(&login.session_token);
    redirect_no_store(&completed.return_to, &[clear_flow_cookie, session_cookie])
}

fn oidc_error_redirect(code: &str, clear_flow_cookie: &str) -> Response {
    let location = format!("/login?oidc_error={code}");
    redirect_no_store(&location, &[clear_flow_cookie.to_owned()])
}

fn redirect_no_store(location: &str, cookies: &[String]) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    response.headers_mut().insert(
        LOCATION,
        HeaderValue::from_str(location)
            .unwrap_or_else(|_| HeaderValue::from_static("/login?oidc_error=invalid_callback")),
    );
    for cookie in cookies {
        if let Ok(value) = HeaderValue::from_str(cookie) {
            response.headers_mut().append(SET_COOKIE, value);
        }
    }
    set_no_store(&mut response);
    response
}

fn set_no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (cookie_name, value) = cookie.trim().split_once('=')?;
                (cookie_name == name && !value.is_empty()).then_some(value)
            })
        })
}

async fn admin_login(
    State(state): State<AppState>,
    Json(input): Json<LoginInput>,
) -> Result<Response, AppError> {
    let _permit = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ADMIN_LOGIN_WORKERS.acquire(),
    )
    .await
    .map_err(|_| AppError::RateLimited {
        message: "too many concurrent admin login attempts".to_owned(),
        retry_after_secs: 1,
    })?
    .map_err(|_| AppError::Config("admin login worker limiter closed".to_owned()))?;
    let auth = state.auth.clone();
    let login = tokio::task::spawn_blocking(move || auth.login(input))
        .await
        .map_err(|error| AppError::Config(format!("authentication worker failed: {error}")))??;
    record_admin_activity(
        &state,
        &login.user,
        "config_change",
        format!("user:{}", login.user.id),
        format!("管理员 {} 登录控制台", login.user.username),
        "info",
    )
    .await;
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
    set_no_store(&mut response);
    Ok(response)
}

async fn admin_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    require_console_write_protection(&headers)?;
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

fn require_admin_write_user(state: &AppState, headers: &HeaderMap) -> Result<PublicUser, AppError> {
    require_console_write_protection(headers)?;
    require_admin_user(state, headers)
}

fn require_high_risk_change(
    state: &AppState,
    headers: &HeaderMap,
    action: &str,
    target: &str,
    payload: &Value,
) -> Result<Option<String>, AppError> {
    let request_id = headers
        .get(&CHANGE_REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(request_id) = request_id else {
        if !state.security.require_dual_approval {
            return Ok(None);
        }
        return Err(AppError::Forbidden(
            "high-risk change requires x-modelport-change-request-id with two distinct approvals"
                .to_owned(),
        ));
    };
    state
        .governance
        .verify_approved_change(request_id, action, target, payload)?;
    Ok(Some(request_id.to_owned()))
}

fn mark_high_risk_change_applied(
    state: &AppState,
    approval_id: Option<&str>,
) -> Result<(), AppError> {
    if let Some(approval_id) = approval_id {
        state.governance.mark_change_applied(approval_id)?;
    }
    Ok(())
}

fn require_console_user(state: &AppState, headers: &HeaderMap) -> Result<PublicUser, AppError> {
    state.auth.require_session(headers)
}

fn require_api_key_writer(state: &AppState, headers: &HeaderMap) -> Result<PublicUser, AppError> {
    let user = state.auth.require_session(headers)?;
    if matches!(user.role.as_str(), "admin" | "user") {
        Ok(user)
    } else {
        Err(AppError::Forbidden(
            "API key write access required".to_owned(),
        ))
    }
}

fn require_api_key_write_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<PublicUser, AppError> {
    require_console_write_protection(headers)?;
    require_api_key_writer(state, headers)
}

fn require_console_write_protection(headers: &HeaderMap) -> Result<(), AppError> {
    if env_flag("MODELPORT_DISABLE_CSRF") {
        return Ok(());
    }
    let csrf_ok = headers
        .get(&CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| matches!(value, "1" | "true" | "TRUE"));
    if !csrf_ok {
        return Err(AppError::Forbidden(
            "CSRF protection header is required for console write requests".to_owned(),
        ));
    }
    validate_admin_request_origin(headers)
}

fn validate_admin_request_origin(headers: &HeaderMap) -> Result<(), AppError> {
    let origin = headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .or_else(|| headers.get("referer").and_then(|value| value.to_str().ok()));
    let Some(origin) = origin else {
        return Ok(());
    };
    let Some(origin_host) = host_from_origin(origin) else {
        return Err(AppError::Forbidden(
            "invalid console request origin".to_owned(),
        ));
    };
    let request_host = headers.get("host").and_then(|value| value.to_str().ok());
    let same_origin = request_host.is_some_and(|host| console_host_matches(host, origin_host));
    let allowed_origin = env::var("MODELPORT_ALLOWED_ORIGINS")
        .ok()
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|allowed| allowed.eq_ignore_ascii_case(origin))
        });
    if same_origin || allowed_origin {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "console request origin is not allowed".to_owned(),
        ))
    }
}

fn host_from_origin(value: &str) -> Option<&str> {
    let value = value.trim();
    let without_scheme = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))?;
    without_scheme
        .split('/')
        .next()
        .filter(|host| !host.is_empty())
}

fn console_host_matches(request_host: &str, origin_host: &str) -> bool {
    if request_host.eq_ignore_ascii_case(origin_host) {
        return true;
    }

    let Some(request_hostname) = hostname_from_authority(request_host) else {
        return false;
    };
    let Some(origin_hostname) = hostname_from_authority(origin_host) else {
        return false;
    };

    is_loopback_hostname(request_hostname) && is_loopback_hostname(origin_hostname)
}

fn hostname_from_authority(authority: &str) -> Option<&str> {
    let authority = authority.trim();
    if authority.is_empty() {
        return None;
    }
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().filter(|host| !host.is_empty());
    }
    authority.split(':').next().filter(|host| !host.is_empty())
}

fn is_loopback_hostname(hostname: &str) -> bool {
    let hostname = hostname.trim_matches(['[', ']']).trim_end_matches('.');
    if hostname.eq_ignore_ascii_case("localhost") {
        return true;
    }
    hostname
        .parse::<IpAddr>()
        .is_ok_and(|addr| addr.is_loopback())
}

fn ensure_api_key_access(
    state: &AppState,
    actor: &PublicUser,
    key_id: &str,
) -> Result<(), AppError> {
    if actor.role == "admin" {
        return Ok(());
    }
    let owner = state.control.api_key_user_id(key_id)?;
    if owner == actor.id {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "API key belongs to another user".to_owned(),
        ))
    }
}

async fn record_admin_activity(
    state: &AppState,
    actor: &PublicUser,
    activity_type: &str,
    target: impl Into<String>,
    message: impl Into<String>,
    severity: &str,
) {
    if let Err(err) = state
        .ledger
        .record_audit_event(&AuditEventInput {
            activity_type: activity_type.to_owned(),
            actor_id: actor.id.clone(),
            actor_name: actor.username.clone(),
            target: target.into(),
            message: message.into(),
            severity: severity.to_owned(),
        })
        .await
    {
        warn!(error = %err, "failed to record admin activity");
    }
}

fn authenticate_client(state: &AppState, headers: &HeaderMap) -> Result<ClientIdentity, AppError> {
    if let Some(identity) = state.control.authenticate_headers(headers)? {
        if !state.auth.is_user_active(&identity.user_id) {
            return Err(AppError::Auth);
        }
        return Ok(identity);
    }
    if !state.security.allow_legacy_client_auth {
        return Err(AppError::Forbidden(
            "control-plane API key is required; legacy router token auth is disabled".to_owned(),
        ));
    }
    state.config.snapshot().validate_client_auth(headers)?;
    Ok(ControlStore::legacy_identity())
}

fn authenticate_inference_client(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ClientIdentity, AppError> {
    let identity = authenticate_client(state, headers)?;
    ensure_inference_identity(&identity)?;
    Ok(identity)
}

fn ensure_inference_identity(identity: &ClientIdentity) -> Result<(), AppError> {
    if identity.purpose.as_deref() == Some("modelport_ops_agent") {
        return Err(AppError::Forbidden(
            "operations-agent credentials cannot access the inference data plane".to_owned(),
        ));
    }
    Ok(())
}

fn client_ip(
    headers: &HeaderMap,
    peer_addr: Option<SocketAddr>,
    trusted_proxies: &TrustedProxyConfig,
) -> Option<String> {
    if let Some(peer) = peer_addr
        && trusted_proxies.is_trusted(peer.ip())
        && let Some(ip) = forwarded_client_ip(headers, peer.ip(), trusted_proxies)
    {
        return Some(ip.to_string());
    }

    peer_addr.map(|peer| peer.ip().to_string())
}

fn forwarded_client_ip(
    headers: &HeaderMap,
    peer_ip: IpAddr,
    trusted_proxies: &TrustedProxyConfig,
) -> Option<IpAddr> {
    if let Some(value) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        let mut chain = value
            .split(',')
            .filter_map(parse_ip_with_optional_port)
            .collect::<Vec<_>>();
        if !chain.is_empty() {
            // Walk from the connected peer towards the client and discard only
            // explicitly trusted proxy hops. A caller-controlled leftmost XFF
            // value can therefore never override the address appended by the
            // nearest trusted proxy.
            chain.push(peer_ip);
            if let Some(client) = chain
                .iter()
                .rev()
                .copied()
                .find(|ip| !trusted_proxies.is_trusted(*ip))
            {
                return Some(client);
            }
            return Some(peer_ip);
        }
    }

    for name in ["x-real-ip", "cf-connecting-ip"] {
        if let Some(ip) = headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_ip_with_optional_port)
        {
            return Some(ip);
        }
    }
    None
}

fn parse_ip_with_optional_port(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip);
    }
    value
        .rsplit_once(':')
        .and_then(|(host, _)| host.parse::<IpAddr>().ok())
}

async fn admin_dashboard(
    State(state): State<AppState>,
    Query(query): Query<DashboardQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(dashboard_body(&state, &query).await?))
}

async fn admin_router_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    let config = effective_config(&state);
    Ok(Json(state.smart_router.status(&config)))
}

async fn admin_aliases(
    State(state): State<AppState>,
    Query(query): Query<admin_providers::ProviderCatalogQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    let rows = if actor.role == "admin" {
        alias_rows(&state)
    } else {
        catalog_alias_rows(&state, &actor.id, query.api_key_id.as_deref())
    };
    Ok(Json(Value::Array(rows)))
}

async fn admin_create_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
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
    )
    .await;
    let config = effective_config(&state);
    Ok(Json(alias_row(&config, alias, target)))
}

async fn admin_delete_alias(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(alias): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let tombstone = state.config.snapshot().aliases.contains_key(&alias);
    state.control.delete_alias(&alias, tombstone)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("alias:{alias}"),
        format!("删除模型别名 {alias}"),
        "warning",
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(settings_row(&state)))
}

async fn admin_reload_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let config = state.config.reload()?;
    // A successful probe belongs to the exact endpoint/protocol/credential
    // configuration that was tested. Reloading may change any of those, so
    // stale evidence must not continue to advertise the Provider as verified.
    state.control.clear_provider_tests()?;
    let issues = config_issues_json(&config);
    let warning_count = issues
        .iter()
        .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
        .count();

    record_admin_activity(
        &state,
        &actor,
        "config_change",
        "gateway",
        format!(
            "热重载配置：{} 个供应商，默认供应商 {}",
            config.providers.len(),
            config.default_provider
        ),
        if warning_count > 0 { "warning" } else { "info" },
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "settings": settings_row(&state),
        "providerCount": config.providers.len(),
        "defaultProvider": config.default_provider,
        "providerOrder": config.provider_order,
        "issues": issues,
        "reloadScope": {
            "applied": ["provider catalog", "base provider keys", "base urls", "model lists", "aliases", "legacy client auth token"],
            "requiresRestart": ["bind address", "request body limit", "concurrency layer", "rate limits", "HTTP client timeouts", "trusted proxies", "security flags", "admin session and cookie settings", "storage", "new credential-profile environment variables"],
        },
    })))
}

async fn admin_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    validate_settings_update_body(&body)?;
    let mut changes = Vec::new();
    if let Some(gateway) = body.get("gateway") {
        let config = effective_config(&state);
        if let Some(provider_id) = gateway.get("defaultProvider").and_then(Value::as_str) {
            if !config.providers.contains_key(provider_id) {
                return Err(AppError::ProviderNotFound(provider_id.to_owned()));
            }
            state.control.set_default_provider(provider_id.to_owned())?;
            changes.push(format!("默认供应商设为 {provider_id}"));
        }

        if let Some(order) = gateway.get("providerOrder") {
            let provider_order = parse_provider_order(&config, order)?;
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
        )
        .await;
    }

    Ok(Json(settings_row(&state)))
}

fn validate_settings_update_body(body: &Value) -> Result<(), AppError> {
    let object = body
        .as_object()
        .ok_or_else(|| AppError::InvalidRequest("settings body must be an object".to_owned()))?;
    if let Some(field) = object.keys().find(|field| field.as_str() != "gateway") {
        return Err(AppError::InvalidRequest(format!(
            "settings field `{field}` is read-only; change the deployment configuration and restart"
        )));
    }
    let Some(gateway) = object.get("gateway") else {
        return Ok(());
    };
    let gateway = gateway
        .as_object()
        .ok_or_else(|| AppError::InvalidRequest("settings.gateway must be an object".to_owned()))?;
    if let Some(field) = gateway
        .keys()
        .find(|field| !matches!(field.as_str(), "defaultProvider" | "providerOrder"))
    {
        return Err(AppError::InvalidRequest(format!(
            "settings.gateway field `{field}` is not supported"
        )));
    }
    Ok(())
}

async fn admin_test_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let provider_id = body
        .get("providerId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let config = management_config(&state);
    let Some(provider) = config.providers.get(provider_id).cloned() else {
        return Ok(Json(json!({
            "success": false,
            "message": "provider not found",
        })));
    };

    let probe = run_provider_management_probe(&state, provider_id, &provider).await?;
    let tested_credential_id = probe.tested_credential_id;
    let (success, message, models) = match probe.result {
        Ok(models) => {
            let message = if provider_supports_model_discovery(provider_id, &provider) {
                format!("connected; discovered {} model(s)", models.len())
            } else {
                "configured".to_owned()
            };
            (true, message, models)
        }
        Err(message) => (false, message, Vec::new()),
    };
    let tested_at = state.control.record_provider_test_for_credential(
        provider_id.to_owned(),
        success,
        message.to_owned(),
        models.clone(),
        tested_credential_id.clone(),
    )?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}"),
        format!("测试供应商 {provider_id}: {message}"),
        if success { "info" } else { "warning" },
    )
    .await;

    Ok(Json(json!({
        "success": success,
        "message": message,
        "models": models,
        "modelCount": models.len(),
        "testedCredentialId": tested_credential_id,
        "testedAt": tested_at.to_string(),
    })))
}

async fn admin_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    let (events, total) = state.ledger.audit_events(100).await?;
    Ok(Json(json!({
        "events": events,
        "total": total,
    })))
}

async fn admin_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        "backup",
        "导出控制面诊断快照",
        "info",
    )
    .await;

    let (audit_events, audit_total) = state.ledger.audit_events(1_000).await?;
    Ok(Json(json!({
        "schemaVersion": 2,
        "service": "model-port",
        "build": crate::version::json(),
        "generatedAt": now_millis_string(),
        "containsSecrets": false,
        "containsPersonalData": true,
        "settings": settings_row(&state),
        "users": state.auth.list_users(0),
        "control": state.control.export_snapshot(),
        "audit": {
            "events": audit_events,
            "total": audit_total,
        },
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RetentionRunInput {
    #[serde(default = "default_retention_dry_run")]
    dry_run: bool,
    preview_token: Option<String>,
}

fn default_retention_dry_run() -> bool {
    true
}

async fn admin_run_retention(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RetentionRunInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let (result, preview_token, preview_expires_at_ms) = if input.dry_run {
        let result = state
            .ledger
            .run_retention(RetentionPolicy::from_env()?, true)
            .await?;
        let issued_at_ms = now_millis();
        let (token, expires_at_ms) = state.retention_previews.issue(
            &actor.id,
            result.policy,
            result.evaluated_at_ms,
            issued_at_ms,
        );
        (result, Some(token), Some(expires_at_ms))
    } else {
        let token = input
            .preview_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                AppError::InvalidRequest(
                    "previewToken is required when dryRun is false; run a new preview".to_owned(),
                )
            })?;
        let preview = state
            .retention_previews
            .consume(&actor.id, token, now_millis())?;
        let result = state
            .ledger
            .run_retention_at(preview.policy, false, preview.evaluated_at_ms)
            .await?;
        (result, None, None)
    };
    record_admin_activity(
        &state,
        &actor,
        "data_retention",
        "operational_ledger",
        format!(
            "数据保留{}：请求明细 {}，Provider attempts {}，用户用量 {}，审计事件 {}{}",
            if result.dry_run { "预览" } else { "执行" },
            result.counts.request_details_redacted,
            result.counts.provider_attempts_redacted,
            result.counts.user_usage_rows_deidentified,
            result.counts.audit_events_deleted,
            if result.skipped_reason == Some("legal_hold") {
                "（legal hold，未修改）"
            } else {
                ""
            }
        ),
        if result.applied { "warning" } else { "info" },
    )
    .await;
    let mut body = serde_json::to_value(result)?;
    if let Some(body) = body.as_object_mut() {
        body.insert(
            "previewToken".to_owned(),
            preview_token.map_or(Value::Null, Value::String),
        );
        body.insert(
            "previewExpiresAtMs".to_owned(),
            preview_expires_at_ms.map_or(Value::Null, Value::from),
        );
    }
    Ok(Json(body))
}

struct ProviderManagementProbe {
    tested_credential_id: Option<String>,
    result: Result<Vec<String>, String>,
}

/// Run the management connectivity probe through the exact credential
/// selection path used by inference requests. An explicit pool is an
/// authorization boundary: when it has no selectable member, the provider's
/// static credential must never be used as an implicit fallback.
async fn run_provider_management_probe(
    state: &AppState,
    provider_id: &str,
    provider: &ProviderConfig,
) -> Result<ProviderManagementProbe, AppError> {
    let mut selected_provider = provider.clone();
    let tested_credential_id = match state
        .control
        .apply_selected_provider_credential_for_request(provider_id, &mut selected_provider)
    {
        Ok(credential_id) => credential_id,
        Err(err) => {
            let message = err.audit_message();
            state.control.record_provider_outcome_for_credential(
                provider_id,
                None,
                false,
                err.http_status().as_u16(),
                Some(&message),
                true,
            )?;
            return Ok(ProviderManagementProbe {
                tested_credential_id: None,
                result: Err(message),
            });
        }
    };

    let result = match discover_provider_models(state, provider_id, &selected_provider).await {
        Ok(models) => {
            state.control.record_provider_outcome_for_credential(
                provider_id,
                tested_credential_id.as_deref(),
                true,
                StatusCode::OK.as_u16(),
                None,
                true,
            )?;
            Ok(models)
        }
        Err(err) => {
            let message = err.audit_message();
            state.control.record_provider_outcome_for_credential(
                provider_id,
                tested_credential_id.as_deref(),
                false,
                err.http_status().as_u16(),
                Some(&message),
                true,
            )?;
            Err(message)
        }
    };

    Ok(ProviderManagementProbe {
        tested_credential_id,
        result,
    })
}

async fn discover_provider_models(
    state: &AppState,
    provider_id: &str,
    provider: &ProviderConfig,
) -> Result<Vec<String>, AppError> {
    crate::config::validate_provider_base_url_dns_for_request(
        provider_id,
        &provider.base_url,
        state.security.allow_private_provider_urls,
    )
    .await?;
    if !provider_supports_model_discovery(provider_id, provider) {
        probe_anthropic_provider(state, provider_id, provider).await?;
        return Ok(configured_provider_models(provider));
    }

    let url = if provider_id == "cpa_claude" {
        provider.endpoint("/v1/models")
    } else {
        provider.endpoint("/models")
    };
    let body = state
        .transport
        .get_json(
            provider_id,
            state.security.allow_private_provider_urls(),
            &url,
            &openai_compatible_headers(provider)?,
        )
        .await?;
    let models = if parse_model_ids(&body).is_empty() {
        configured_provider_models(provider)
    } else {
        parse_model_ids(&body)
    };
    if provider.protocol == ProviderProtocol::OpenaiCompat {
        probe_openai_provider(state, provider_id, provider).await?;
    } else {
        probe_anthropic_provider(state, provider_id, provider).await?;
    }
    Ok(models)
}

async fn probe_openai_provider(
    state: &AppState,
    provider_id: &str,
    provider: &ProviderConfig,
) -> Result<(), AppError> {
    let mut body = json!({
        "model": provider.default_model,
        "messages": [{ "role": "user", "content": "Reply OK." }],
        "stream": false,
    });
    match provider.max_tokens_field {
        MaxTokensField::MaxCompletionTokens => body["max_completion_tokens"] = json!(1),
        MaxTokensField::MaxTokens => body["max_tokens"] = json!(1),
        MaxTokensField::Both => {
            body["max_completion_tokens"] = json!(1);
            body["max_tokens"] = json!(1);
        }
    }
    let response = state
        .transport
        .post_json(
            provider_id,
            state.security.allow_private_provider_urls(),
            &provider.endpoint("/chat/completions"),
            &openai_compatible_headers(provider)?,
            &body,
        )
        .await?;
    if response
        .get("choices")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err(AppError::UpstreamProtocol(
            "OpenAI connectivity probe response is missing choices".to_owned(),
        ));
    }
    Ok(())
}

async fn probe_anthropic_provider(
    state: &AppState,
    provider_id: &str,
    provider: &ProviderConfig,
) -> Result<(), AppError> {
    let headers = crate::providers::anthropic::headers(provider, &HeaderMap::new())?;
    let (url, body, expects_token_count) =
        if provider.token_counting.mode == TokenCountingMode::Anthropic {
            (
                provider.endpoint("/v1/messages/count_tokens"),
                json!({
                    "model": provider.default_model,
                    "messages": [{ "role": "user", "content": "ModelPort connectivity probe" }],
                }),
                true,
            )
        } else {
            // Some Anthropic-compatible Providers do not implement count_tokens.
            // A fixed one-token request is the smallest protocol/auth/model probe
            // that produces real connection evidence without sending user data.
            (
                provider.endpoint("/v1/messages"),
                json!({
                    "model": provider.default_model,
                    "max_tokens": 1,
                    "messages": [{ "role": "user", "content": "Reply OK." }],
                }),
                false,
            )
        };
    let response = state
        .transport
        .post_json(
            provider_id,
            state.security.allow_private_provider_urls(),
            &url,
            &headers,
            &body,
        )
        .await?;
    if expects_token_count
        && response
            .get("input_tokens")
            .and_then(Value::as_u64)
            .is_none()
    {
        return Err(AppError::UpstreamProtocol(
            "Anthropic connectivity probe response is missing integer input_tokens".to_owned(),
        ));
    }
    if !expects_token_count && !response.is_object() {
        return Err(AppError::UpstreamProtocol(
            "Anthropic connectivity probe returned a non-object response".to_owned(),
        ));
    }
    Ok(())
}

fn provider_supports_model_discovery(provider_id: &str, provider: &ProviderConfig) -> bool {
    provider.protocol == ProviderProtocol::OpenaiCompat || provider_id == "cpa_claude"
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
    Query(query): Query<LogsQuery>,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    query.validate()?;
    // Request metadata is personal operational data. Read-only viewers do not
    // implicitly gain organization-wide visibility; only administrators can
    // remove this ownership boundary.
    let query = if actor.role == "admin" {
        query
    } else {
        query.scoped_to_user(&actor.id)
    };
    Ok(Json(logs_body(&state, &query).await?))
}

async fn admin_log_by_id(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(log_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    let row = log_body(&state, &log_id).await?;
    // Return the same not-found response for missing and unauthorized rows so
    // a user/viewer cannot enumerate another principal's request IDs.
    row.filter(|row| actor.role == "admin" || log_belongs_to_user(row, &actor.id))
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("request log {log_id}")))
}

async fn admin_latency(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(latency_body(&state).await?))
}

async fn admin_enterprise_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(json!(state.ledger.overview().await?)))
}

async fn admin_enterprise_budget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(scope): Query<EnterpriseBudgetScopeQuery>,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(json!(state.ledger.budget_view(&scope).await?)))
}

async fn admin_update_enterprise_budget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EnterpriseBudgetUpdate>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let approval_payload = serde_json::to_value(&body)?;
    let approval_id = require_high_risk_change(
        &state,
        &headers,
        "budget.hard_limit",
        "enterprise-budget",
        &approval_payload,
    )?;
    let view = state.ledger.update_budget(&body).await?;
    mark_high_risk_change_applied(&state, approval_id.as_deref())?;
    record_admin_activity(
        &state,
        &actor,
        "budget_change",
        "enterprise-budget",
        "更新企业租户预算上限",
        "warning",
    )
    .await;
    Ok(Json(json!(view)))
}

async fn admin_adjust_enterprise_budget(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EnterpriseBudgetAdjustmentInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let view = state.ledger.adjust_budget(&body, &actor.id).await?;
    record_admin_activity(
        &state,
        &actor,
        "budget_adjustment",
        "enterprise-budget",
        "登记带证据引用的企业预算调整",
        "warning",
    )
    .await;
    Ok(Json(json!(view)))
}

async fn admin_enterprise_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<EnterpriseLedgerQuery>,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(json!(state.ledger.list_requests(&query).await?)))
}

async fn admin_enterprise_request_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ledger_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    let detail = state
        .ledger
        .request_detail(&ledger_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("enterprise ledger request {ledger_id}")))?;
    Ok(Json(json!(detail)))
}

async fn admin_teams(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    let mut teams = state.control.list_teams();
    let usage = state.ledger.management_usage().await?;
    for team in &mut teams {
        let Some(team_id) = team.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(stats) = usage.teams.get(team_id) else {
            continue;
        };
        team["requestsToday"] = json!(stats.requests_today);
        team["dailySpendUsd"] = json!(stats.daily_spend_usd);
        team["monthlySpendUsd"] = json!(stats.monthly_spend_usd);
    }
    Ok(Json(json!(teams)))
}

async fn admin_upsert_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertTeamInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let team = state.control.upsert_team(body)?;
    let team_name = team
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let team_id = team.get("id").and_then(Value::as_str).unwrap_or("unknown");
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("team:{team_id}"),
        format!("保存团队/项目 {team_name}"),
        "info",
    )
    .await;
    Ok(Json(team))
}

async fn admin_update_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(team_id): Path<String>,
    Json(mut body): Json<UpsertTeamInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    body.id = Some(team_id.clone());
    let team = state.control.upsert_team(body)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("team:{team_id}"),
        format!(
            "更新团队/项目 {}",
            team.get("name").and_then(Value::as_str).unwrap_or(&team_id)
        ),
        "info",
    )
    .await;
    Ok(Json(team))
}

async fn admin_delete_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(team_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    state.control.delete_team(&team_id)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("team:{team_id}"),
        format!("删除团队/项目 {team_id}，相关 API Key 已解除绑定"),
        "warning",
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

async fn admin_quotas(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_admin_user(&state, &headers)?;
    let mut quotas = state.control.list_quotas()?;
    let limits = state.control.usage_quota_limits();
    let values = state.ledger.quota_usage_values(&limits).await?;
    for quota in &mut quotas {
        quota.used = values.get(&quota.id).copied().unwrap_or(0.0);
    }
    Ok(Json(json!(quotas)))
}

async fn admin_create_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<UpsertQuotaInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    populate_quota_user(state.auth.as_ref(), &mut body)?;
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
    )
    .await;
    Ok(Json(json!(quota)))
}

async fn admin_update_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(quota_id): Path<String>,
    Json(mut body): Json<UpsertQuotaInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    body.id = Some(quota_id);
    populate_quota_user(state.auth.as_ref(), &mut body)?;
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
    )
    .await;
    Ok(Json(json!(quota)))
}

async fn admin_delete_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(quota_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    state.control.delete_quota(&quota_id)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("quota:{quota_id}"),
        format!("删除配额 {quota_id}"),
        "warning",
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

fn populate_quota_user(auth: &AuthStore, body: &mut UpsertQuotaInput) -> Result<(), AppError> {
    let user = auth
        .user_by_id(&body.user_id)
        .ok_or_else(|| AppError::InvalidRequest("quota user not found".to_owned()))?;
    body.user_id = user.id;
    body.username = user.username;
    Ok(())
}

fn parse_ip_rule(value: &str) -> Result<IpRule, ()> {
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok(IpRule::Exact(ip));
    }
    let Some((base, prefix)) = value.split_once('/') else {
        return Err(());
    };
    let base = base.parse::<IpAddr>().map_err(|_| ())?;
    let prefix = prefix.parse::<u8>().map_err(|_| ())?;
    let max_prefix = match base {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    if prefix > max_prefix {
        return Err(());
    }
    Ok(IpRule::Cidr { base, prefix })
}

fn ip_rule_matches(rule: &IpRule, ip: IpAddr) -> bool {
    match (rule, ip) {
        (IpRule::Exact(exact), ip) => *exact == ip,
        (IpRule::Cidr { base, prefix }, IpAddr::V4(ip)) => match base {
            IpAddr::V4(base) if *prefix <= 32 => {
                cidr_matches(u32::from(*base).into(), u32::from(ip).into(), *prefix, 32)
            }
            _ => false,
        },
        (IpRule::Cidr { base, prefix }, IpAddr::V6(ip)) => match base {
            IpAddr::V6(base) if *prefix <= 128 => {
                cidr_matches(u128::from(*base), u128::from(ip), *prefix, 128)
            }
            _ => false,
        },
    }
}

fn cidr_matches(base: u128, ip: u128, prefix: u8, bits: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = u32::from(bits - prefix);
    (base >> shift) == (ip >> shift)
}

fn effective_config(state: &AppState) -> AppConfig {
    merged_config(state, false)
}

fn management_config(state: &AppState) -> AppConfig {
    merged_config(state, true)
}

fn merged_config(state: &AppState, include_disabled: bool) -> AppConfig {
    let mut config = state.config.snapshot();
    // The checked-in/runtime configuration is the organization-reviewed
    // catalog. Control-plane overrides may tune an approved entry, but must
    // never manufacture a new OpenAI-compatible endpoint or model.
    let organization_catalog = config.providers.clone();
    let controls = state.control.provider_control_snapshot();
    let snapshot = state.control.routing_config();
    let discovered_models = state.control.provider_discovered_models();

    for provider_id in &controls.deleted_providers {
        config.providers.remove(provider_id);
        config.provider_order.retain(|id| id != provider_id);
    }

    for (provider_id, record) in &controls.provider_overrides {
        if controls.deleted_providers.contains(provider_id) {
            continue;
        }
        let Some(approved) = organization_catalog.get(provider_id) else {
            continue;
        };
        let Ok(provider) = provider_record_to_config(record) else {
            continue;
        };
        if provider.protocol != approved.protocol
            || provider.base_url != approved.base_url
            || provider
                .models
                .iter()
                .any(|model| !approved.models.contains(model))
            || provider.passthrough_unknown_models != approved.passthrough_unknown_models
        {
            continue;
        }
        config.providers.insert(provider_id.clone(), provider);
        if !config.provider_order.contains(provider_id) {
            config.provider_order.push(provider_id.clone());
        }
    }

    if !include_disabled {
        for provider_id in &controls.disabled_providers {
            config.providers.remove(provider_id);
            config.provider_order.retain(|id| id != provider_id);
        }
    }

    for (provider_id, models) in discovered_models {
        let Some(provider) = config.providers.get_mut(&provider_id) else {
            continue;
        };
        let Some(approved) = organization_catalog.get(&provider_id) else {
            continue;
        };
        let mut seen = provider.models.iter().cloned().collect::<BTreeSet<_>>();
        for model in models {
            if approved.models.contains(&model) && seen.insert(model.clone()) {
                provider.models.push(model);
            }
        }
    }

    apply_provider_credentials(&mut config, &controls);
    apply_provider_model_overrides(&mut config, &controls.provider_model_overrides);
    for (provider_id, provider) in &mut config.providers {
        if let Some(approved) = organization_catalog.get(provider_id) {
            provider
                .models
                .retain(|model| approved.models.contains(model));
            if !provider.models.contains(&provider.default_model) {
                provider.default_model.clone_from(&approved.default_model);
            }
            // Runtime prefixes and passthrough are convenient discovery features,
            // but they are not an organization approval. The effective catalog is
            // deliberately exact so a newly published upstream model cannot become
            // routable without a reviewed configuration change.
            provider.model_prefixes.clear();
            provider.passthrough_unknown_models = false;
        }
    }
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

    normalize_provider_order(&mut config);
    if !config.providers.contains_key(&config.default_provider)
        && let Some(provider_id) = config.provider_order.first().cloned()
    {
        config.default_provider = provider_id;
    }

    config
}

fn apply_provider_credentials(
    config: &mut AppConfig,
    controls: &crate::control::ProviderControlSnapshot,
) {
    for (provider_id, credential_id) in &controls.active_provider_credentials {
        let Some(provider) = config.providers.get_mut(provider_id) else {
            continue;
        };
        let Some(record) = controls
            .provider_credentials
            .get(provider_id)
            .and_then(|credentials| credentials.get(credential_id))
        else {
            continue;
        };
        if record.status == "disabled" {
            continue;
        }
        provider.api_key_env = Some(record.api_key_env.clone());
        provider.api_key = env::var(&record.api_key_env).ok();
        if let Some(base_url) = &record.base_url {
            provider.base_url = base_url.clone();
        }
    }
}

fn provider_record_to_config(record: &ProviderOverrideRecord) -> Result<ProviderConfig, AppError> {
    let mut model_profiles = record.model_profiles.clone();
    for profile in model_profiles.values_mut() {
        profile.source = Some(ModelProfileSource::Control);
    }
    Ok(ProviderConfig {
        display_name: record.display_name.clone(),
        protocol: parse_provider_protocol(&record.protocol)?,
        base_url: record.base_url.clone(),
        api_key_env: record.api_key_env.clone(),
        api_key: record
            .api_key_env
            .as_deref()
            .and_then(|name| env::var(name).ok()),
        api_key_required: record.api_key_required,
        default_model: record.default_model.clone(),
        models: record.models.clone(),
        model_prefixes: record.model_prefixes.clone(),
        passthrough_unknown_models: record.passthrough_unknown_models,
        max_tokens_field: parse_max_tokens_field(&record.max_tokens_field)?,
        deduplicate_stream_text: record.deduplicate_stream_text,
        buffer_stream_text: record.buffer_stream_text,
        fidelity_mode: parse_fidelity_mode(&record.fidelity_mode)?,
        tool_use: record.tool_use,
        model_profile_defaults: record.model_profile_defaults.clone(),
        model_profiles,
        reasoning: record.reasoning.clone(),
        sampling: record.sampling.clone(),
        token_counting: record.token_counting.clone(),
        static_headers: record.static_headers.clone(),
        request_timeout_ms: record.request_timeout_ms,
        stream_idle_timeout_ms: record.stream_idle_timeout_ms,
        retry: record.retry.clone(),
        pricing: record.pricing,
        model_pricing: record.model_pricing.clone(),
        trust_upstream_cost: record.trust_upstream_cost,
    })
}

fn apply_provider_model_overrides(
    config: &mut AppConfig,
    model_overrides: &std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, ProviderModelOverrideRecord>,
    >,
) {
    for (provider_id, models) in model_overrides {
        let Some(provider) = config.providers.get_mut(provider_id) else {
            continue;
        };
        for record in models.values() {
            if record.status == "disabled" {
                provider.models.retain(|model| model != &record.model);
                if provider.default_model == record.model
                    && let Some(next_model) = provider.models.first().cloned()
                {
                    provider.default_model = next_model;
                }
                continue;
            }
            if !provider.models.contains(&record.model) {
                provider.models.push(record.model.clone());
            }
            let mut model_profile = ModelProfileOverride {
                display_name: record.display_name.clone(),
                family: record.family.clone(),
                context_window: record.context_window,
                source: Some(ModelProfileSource::Control),
                ..ModelProfileOverride::default()
            };
            model_profile.merge(&record.profile);
            model_profile.source = Some(ModelProfileSource::Control);
            provider
                .model_profiles
                .insert(record.model.clone(), model_profile);
        }
    }
}

fn normalize_provider_order(config: &mut AppConfig) {
    let mut seen = BTreeSet::new();
    config
        .provider_order
        .retain(|id| config.providers.contains_key(id) && seen.insert(id.clone()));
    let mut remaining = config.providers.keys().cloned().collect::<Vec<_>>();
    remaining.sort();
    for provider_id in remaining {
        if !seen.contains(&provider_id) {
            config.provider_order.push(provider_id.clone());
            seen.insert(provider_id);
        }
    }
}

fn provider_delete_dependencies(
    state: &AppState,
    config: &AppConfig,
    provider_id: &str,
) -> Vec<Value> {
    let mut dependencies = Vec::new();
    if config.default_provider == provider_id {
        dependencies.push(json!({
            "type": "defaultProvider",
            "id": provider_id,
            "field": "gateway.defaultProvider",
        }));
    }
    if config.provider_order.iter().any(|id| id == provider_id) {
        dependencies.push(json!({
            "type": "providerOrder",
            "id": provider_id,
            "field": "gateway.providerOrder",
        }));
    }
    for (alias, target) in &config.aliases {
        let direct = target == provider_id
            || target
                .split_once(':')
                .is_some_and(|(target_provider, _)| target_provider == provider_id);
        let resolved = config
            .resolve(alias)
            .ok()
            .is_some_and(|resolved| resolved.provider_id == provider_id);
        if direct || resolved {
            dependencies.push(json!({
                "type": "alias",
                "id": alias,
                "target": target,
                "field": "aliases",
            }));
        }
    }
    dependencies.extend(state.control.provider_policy_references(provider_id));
    dependencies
}

fn parse_provider_protocol(value: &str) -> Result<ProviderProtocol, AppError> {
    match value.trim() {
        "anthropic" => Ok(ProviderProtocol::Anthropic),
        "openai-compat" | "openai_compat" | "openaiCompatible" => {
            Ok(ProviderProtocol::OpenaiCompat)
        }
        _ => Err(AppError::InvalidRequest(
            "protocol must be anthropic or openai-compat".to_owned(),
        )),
    }
}

fn parse_max_tokens_field(value: &str) -> Result<MaxTokensField, AppError> {
    match value.trim() {
        "max_completion_tokens" | "max-completion-tokens" => {
            Ok(MaxTokensField::MaxCompletionTokens)
        }
        "max_tokens" | "max-tokens" => Ok(MaxTokensField::MaxTokens),
        "both" => Ok(MaxTokensField::Both),
        _ => Err(AppError::InvalidRequest(
            "maxTokensField must be max_completion_tokens, max_tokens, or both".to_owned(),
        )),
    }
}

fn parse_fidelity_mode(value: &str) -> Result<FidelityMode, AppError> {
    match value.trim() {
        "strict" => Ok(FidelityMode::Strict),
        "best_effort" | "best-effort" => Ok(FidelityMode::BestEffort),
        "stability" => Ok(FidelityMode::Stability),
        _ => Err(AppError::InvalidRequest(
            "fidelityMode must be strict, best_effort, or stability".to_owned(),
        )),
    }
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

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{
        body::{Body, to_bytes},
        extract::connect_info::ConnectInfo,
        http::{
            Method, Request, StatusCode,
            header::{CONTENT_TYPE, COOKIE, HOST, HeaderValue, ORIGIN, SET_COOKIE},
        },
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        auth::{AuthStore, CreateUserInput, UpdateUserInput},
        config::{
            FidelityMode, MaxTokensField, ProviderConfig, ReasoningConfig, ReasoningMode,
            RouteCandidateConfig, RouteGroupConfig, RoutingProfile, SmartRoutingConfig,
            SmartRoutingMode, TokenCountingConfig, TokenCountingMode,
        },
        control::{BindApiKeyScopeInput, CreateApiKeyInput, UpdateApiKeyInput},
        metrics::Metrics,
    };

    const CLIENT_TOKEN: &str = "client-token";

    #[tokio::test]
    async fn composed_router_matches_the_complete_route_inventory() {
        let app = router(test_state("http://127.0.0.1:9".to_owned(), 1024));

        for contract in route_contract::all() {
            let path = concrete_contract_path(contract.path);
            let unowned_method = contract_response(&app, Method::TRACE, &path).await;
            assert_eq!(
                unowned_method,
                StatusCode::METHOD_NOT_ALLOWED,
                "{} does not resolve to its declared {} domain",
                contract.path,
                contract.domain,
            );

            for method in contract.methods {
                let method = Method::from_bytes(method.as_bytes()).unwrap();
                let status = contract_response(&app, method.clone(), &path).await;
                assert_ne!(
                    status,
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{} {} is missing from the composed router",
                    method,
                    contract.path,
                );
            }
        }
    }

    async fn contract_response(app: &Router, method: Method, path: &str) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    fn concrete_contract_path(path: &str) -> String {
        let mut concrete = String::with_capacity(path.len());
        let mut remaining = path;
        while let Some(start) = remaining.find('{') {
            concrete.push_str(&remaining[..start]);
            let parameter = &remaining[start..];
            let end = parameter.find('}').expect("route parameter closes");
            concrete.push_str("route-contract");
            remaining = &parameter[end + 1..];
        }
        concrete.push_str(remaining);
        concrete
    }

    #[tokio::test]
    async fn public_auth_methods_report_disabled_oidc_without_cache() {
        let app = router(test_state("http://127.0.0.1:9".to_owned(), 1024));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/auth/methods")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["passwordEnabled"], true);
        assert_eq!(body["oidc"]["enabled"], false);
        assert_eq!(body["oidc"]["startUrl"], "/admin/auth/oidc/start");
    }

    #[tokio::test]
    async fn disabled_oidc_start_redirects_safely_and_clears_flow_cookie() {
        let app = router(test_state("http://127.0.0.1:9".to_owned(), 1024));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/auth/oidc/start?returnTo=%2Fdashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()["location"], "/login?oidc_error=disabled");
        assert_eq!(response.headers()["cache-control"], "no-store");
        let cookie = response.headers()[SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("modelport_oidc_flow="));
        assert!(cookie.contains("Max-Age=0"));
    }

    #[test]
    fn quota_owner_is_loaded_from_the_auth_store() {
        let auth = AuthStore::for_tests();
        let user = auth
            .create_user(CreateUserInput {
                username: "quota-user".to_owned(),
                email: "quota@example.com".to_owned(),
                password: "strong-quota-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let mut input = UpsertQuotaInput {
            id: None,
            user_id: user.id.clone(),
            username: "forged-name".to_owned(),
            quota_type: "tokens".to_owned(),
            limit: 1_000.0,
            period: "monthly".to_owned(),
        };

        populate_quota_user(&auth, &mut input).unwrap();
        assert_eq!(input.user_id, user.id);
        assert_eq!(input.username, "quota-user");

        input.user_id = "usr_missing".to_owned();
        assert!(populate_quota_user(&auth, &mut input).is_err());
    }

    #[test]
    fn console_origin_allows_loopback_dev_ports() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("127.0.0.1:17878"));
        headers.insert(ORIGIN, HeaderValue::from_static("http://127.0.0.1:5173"));

        assert!(validate_admin_request_origin(&headers).is_ok());
    }

    #[test]
    fn console_origin_allows_localhost_to_loopback_dev_ports() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("127.0.0.1:17878"));
        headers.insert(ORIGIN, HeaderValue::from_static("http://localhost:5173"));

        assert!(validate_admin_request_origin(&headers).is_ok());
    }

    #[test]
    fn console_origin_rejects_non_loopback_cross_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("modelport.internal"));
        headers.insert(ORIGIN, HeaderValue::from_static("https://evil.example"));

        assert!(validate_admin_request_origin(&headers).is_err());
    }

    #[test]
    fn public_model_rows_hide_unconfigured_providers() {
        let active = ProviderConfig {
            display_name: "Mimo".to_owned(),
            protocol: ProviderProtocol::OpenaiCompat,
            base_url: "http://mimo.local/v1".to_owned(),
            api_key_env: None,
            api_key: Some("upstream-key".to_owned()),
            api_key_required: true,
            default_model: "mimo-v2.5-pro".to_owned(),
            models: vec!["mimo-v2.5-pro".to_owned()],
            model_prefixes: vec!["mimo-".to_owned()],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            deduplicate_stream_text: true,
            buffer_stream_text: true,
            fidelity_mode: FidelityMode::Stability,
            tool_use: ToolUseConfig::default_for_provider(
                "mimo",
                ProviderProtocol::OpenaiCompat,
                true,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: Default::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };
        let inactive = ProviderConfig {
            display_name: "DeepSeek".to_owned(),
            protocol: ProviderProtocol::Anthropic,
            base_url: "http://deepseek.local/v1".to_owned(),
            api_key_env: Some("DEEPSEEK_ANTHROPIC_AUTH_TOKEN".to_owned()),
            api_key: None,
            api_key_required: true,
            default_model: "deepseek-v4-flash".to_owned(),
            models: vec!["deepseek-v4-flash".to_owned()],
            model_prefixes: vec!["deepseek-".to_owned()],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::BestEffort,
            tool_use: ToolUseConfig::default_for_provider(
                "deepseek",
                ProviderProtocol::Anthropic,
                false,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: Default::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };
        let config = AppConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            max_request_body_bytes: 1024 * 1024,
            max_concurrent_requests: 16,
            auth_token: Some(CLIENT_TOKEN.to_owned()),
            default_provider: "mimo".to_owned(),
            provider_order: vec!["deepseek".to_owned(), "mimo".to_owned()],
            providers: HashMap::from([
                ("deepseek".to_owned(), inactive),
                ("mimo".to_owned(), active),
            ]),
            aliases: HashMap::from([
                ("fast-chat".to_owned(), "mimo:mimo-v2.5-pro".to_owned()),
                (
                    "deepseek-route".to_owned(),
                    "deepseek:deepseek-v4-flash".to_owned(),
                ),
            ]),
            smart_routing: Default::default(),
            runtime_adapters: Default::default(),
        };

        let rows = client_api::public_model_rows(&config);
        let ids = rows
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(!ids.contains(&"mimo-v2.5-pro"));
        assert!(ids.contains(&"mimo:mimo-v2.5-pro"));
        assert!(ids.contains(&"fast-chat"));
        assert!(!ids.contains(&"deepseek-v4-flash"));
        assert!(!ids.contains(&"deepseek:deepseek-v4-flash"));
        assert!(!ids.contains(&"deepseek-route"));
    }

    #[test]
    fn public_model_rows_use_model_owner_for_third_party_channels() {
        let provider = ProviderConfig {
            display_name: "小米 MiMo".to_owned(),
            protocol: ProviderProtocol::OpenaiCompat,
            base_url: "https://w.ciykj.cn/v1".to_owned(),
            api_key_env: None,
            api_key: Some("upstream-key".to_owned()),
            api_key_required: true,
            default_model: "mimo-v2.5-pro".to_owned(),
            models: vec!["gpt-5.2".to_owned(), "mimo-v2.5-pro".to_owned()],
            model_prefixes: vec!["mimo-".to_owned()],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            deduplicate_stream_text: true,
            buffer_stream_text: true,
            fidelity_mode: FidelityMode::Stability,
            tool_use: ToolUseConfig::default_for_provider(
                "mimo",
                ProviderProtocol::OpenaiCompat,
                true,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: Default::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };
        let config = AppConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            max_request_body_bytes: 1024 * 1024,
            max_concurrent_requests: 16,
            auth_token: Some(CLIENT_TOKEN.to_owned()),
            default_provider: "mimo".to_owned(),
            provider_order: vec!["mimo".to_owned()],
            providers: HashMap::from([("mimo".to_owned(), provider)]),
            aliases: HashMap::new(),
            smart_routing: Default::default(),
            runtime_adapters: Default::default(),
        };

        let rows = client_api::public_model_rows(&config);
        let display_name = |id: &str| {
            rows.iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .and_then(|row| row.get("display_name").and_then(Value::as_str))
                .unwrap()
        };

        assert_eq!(display_name("mimo:gpt-5.2"), "第三方 · OpenAI");
        assert_eq!(display_name("mimo:mimo-v2.5-pro"), "第三方 · 小米 MiMo");
    }

    #[test]
    fn public_model_rows_recognize_deployment_specific_local_providers() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let mut config = state.config.snapshot();
        let mut provider = config.providers.remove("mimo").unwrap();
        provider.display_name = "Qwen3.5-9B Q5_K_M（本地）".to_owned();
        provider.base_url = "http://qwen-runtime:8080/v1".to_owned();
        provider.default_model = "qwen3.5-9b-q5km".to_owned();
        provider.models = vec![provider.default_model.clone()];
        provider.model_prefixes.clear();
        config.default_provider = "local_qwen".to_owned();
        config.provider_order = vec!["local_qwen".to_owned()];
        config.providers.insert("local_qwen".to_owned(), provider);

        let rows = client_api::public_model_rows(&config);

        assert_eq!(rows[0]["id"], "local_qwen:qwen3.5-9b-q5km");
        assert_eq!(rows[0]["owned_by"], "local_qwen");
        assert_eq!(rows[0]["display_name"], "本地 · Qwen");
    }

    #[test]
    fn provider_rows_expose_tool_use_capabilities() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);

        let rows = provider_rows(&state);
        let provider = rows
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some("mimo"))
            .expect("mimo provider row");

        assert_eq!(provider["toolUse"]["supported"], true);
        assert_eq!(provider["toolUse"]["toolChoice"], true);
        assert_eq!(provider["toolUse"]["streamingArguments"], "cumulative");
    }

    #[test]
    fn provider_row_by_id_keeps_provider_not_found_boundary() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);

        let provider = provider_row_by_id(&state, "mimo").expect("mimo provider row");
        let missing = provider_row_by_id(&state, "missing-provider").unwrap_err();

        assert_eq!(provider["id"], "mimo");
        assert!(matches!(
            missing,
            AppError::ProviderNotFound(provider) if provider == "missing-provider"
        ));
    }

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
        let state = test_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();
        let app = router(state);

        let response = post_message_response(app, CLIENT_TOKEN, message_body(false)).await;
        let status = response.status();
        let decision_id = response
            .headers()
            .get(&ROUTING_DECISION_ID)
            .and_then(|value| value.to_str().ok())
            .unwrap();
        assert!(decision_id.starts_with("rtd_"));
        assert_eq!(
            response
                .headers()
                .get(&ROUTING_MODE)
                .and_then(|value| value.to_str().ok()),
            Some("static")
        );
        let body = response_body(response).await;

        assert_eq!(status, StatusCode::OK);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["content"][0]["text"], "hello from upstream");
        assert_eq!(body["usage"]["input_tokens"], 3);
        assert_eq!(body["usage"]["output_tokens"], 4);
        assert_eq!(
            ledger
                .incomplete_requests(&crate::domain::TenantScope::legacy_local())
                .await,
            0
        );
    }

    #[tokio::test]
    async fn smart_alias_shadow_route_returns_and_persists_decision_evidence() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl_smart_route",
                "choices": [{
                    "message": {"role": "assistant", "content": "smart route"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 2, "completion_tokens": 2}
            }"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        let mut config = state.config.snapshot();
        config.smart_routing = SmartRoutingConfig {
            mode: SmartRoutingMode::Shadow,
            default_profile: RoutingProfile::Balanced,
            policy_version: "integration-v1".to_owned(),
            activation_percent: 0,
            groups: HashMap::from([(
                "general".to_owned(),
                RouteGroupConfig {
                    aliases: vec!["modelport-auto".to_owned()],
                    default_profile: None,
                    candidates: vec![RouteCandidateConfig {
                        provider: "mimo".to_owned(),
                        model: "mimo-v2.5-pro".to_owned(),
                        quality: 0.8,
                        latency_hint_ms: 800,
                        enabled: true,
                    }],
                },
            )]),
        };
        state.config = Arc::new(RuntimeConfig::new(config));
        let ledger = state.ledger.clone();
        let mut request = message_body(false);
        request["model"] = json!("modelport-auto");

        let response = post_message_response(router(state), CLIENT_TOKEN, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(&ROUTING_MODE)
                .and_then(|value| value.to_str().ok()),
            Some("shadow")
        );
        let decision_id = response
            .headers()
            .get(&ROUTING_DECISION_ID)
            .and_then(|value| value.to_str().ok())
            .unwrap()
            .to_owned();
        let _ = response_body(response).await;
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(
            rows[0]["routingDecision"]["decisionId"],
            decision_id.as_str()
        );
        assert_eq!(rows[0]["routingDecision"]["groupId"], "general");
        assert_eq!(
            rows[0]["routingDecision"]["policyVersion"],
            "integration-v1"
        );
        assert_eq!(rows[0]["routingDecision"]["mode"], "shadow");
    }

    #[tokio::test]
    async fn repairs_strict_tool_arguments_once_and_accounts_both_attempts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
        let handler_calls = calls.clone();
        let handler_bodies = bodies.clone();
        let upstream = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(body): Json<Value>| {
                let call = handler_calls.fetch_add(1, Ordering::SeqCst);
                handler_bodies.lock().unwrap().push(body);
                async move {
                    let (arguments, prompt_tokens, completion_tokens) = if call == 0 {
                        (r#"{"city":42,"private":"do-not-copy"}"#, 10, 2)
                    } else {
                        (r#"{"city":"Shanghai"}"#, 12, 3)
                    };
                    Json(json!({
                        "id": format!("chatcmpl-repair-{call}"),
                        "choices": [{
                            "message": {
                                "role": "assistant",
                                "content": null,
                                "tool_calls": [{
                                    "id": format!("call_{call}"),
                                    "type": "function",
                                    "function": {"name": "weather", "arguments": arguments}
                                }]
                            },
                            "finish_reason": "tool_calls"
                        }],
                        "usage": {
                            "prompt_tokens": prompt_tokens,
                            "completion_tokens": completion_tokens
                        }
                    }))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });

        let mut state = test_state(format!("http://{addr}/v1"), 1024 * 1024);
        let mut config = state.config.snapshot();
        let provider = config.providers.get_mut("mimo").unwrap();
        provider.tool_use.response_validation = ToolResponseValidation::Strict;
        provider.tool_use.repair_invalid_arguments = true;
        state.config = Arc::new(RuntimeConfig::new(config));
        let ledger = state.ledger.clone();
        let mut body = message_body(false);
        body["tools"] = json!([{
            "name": "weather",
            "description": "Look up weather",
            "input_schema": {
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"],
                "additionalProperties": false
            }
        }]);

        let (status, response) = post_message(router(state), body).await;

        assert_eq!(status, StatusCode::OK, "{response}");
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["content"][0]["input"]["city"], "Shanghai");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        {
            let bodies = bodies.lock().unwrap();
            let repair_prompt = bodies[1]["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .and_then(|message| message["content"].as_str())
                .unwrap();
            assert!(repair_prompt.contains("JSON Schema validation"));
            assert!(!repair_prompt.contains("do-not-copy"));
        }
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows[0]["toolRepairAttempted"], true);
        assert_eq!(rows[0]["toolRepairRecovered"], true);
        assert_eq!(rows[0]["retryCount"], 1);
        assert_eq!(rows[0]["fallbackFromProvider"], Value::Null);
        assert_eq!(rows[0]["inputTokens"], 22);
        assert_eq!(rows[0]["outputTokens"], 5);
        assert_eq!(rows[0]["billingMode"], "upstream-returned+tool-repair");
    }

    #[tokio::test]
    async fn fallback_accounts_every_sent_provider_attempt() {
        let primary = spawn_openai_upstream(
            StatusCode::SERVICE_UNAVAILABLE,
            r#"{"error":{"message":"temporarily unavailable"}}"#,
            "application/json",
        )
        .await;
        let fallback = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id":"chatcmpl-fallback",
                "choices":[{
                    "message":{"role":"assistant","content":"ok"},
                    "finish_reason":"stop"
                }],
                "usage":{"prompt_tokens":3,"completion_tokens":1}
            }"#,
            "application/json",
        )
        .await;
        let mut state = test_state(primary, 1024 * 1024);
        let mut config = state.config.snapshot();
        let mut fallback_provider = config.providers["mimo"].clone();
        fallback_provider.display_name = "Fallback".to_owned();
        fallback_provider.base_url = fallback;
        config.provider_order = vec!["mimo".to_owned(), "fallback".to_owned()];
        config
            .providers
            .insert("fallback".to_owned(), fallback_provider);
        state.config = Arc::new(RuntimeConfig::new(config));
        let ledger = state.ledger.clone();

        let (status, response) = post_message(router(state), message_body(false)).await;

        assert_eq!(status, StatusCode::OK, "{response}");
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows[0]["provider"], "fallback");
        assert_eq!(rows[0]["retryCount"], 1);
        assert_eq!(rows[0]["fallbackFromProvider"], "mimo");
        assert_eq!(rows[0]["billingMode"], "mixed-attempts");
        assert!(rows[0]["inputTokens"].as_u64().unwrap() > 3);
        assert!(rows[0]["outputTokens"].as_u64().unwrap() > 1);
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
        assert!(body.contains("provider_authentication_failed"));
        assert!(!body.contains("INVALID_API_KEY"));
        assert!(!body.contains("Invalid API key"));
    }

    #[tokio::test]
    async fn rejects_stream_upstream_status_during_handshake() {
        let upstream = spawn_openai_upstream(
            StatusCode::UNAUTHORIZED,
            r#"{"code":"INVALID_API_KEY","message":"Invalid API key"}"#,
            "application/json",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();
        let app = router(state);

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("provider_authentication_failed"));
        assert!(!body.contains("INVALID_API_KEY"));
        assert!(!body.contains("Invalid API key"));
        let logs = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(logs[0]["status"], "error");
        assert_eq!(logs[0]["statusCode"], 401);
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
        let state = test_state(upstream, 1024 * 1024);
        let control = state.control.clone();
        let metrics = state.metrics.clone();
        let ledger = state.ledger.clone();
        let app = router(state);

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains(r#""text":"hel""#));
        assert!(body.contains(r#""text":"lo""#));
        assert!(!body.contains("event: error"));

        let logs = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["status"], "success");
        assert_eq!(logs[0]["statusCode"], 200);
        assert_eq!(logs[0]["terminalReason"], "completed");
        assert!(
            logs[0]["attemptId"]
                .as_str()
                .is_some_and(|attempt_id| attempt_id.starts_with("att_"))
        );
        let provider_health = control.provider_health_rows();
        assert_eq!(provider_health["mimo"]["successesTotal"], 1);
        assert_eq!(provider_health["mimo"]["failuresTotal"], 0);
        let metrics = metrics.snapshot();
        let message = metrics
            .messages
            .iter()
            .find(|message| message.provider == "mimo" && message.stream)
            .expect("stream message metrics");
        assert_eq!(message.successes_total, 1);
        assert_eq!(message.failures_total, 0);
        tokio::task::yield_now().await;
        assert_eq!(
            ledger
                .incomplete_requests(&crate::domain::TenantScope::legacy_local())
                .await,
            0
        );
    }

    #[tokio::test]
    async fn stream_protocol_failure_is_reconciled_after_successful_handshake() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"data: {"choices":[{"delta":{"content":"partial"},"finish_reason":null}]}

"#,
            "text/event-stream",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let control = state.control.clone();
        let metrics = state.metrics.clone();
        let ledger = state.ledger.clone();

        let (status, body) = post_message(router(state), message_body(true)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("event: error"));
        assert!(body.contains("upstream provider returned an incompatible response"));
        assert!(!body.contains("ended without [DONE] or finish_reason"));
        let logs = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["status"], "error");
        assert_eq!(logs[0]["statusCode"], 502);
        assert_eq!(logs[0]["terminalReason"], "upstream_error");
        let provider_health = control.provider_health_rows();
        assert_eq!(provider_health["mimo"]["successesTotal"], 0);
        assert_eq!(provider_health["mimo"]["failuresTotal"], 1);
        let metrics = metrics.snapshot();
        let message = metrics
            .messages
            .iter()
            .find(|message| message.provider == "mimo" && message.stream)
            .expect("stream message metrics");
        assert_eq!(message.successes_total, 0);
        assert_eq!(message.failures_total, 1);
    }

    #[tokio::test]
    async fn dropping_stream_response_records_downstream_cancellation_once() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl_buffered_cancel",
                "choices": [{
                    "message": {"role": "assistant", "content": "completed upstream"},
                    "finish_reason": "stop"
                }]
            }"#,
            "application/json",
        )
        .await;
        let state = test_state_with_flags(upstream, 1024 * 1024, true, true);
        let control = state.control.clone();
        let metrics = state.metrics.clone();
        let ledger = state.ledger.clone();
        let permits = state.stream_permits.clone();

        let response = post_message_response(router(state), CLIENT_TOKEN, message_body(true)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .get(&ROUTING_DECISION_ID)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("rtd_"))
        );
        assert_eq!(
            response
                .headers()
                .get(&ROUTING_MODE)
                .and_then(|value| value.to_str().ok()),
            Some("static")
        );
        assert!(ledger.usage_rows().await.unwrap().is_empty());
        assert_eq!(permits.available_permits(), 15);
        drop(response);

        assert_eq!(permits.available_permits(), 16);
        let logs = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["status"], "error");
        assert_eq!(logs[0]["statusCode"], 499);
        assert_eq!(
            logs[0]["terminalReason"],
            "downstream_cancelled_after_upstream_complete"
        );
        let provider_health = control.provider_health_rows();
        assert_eq!(provider_health["mimo"]["successesTotal"], 1);
        assert_eq!(provider_health["mimo"]["failuresTotal"], 0);
        let metrics = metrics.snapshot();
        let message = metrics
            .messages
            .iter()
            .find(|message| message.provider == "mimo" && message.stream)
            .expect("stream message metrics");
        assert_eq!(message.requests_total, 1);
        assert_eq!(message.failures_total, 1);
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
        let mut state = test_state_with_flags(upstream, 1024 * 1024, false, false);
        let mut config = state.config.snapshot();
        config
            .providers
            .get_mut("mimo")
            .expect("mimo provider")
            .tool_use
            .streaming_arguments = crate::config::ToolArgumentMode::Cumulative;
        state.config = Arc::new(RuntimeConfig::new(config));
        let app = router(state);

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
    async fn streams_legacy_openai_function_call_as_tool_use() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"data: {"choices":[{"delta":{"function_call":{"name":"read_file","arguments":""}},"finish_reason":null}]}
data: {"choices":[{"delta":{"function_call":{"arguments":"{\"path\":\"Cargo.toml\"}"}},"finish_reason":null}]}
data: {"choices":[{"delta":{},"finish_reason":"function_call"}]}
data: [DONE]

"#,
            "text/event-stream",
        )
        .await;
        let app = router(test_state(upstream, 1024 * 1024));

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""type":"tool_use""#));
        assert!(body.contains(r#""name":"read_file""#));
        assert!(body.contains(r#""partial_json":"{\"path\":\"Cargo.toml\"}""#));
        assert!(body.contains(r#""stop_reason":"tool_use""#));
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
    async fn buffered_stream_rejects_upstream_status_before_sse_response() {
        let upstream = spawn_openai_upstream(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"invalid credential"}}"#,
            "application/json",
        )
        .await;
        let app = router(test_state_with_flags(upstream, 1024 * 1024, true, true));

        let (status, body) = post_message(app, message_body(true)).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("provider_authentication_failed"));
        assert!(!body.contains("invalid credential"));
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
    async fn rejects_empty_message_list_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["messages"] = json!([]);

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("messages must not be empty"));
    }

    #[tokio::test]
    async fn rejects_invalid_message_shape_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["messages"] = json!([
            {
                "role": "system",
                "content": "system content belongs in the top-level system field"
            }
        ]);

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("messages[0].role must be user or assistant"));
    }

    #[tokio::test]
    async fn rejects_invalid_message_content_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["messages"] = json!([
            {
                "role": "user",
                "content": null
            }
        ]);

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("messages[0].content must be a string or array"));
    }

    #[tokio::test]
    async fn rejects_invalid_tools_shape_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["tools"] = json!({
            "name": "not-an-array"
        });

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("tools must be an array"));
    }

    #[tokio::test]
    async fn rejects_invalid_tool_name_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["tools"] = json!([
            {
                "name": "read file",
                "description": "Read a file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }
        ]);

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("tools[0].name may only contain"));
    }

    #[tokio::test]
    async fn rejects_invalid_tool_choice_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["tool_choice"] = json!({
            "type": "tool"
        });

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("tool_choice.name is required"));
    }

    #[tokio::test]
    async fn rejects_invalid_tool_result_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["messages"] = json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "content": "missing tool id"
                    }
                ]
            }
        ]);

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("messages[0].content[0].tool_use_id is required"));
    }

    #[tokio::test]
    async fn rejects_tools_when_provider_capability_disables_tool_use() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let mut state = test_state(upstream, 1024 * 1024);
        let mut config = state.config.snapshot();
        config
            .providers
            .get_mut("mimo")
            .expect("mimo provider")
            .tool_use
            .supported = false;
        config
            .providers
            .get_mut("mimo")
            .expect("mimo provider")
            .tool_use
            .tool_choice = false;
        config
            .providers
            .get_mut("mimo")
            .expect("mimo provider")
            .tool_use
            .parallel_tool_calls = false;
        state.config = Arc::new(RuntimeConfig::new(config));
        let app = router(state);
        let mut body = message_body(false);
        body["tools"] = json!([
            {
                "name": "read_file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }
        ]);

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("provider `mimo` does not support tool use"));
    }

    #[tokio::test]
    async fn rejects_zero_max_tokens_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body["max_tokens"] = json!(0);

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("max_tokens must be greater than 0"));
    }

    #[tokio::test]
    async fn rejects_missing_max_tokens_before_routing() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));
        let mut body = message_body(false);
        body.as_object_mut().unwrap().remove("max_tokens");

        let (status, body) = post_message(app, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("max_tokens is required"));
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
    async fn health_is_minimal_without_auth_and_readyz_requires_auth() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let app = router(test_state(upstream, 1024 * 1024));

        let health_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health_response.status(), StatusCode::OK);
        assert_eq!(
            health_response
                .headers()
                .get("x-content-type-options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        let health_body = to_bytes(health_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let health_body: Value = serde_json::from_slice(&health_body).unwrap();
        assert_eq!(health_body["status"], json!("ok"));
        assert!(health_body.get("providerHealth").is_none());

        let readyz_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(readyz_response.status(), StatusCode::UNAUTHORIZED);

        let detailed_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/readyz")
                    .header("x-api-key", CLIENT_TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detailed_response.status(), StatusCode::OK);
        let detailed_body = to_bytes(detailed_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let detailed_body: Value = serde_json::from_slice(&detailed_body).unwrap();
        assert!(detailed_body.get("providerHealth").is_some());
    }

    #[tokio::test]
    async fn draining_disables_readiness_and_rejects_new_inference() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"unused","choices":[]}"#,
            "application/json",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        state.draining.store(true, Ordering::Release);
        let app = router(state);

        let readyz = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/readyz")
                    .header("x-api-key", CLIENT_TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(readyz.status(), StatusCode::SERVICE_UNAVAILABLE);

        let (message_status, _) = post_message(app.clone(), message_body(false)).await;
        assert_eq!(message_status, StatusCode::SERVICE_UNAVAILABLE);

        let metrics = app
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
        assert_eq!(metrics.status(), StatusCode::OK);
        let body = to_bytes(metrics.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("modelport_gateway_ready 0"));
        assert!(body.contains("modelport_gateway_draining 1"));
    }

    #[tokio::test]
    async fn control_api_key_mode_rejects_legacy_token_but_accepts_api_key_records() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        state.security = Arc::new(GatewaySecurityPolicy::require_control_api_keys_for_tests());
        let owner = state
            .auth
            .create_user(CreateUserInput {
                username: "test-user".to_owned(),
                email: "test-user@example.com".to_owned(),
                password: "strong-test-user-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let created = state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: owner.id,
                username: Some(owner.username),
                name: "Claude Code".to_owned(),
                principal_type: None,
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: None,
                allowed_providers: None,
                expires_at: None,
            })
            .unwrap();
        let app = router(state);

        let (legacy_status, _) = post_message(app.clone(), message_body(false)).await;
        assert_eq!(legacy_status, StatusCode::FORBIDDEN);

        let (api_key_status, body) =
            post_message_with_key(app, &created.key, message_body(false)).await;
        assert_eq!(api_key_status, StatusCode::OK);
        assert!(body.contains("ok"));
    }

    #[tokio::test]
    async fn models_enforces_api_key_ip_policy_with_trusted_proxy_client_ip() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let owner = state
            .auth
            .create_user(CreateUserInput {
                username: "models-ip-owner".to_owned(),
                email: "models-ip-owner@example.com".to_owned(),
                password: "strong-models-ip-owner-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let created = create_test_api_key(&state, &owner, "models-ip-key");
        state
            .control
            .update_api_key(
                &created.public.id,
                UpdateApiKeyInput {
                    name: None,
                    group: None,
                    team_id: None,
                    allowed_models: None,
                    allowed_providers: None,
                    expires_at: None,
                    status: None,
                    ip_restricted: Some(true),
                    allowed_ips: Some(vec!["203.0.113.10".to_owned()]),
                    spend_limit_usd: None,
                    rate_limited: None,
                    five_hour_limit_usd: None,
                    daily_limit_usd: None,
                    weekly_limit_usd: None,
                    monthly_limit_usd: None,
                },
            )
            .unwrap();
        let app = router(state);

        let allowed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .extension(ConnectInfo(
                        "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                    ))
                    .header("x-api-key", created.key.clone())
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);

        let denied = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .extension(ConnectInfo(
                        "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                    ))
                    .header("x-api-key", created.key)
                    .header("x-forwarded-for", "198.51.100.20")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert!(response_body(denied).await.contains("not allowed"));
    }

    #[tokio::test]
    async fn api_key_tenant_scope_is_enforced_for_both_client_protocols() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        state.security = Arc::new(GatewaySecurityPolicy::require_control_api_keys_for_tests());
        let owner = state
            .auth
            .create_user(CreateUserInput {
                username: "scoped-user".to_owned(),
                email: "scoped-user@example.com".to_owned(),
                password: "strong-scoped-user-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let created = state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: owner.id,
                username: Some(owner.username),
                name: "scoped client".to_owned(),
                principal_type: None,
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: None,
                allowed_providers: None,
                expires_at: None,
            })
            .unwrap();
        state
            .control
            .bind_api_key_scope(
                &created.public.id,
                BindApiKeyScopeInput {
                    organization_id: "org_acme".to_owned(),
                    project_id: "prj_agent".to_owned(),
                    environment_id: "env_prod".to_owned(),
                },
            )
            .unwrap();
        let app = router(state);
        let bearer = format!("Bearer {}", created.key);
        let requests = [
            (
                "/v1/messages",
                "x-api-key",
                created.key.as_str(),
                message_body(false),
            ),
            (
                "/v1/chat/completions",
                "authorization",
                bearer.as_str(),
                chat_body(false),
            ),
        ];

        for (uri, auth_header, auth_value, body) in requests {
            let forged = post_scoped_json_response(
                app.clone(),
                uri,
                auth_header,
                auth_value,
                ("org_acme", "prj_other", "env_prod"),
                body.clone(),
            )
            .await;
            assert_eq!(forged.status(), StatusCode::FORBIDDEN, "{uri}");
            assert!(response_body(forged).await.contains("not bound"));

            let matching = post_scoped_json_response(
                app.clone(),
                uri,
                auth_header,
                auth_value,
                ("org_acme", "prj_agent", "env_prod"),
                body,
            )
            .await;
            assert_eq!(matching.status(), StatusCode::OK, "{uri}");
        }
    }

    #[tokio::test]
    async fn control_api_key_rejects_an_inactive_owner() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        state.security = Arc::new(GatewaySecurityPolicy::require_control_api_keys_for_tests());
        let owner = state
            .auth
            .create_user(CreateUserInput {
                username: "disabled-owner".to_owned(),
                email: "disabled-owner@example.com".to_owned(),
                password: "strong-disabled-owner-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("disabled".to_owned()),
            })
            .unwrap();
        let created = state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: owner.id,
                username: Some(owner.username),
                name: "disabled owner key".to_owned(),
                principal_type: None,
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: None,
                allowed_providers: None,
                expires_at: None,
            })
            .unwrap();

        let (status, _) =
            post_message_with_key(router(state), &created.key, message_body(false)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn message_rate_limiter_rejects_excess_api_key_requests() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        state.rate_limiter = Arc::new(RateLimiter::for_tests(1, 0, 0, 0));
        let app = router(state);

        let (first_status, _) = post_message(app.clone(), message_body(false)).await;
        let second_response = post_message_response(app, CLIENT_TOKEN, message_body(false)).await;
        let retry_after = second_response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let second_status = second_response.status();
        let body = response_body(second_response).await;

        assert_eq!(first_status, StatusCode::OK);
        assert_eq!(second_status, StatusCode::TOO_MANY_REQUESTS);
        assert!(body.contains("rate limit"));
        assert_eq!(retry_after.as_deref(), Some("60"));
    }

    #[tokio::test]
    async fn message_rate_limiter_can_limit_by_provider() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        state.rate_limiter = Arc::new(RateLimiter::for_tests(0, 0, 1, 0));
        let app = router(state);

        let (first_status, _) = post_message(app.clone(), message_body(false)).await;
        let (second_status, body) = post_message(app, message_body(false)).await;

        assert_eq!(first_status, StatusCode::OK);
        assert_eq!(second_status, StatusCode::TOO_MANY_REQUESTS);
        assert!(body.contains("provider request rate limit exceeded"));
    }

    #[tokio::test]
    async fn self_service_delete_and_recreate_does_not_reset_api_key_rpm() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        state.rate_limiter = Arc::new(RateLimiter::for_tests(1, 0, 0, 0));
        let owner = state
            .auth
            .create_user(CreateUserInput {
                username: "rpm-lineage".to_owned(),
                email: "rpm-lineage@example.com".to_owned(),
                password: "strong-rpm-lineage-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let owner_id = owner.id;
        let owner_name = owner.username;
        let control = state.control.clone();
        let create = || {
            control
                .create_api_key_with_active_limit(
                    CreateApiKeyInput {
                        user_id: owner_id.clone(),
                        username: Some(owner_name.clone()),
                        name: "self-service RPM".to_owned(),
                        principal_type: Some("user".to_owned()),
                        purpose: None,
                        group: None,
                        team_id: None,
                        allowed_models: None,
                        allowed_providers: None,
                        expires_at: None,
                    },
                    5,
                )
                .unwrap()
        };
        let key_a = create();
        let app = router(state);

        let (first_status, first_body) =
            post_message_with_key(app.clone(), &key_a.key, message_body(false)).await;
        assert_eq!(first_status, StatusCode::OK, "{first_body}");
        control.delete_api_key(&key_a.public.id).unwrap();
        let key_b = create();
        let (second_status, second_body) =
            post_message_with_key(app, &key_b.key, message_body(false)).await;
        assert_eq!(second_status, StatusCode::TOO_MANY_REQUESTS);
        assert!(second_body.contains("rate limit"));
    }

    #[tokio::test]
    async fn self_service_delete_and_recreate_does_not_reset_all_time_spend() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id":"ok",
                "choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":900,"completion_tokens":100}
            }"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        let mut config = state.config.snapshot();
        config
            .providers
            .get_mut("mimo")
            .unwrap()
            .model_pricing
            .insert(
                "mimo-v2.5-pro".to_owned(),
                crate::pricing::ModelPricingCard {
                    rates: crate::pricing::ModelPricing {
                        input_per_million: 1.0,
                        output_per_million: 1.0,
                        cache_read_per_million: 0.0,
                        cache_write_per_million: 0.0,
                    },
                    version: "test-contract-v1".to_owned(),
                    effective_at: "2026-08-01T00:00:00Z".to_owned(),
                    currency: "USD".to_owned(),
                    source: crate::pricing::PricingSource::ProviderContract,
                    service_tier: crate::pricing::PricingServiceTier::Standard,
                    region: None,
                    evidence: "contract://test/mimo-v1".to_owned(),
                },
            );
        state.config = Arc::new(RuntimeConfig::new(config));
        let owner = state
            .auth
            .create_user(CreateUserInput {
                username: "spend-lineage".to_owned(),
                email: "spend-lineage@example.com".to_owned(),
                password: "strong-spend-lineage-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let owner_id = owner.id;
        let owner_name = owner.username;
        let control = state.control.clone();
        let ledger = state.ledger.clone();
        let create = || {
            control
                .create_api_key_with_active_limit(
                    CreateApiKeyInput {
                        user_id: owner_id.clone(),
                        username: Some(owner_name.clone()),
                        name: "self-service spend".to_owned(),
                        principal_type: Some("user".to_owned()),
                        purpose: None,
                        group: None,
                        team_id: None,
                        allowed_models: None,
                        allowed_providers: None,
                        expires_at: None,
                    },
                    5,
                )
                .unwrap()
        };
        let key_a = create();
        let limits = UpdateApiKeyInput {
            name: None,
            group: None,
            team_id: None,
            allowed_models: None,
            allowed_providers: None,
            expires_at: None,
            status: None,
            ip_restricted: None,
            allowed_ips: None,
            spend_limit_usd: Some(0.0005),
            rate_limited: None,
            five_hour_limit_usd: None,
            daily_limit_usd: None,
            weekly_limit_usd: None,
            monthly_limit_usd: None,
        };
        control.update_api_key(&key_a.public.id, limits).unwrap();
        let app = router(state);

        let (first_status, first_body) =
            post_message_with_key(app.clone(), &key_a.key, message_body(false)).await;
        assert_eq!(first_status, StatusCode::OK, "{first_body}");
        wait_for_usage_rows(&ledger, 1).await;
        let usage_rows = ledger.usage_rows().await.unwrap();
        assert_eq!(usage_rows[0]["actualCost"], 0.001);
        assert_eq!(usage_rows[0]["billableCost"], 0.001);
        assert_eq!(usage_rows[0]["reconciliationStatus"], "billable");
        assert_eq!(
            usage_rows[0]["pricingEvidence"]["method"],
            "exact_rate_card"
        );
        control.delete_api_key(&key_a.public.id).unwrap();
        let key_b = create();
        assert_eq!(key_b.public.spend_limit_usd, 0.0005);

        let (second_status, second_body) =
            post_message_with_key(app, &key_b.key, message_body(false)).await;
        assert_eq!(second_status, StatusCode::TOO_MANY_REQUESTS);
        assert!(second_body.contains("quota"));
    }

    #[test]
    fn rejected_rate_limit_check_does_not_consume_other_windows() {
        let limiter = RateLimiter::for_tests(2, 0, 1, 0);
        let identity = ControlStore::legacy_identity();

        assert!(
            limiter
                .check(RateLimitScope {
                    identity: &identity,
                    client_ip: None,
                    provider_id: Some("provider-a"),
                    model: None,
                })
                .is_ok()
        );
        assert!(
            limiter
                .check(RateLimitScope {
                    identity: &identity,
                    client_ip: None,
                    provider_id: Some("provider-a"),
                    model: None,
                })
                .is_err()
        );
        assert!(
            limiter
                .check(RateLimitScope {
                    identity: &identity,
                    client_ip: None,
                    provider_id: Some("provider-b"),
                    model: None,
                })
                .is_ok()
        );
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
            r#"modelport_message_requests_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 1"#
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
    async fn enterprise_ledger_admin_api_requires_admin_and_returns_request_facts() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl_enterprise",
                "choices": [{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}
            }"#,
            "application/json",
        )
        .await;
        let app = router(test_state_with_admin(upstream, 1024 * 1024));

        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/enterprise/overview")
                    .header("x-api-key", CLIENT_TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let (message_status, _) = post_chat_completion(app.clone(), chat_body(false)).await;
        assert_eq!(message_status, StatusCode::OK);

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
        let session_cookie = login_response
            .headers()
            .get(SET_COOKIE)
            .expect("login should set a session cookie")
            .clone();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/enterprise/requests?page=1&pageSize=20&protocol=openai-chat-completions")
                    .header(COOKIE, session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["total"], 1);
        assert_eq!(
            body["requests"][0]["clientProtocol"],
            "openai-chat-completions"
        );
        assert_eq!(body["requests"][0]["attemptCount"], 1);
        assert!(body["requests"][0].get("idempotencyKeyHash").is_none());
    }

    #[tokio::test]
    async fn viewer_cannot_read_admin_dashboard_or_create_users() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let state = test_state(upstream, 1024 * 1024);
        state
            .auth
            .create_user(CreateUserInput {
                username: "viewer".to_owned(),
                email: "viewer@modelport.local".to_owned(),
                password: "strong-password-123".to_owned(),
                role: Some("viewer".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let app = router(state);

        let login_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/auth/login")
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(
                        json!({
                            "username": "viewer",
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
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/dashboard")
                    .header(COOKIE, session_cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(dashboard_response.status(), StatusCode::FORBIDDEN);

        let create_user_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/users")
                    .header(COOKIE, session_cookie)
                    .header("x-modelport-csrf", "1")
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(
                        json!({
                            "username": "blocked",
                            "email": "blocked@modelport.local",
                            "password": "strong-password-123",
                            "role": "user",
                            "status": "active",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_user_response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn organization_read_surfaces_are_admin_only_and_users_are_self_scoped() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let user = state
            .auth
            .create_user(CreateUserInput {
                username: "boundary-user".to_owned(),
                email: "boundary-user@example.com".to_owned(),
                password: "strong-boundary-user-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let viewer = state
            .auth
            .create_user(CreateUserInput {
                username: "boundary-viewer".to_owned(),
                email: "boundary-viewer@example.com".to_owned(),
                password: "strong-boundary-viewer-password-123".to_owned(),
                role: Some("viewer".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        state
            .auth
            .create_user(CreateUserInput {
                username: "boundary-admin".to_owned(),
                email: "boundary-admin@example.com".to_owned(),
                password: "strong-boundary-admin-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let app = router(state);
        let user_cookie = login_cookie(
            app.clone(),
            "boundary-user",
            "strong-boundary-user-password-123",
        )
        .await;
        let viewer_cookie = login_cookie(
            app.clone(),
            "boundary-viewer",
            "strong-boundary-viewer-password-123",
        )
        .await;
        let admin_cookie = login_cookie(
            app.clone(),
            "boundary-admin",
            "strong-boundary-admin-password-123",
        )
        .await;

        for (cookie, own_user_id) in [
            (user_cookie, user.id.as_str()),
            (viewer_cookie, viewer.id.as_str()),
        ] {
            for uri in ["/admin/audit", "/admin/latency", "/admin/quotas"] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(uri)
                            .header(COOKIE, cookie.clone())
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "GET {uri}");
            }

            let own_users = get_console_json(app.clone(), "/admin/users", cookie).await;
            let rows = own_users.as_array().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["id"], own_user_id);
        }

        for uri in ["/admin/audit", "/admin/latency", "/admin/quotas"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header(COOKIE, admin_cookie.clone())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "GET {uri}");
        }
        let all_users = get_console_json(app, "/admin/users", admin_cookie).await;
        assert_eq!(all_users.as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn active_session_is_revoked_immediately_after_role_downgrade() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let target = state
            .auth
            .create_user(CreateUserInput {
                username: "downgraded-admin".to_owned(),
                email: "downgraded-admin@example.com".to_owned(),
                password: "strong-downgraded-admin-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let other_admin = state
            .auth
            .create_user(CreateUserInput {
                username: "remaining-admin".to_owned(),
                email: "remaining-admin@example.com".to_owned(),
                password: "strong-remaining-admin-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let auth = state.auth.clone();
        let app = router(state);
        let cookie = login_cookie(
            app.clone(),
            "downgraded-admin",
            "strong-downgraded-admin-password-123",
        )
        .await;

        auth.update_user(
            &target.id,
            &other_admin.id,
            UpdateUserInput {
                email: None,
                password: None,
                role: Some("viewer".to_owned()),
                status: None,
            },
        )
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/auth/me")
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn request_logs_fail_closed_for_users_and_viewers() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let alice = state
            .auth
            .create_user(CreateUserInput {
                username: "alice".to_owned(),
                email: "alice@modelport.local".to_owned(),
                password: "strong-alice-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let bob = state
            .auth
            .create_user(CreateUserInput {
                username: "bob".to_owned(),
                email: "bob@modelport.local".to_owned(),
                password: "strong-bob-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let viewer = state
            .auth
            .create_user(CreateUserInput {
                username: "auditor".to_owned(),
                email: "auditor@modelport.local".to_owned(),
                password: "strong-viewer-password-123".to_owned(),
                role: Some("viewer".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        state
            .auth
            .create_user(CreateUserInput {
                username: "admin".to_owned(),
                email: "admin@modelport.local".to_owned(),
                password: "strong-admin-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let alice_key = create_test_api_key(&state, &alice, "alice-key");
        let bob_key = create_test_api_key(&state, &bob, "bob-key");
        let viewer_key = create_test_api_key(&state, &viewer, "viewer-key");
        let ledger = state.ledger.clone();
        let app = router(state);

        for key in [&alice_key.key, &bob_key.key, &viewer_key.key] {
            let (status, body) = post_message_with_key(app.clone(), key, message_body(false)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        let rows = wait_for_usage_rows(&ledger, 3).await;
        let bob_log_id = rows
            .iter()
            .find(|row| row["userId"] == bob.id)
            .and_then(|row| row["id"].as_str())
            .unwrap()
            .to_owned();

        let alice_cookie = login_cookie(app.clone(), "alice", "strong-alice-password-123").await;
        let alice_list = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/admin/logs?userId={}", bob.id))
                    .header(COOKIE, alice_cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(alice_list.status(), StatusCode::OK);
        let alice_body: Value =
            serde_json::from_slice(&to_bytes(alice_list.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(alice_body["total"], 1);
        assert!(
            alice_body["logs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["userId"] == alice.id)
        );

        let cross_user_detail = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/admin/logs/{bob_log_id}"))
                    .header(COOKIE, alice_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cross_user_detail.status(), StatusCode::NOT_FOUND);

        let viewer_cookie =
            login_cookie(app.clone(), "auditor", "strong-viewer-password-123").await;
        let viewer_list = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/admin/logs?userId={}", alice.id))
                    .header(COOKIE, viewer_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let viewer_body: Value =
            serde_json::from_slice(&to_bytes(viewer_list.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(viewer_body["total"], 1);
        assert!(
            viewer_body["logs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["userId"] == viewer.id)
        );

        let admin_cookie = login_cookie(app.clone(), "admin", "strong-admin-password-123").await;
        let admin_list = app
            .oneshot(
                Request::builder()
                    .uri("/admin/logs")
                    .header(COOKIE, admin_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let admin_body: Value =
            serde_json::from_slice(&to_bytes(admin_list.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(admin_body["total"], 3);
    }

    #[tokio::test]
    async fn users_can_only_create_and_rotate_bounded_own_api_keys() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let alice = state
            .auth
            .create_user(CreateUserInput {
                username: "self-service".to_owned(),
                email: "self-service@modelport.local".to_owned(),
                password: "strong-self-service-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let bob = state
            .auth
            .create_user(CreateUserInput {
                username: "other-owner".to_owned(),
                email: "other-owner@modelport.local".to_owned(),
                password: "strong-other-owner-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let bob_key = create_test_api_key(&state, &bob, "other-key");
        let app = router(state);
        let cookie = login_cookie(
            app.clone(),
            "self-service",
            "strong-self-service-password-123",
        )
        .await;

        let service_account = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/api-keys")
                    .header(COOKIE, cookie.clone())
                    .header("x-modelport-csrf", "1")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "userId": bob.id,
                            "name": "forged-service-account",
                            "principalType": "service_account",
                            "purpose": "attempted privilege escalation",
                            "allowedModels": ["mimo-v2.5-pro"],
                            "allowedProviders": ["mimo"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(service_account.status(), StatusCode::FORBIDDEN);

        let created_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/users/{}/api-keys", bob.id))
                    .header(COOKIE, cookie.clone())
                    .header("x-modelport-csrf", "1")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "userId": bob.id,
                            "name": "my-claude-code",
                            "allowedModels": ["mimo-v2.5-pro"],
                            "allowedProviders": ["mimo"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created_response.status(), StatusCode::OK);
        let created: Value = serde_json::from_slice(
            &to_bytes(created_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(created["userId"], alice.id);
        assert_eq!(created["principalType"], "user");
        assert!(created["expiresAt"].as_str().is_some());
        let old_key_id = created["id"].as_str().unwrap().to_owned();
        let old_secret = created["key"].as_str().unwrap().to_owned();

        let cross_owner_rotation = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/api-keys/{}/rotate", bob_key.public.id))
                    .header(COOKIE, cookie.clone())
                    .header("x-modelport-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cross_owner_rotation.status(), StatusCode::FORBIDDEN);

        let cancelled_rotation = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/api-keys/{old_key_id}/rotate"))
                    .header(COOKIE, cookie.clone())
                    .header("x-modelport-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cancelled_rotation.status(), StatusCode::OK);
        let cancelled: Value = serde_json::from_slice(
            &to_bytes(cancelled_rotation.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let cancelled_id = cancelled["id"].as_str().unwrap();
        for _ in 0..2 {
            let cancel_response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("DELETE")
                        .uri(format!(
                            "/admin/api-keys/{old_key_id}/rotate/{cancelled_id}"
                        ))
                        .header(COOKIE, cookie.clone())
                        .header("x-modelport-csrf", "1")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(cancel_response.status(), StatusCode::OK);
        }

        let rotate_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/api-keys/{old_key_id}/rotate"))
                    .header(COOKIE, cookie.clone())
                    .header("x-modelport-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rotate_response.status(), StatusCode::OK);
        let rotated: Value = serde_json::from_slice(
            &to_bytes(rotate_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_ne!(rotated["id"], old_key_id);
        assert_eq!(rotated["userId"], alice.id);
        assert_eq!(rotated["allowedModels"], created["allowedModels"]);
        assert_eq!(rotated["allowedProviders"], created["allowedProviders"]);
        assert_eq!(rotated["status"], "pending_rotation");
        let rotated_id = rotated["id"].as_str().unwrap().to_owned();
        let rotated_secret = rotated["key"].as_str().unwrap().to_owned();

        let (old_status, _) =
            post_message_with_key(app.clone(), &old_secret, message_body(false)).await;
        assert_eq!(old_status, StatusCode::OK);
        let (pending_status, _) =
            post_message_with_key(app.clone(), &rotated_secret, message_body(false)).await;
        assert_eq!(pending_status, StatusCode::UNAUTHORIZED);

        let confirm_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/api-keys/{old_key_id}/rotate/{rotated_id}"))
                    .header(COOKIE, cookie.clone())
                    .header("x-modelport-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(confirm_response.status(), StatusCode::OK);

        let replayed_confirm = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/api-keys/{old_key_id}/rotate/{rotated_id}"))
                    .header(COOKIE, cookie.clone())
                    .header("x-modelport-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replayed_confirm.status(), StatusCode::OK);

        let (old_status, _) =
            post_message_with_key(app.clone(), &old_secret, message_body(false)).await;
        assert_eq!(old_status, StatusCode::UNAUTHORIZED);
        let (new_status, body) =
            post_message_with_key(app.clone(), &rotated_secret, message_body(false)).await;
        assert_eq!(new_status, StatusCode::OK, "{body}");

        let revoke_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/api-keys/{rotated_id}/disable"))
                    .header(COOKIE, cookie)
                    .header("x-modelport-csrf", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke_response.status(), StatusCode::OK);
        let (revoked_status, _) =
            post_message_with_key(app, &rotated_secret, message_body(false)).await;
        assert_eq!(revoked_status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn retention_preview_is_admin_only_csrf_protected_and_audited() {
        let state = test_state("http://127.0.0.1:9".to_owned(), 1024 * 1024);
        state
            .auth
            .create_user(CreateUserInput {
                username: "retention-user".to_owned(),
                email: "retention-user@modelport.local".to_owned(),
                password: "strong-retention-user-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        state
            .auth
            .create_user(CreateUserInput {
                username: "retention-admin".to_owned(),
                email: "retention-admin@modelport.local".to_owned(),
                password: "strong-retention-admin-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let ledger = state.ledger.clone();
        let app = router(state);
        let user_cookie = login_cookie(
            app.clone(),
            "retention-user",
            "strong-retention-user-password-123",
        )
        .await;
        let admin_cookie = login_cookie(
            app.clone(),
            "retention-admin",
            "strong-retention-admin-password-123",
        )
        .await;

        let user_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/retention/run")
                    .header(COOKIE, user_cookie)
                    .header("x-modelport-csrf", "1")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"dryRun":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(user_response.status(), StatusCode::FORBIDDEN);

        let no_csrf = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/retention/run")
                    .header(COOKIE, admin_cookie.clone())
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"dryRun":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

        let preview = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/retention/run")
                    .header(COOKIE, admin_cookie)
                    .header("x-modelport-csrf", "1")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"dryRun":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(preview.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["dryRun"], true);
        assert_eq!(body["applied"], false);
        assert_eq!(body["policy"]["contentPersistence"], false);
        assert_eq!(body["immutableBudgetEventsRetained"], true);
        assert!(body["requestDetailCutoffMs"].as_u64().is_some());
        assert!(body["previewToken"].as_str().is_some());
        assert!(body["previewExpiresAtMs"].as_u64().is_some());

        let (audit, _) = ledger.audit_events(20).await.unwrap();
        assert!(audit.iter().any(|event| event["type"] == "data_retention"));
    }

    #[tokio::test]
    async fn operations_agent_configuration_is_opt_in_csrf_protected_and_local_first() {
        let mut state = test_state_with_admin("http://127.0.0.1:9".to_owned(), 1024 * 1024);
        let mut config = state.config.snapshot();
        let mut local = config.providers.remove("mimo").unwrap();
        local.display_name = "Local vLLM".to_owned();
        local.default_model = "qwen3".to_owned();
        local.models = vec!["qwen3".to_owned()];
        config.default_provider = "local_vllm".to_owned();
        config.provider_order = vec!["local_vllm".to_owned()];
        config.providers.insert("local_vllm".to_owned(), local);
        state.config = Arc::new(RuntimeConfig::new(config));
        let app = router(state);
        let cookie = login_cookie(app.clone(), "admin", "strong-password-123").await;

        let initial = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/admin/ops/configuration")
                    .header(COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(initial.status(), StatusCode::OK);
        let initial: Value =
            serde_json::from_slice(&to_bytes(initial.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(initial["enabled"], false);
        assert_eq!(initial["analysisEnabled"], false);
        assert_eq!(initial["preferLocal"], true);
        assert_eq!(initial["recommendedModel"], "local_vllm:qwen3");
        assert_eq!(initial["candidates"][0]["local"], true);

        let body = json!({
            "enabled": true,
            "analysisEnabled": true,
            "selectedModel": "local_vllm:qwen3",
            "preferLocal": true,
        })
        .to_string();
        let without_csrf = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/admin/ops/configuration")
                    .header(COOKIE, cookie.clone())
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(without_csrf.status(), StatusCode::FORBIDDEN);

        let updated = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/admin/ops/configuration")
                    .header(COOKIE, cookie)
                    .header("x-modelport-csrf", "1")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::OK);
        let updated: Value =
            serde_json::from_slice(&to_bytes(updated.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(updated["enabled"], true);
        assert_eq!(updated["analysisEnabled"], true);
        assert_eq!(updated["selectedModelLocal"], true);
        assert_eq!(updated["modelReady"], true);
    }

    #[tokio::test]
    async fn retention_apply_requires_and_consumes_preview_with_fixed_cutoffs() {
        let state = test_state("http://127.0.0.1:9".to_owned(), 1024 * 1024);
        state
            .auth
            .create_user(CreateUserInput {
                username: "retention-apply-admin".to_owned(),
                email: "retention-apply-admin@modelport.local".to_owned(),
                password: "strong-retention-apply-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let app = router(state);
        let cookie = login_cookie(
            app.clone(),
            "retention-apply-admin",
            "strong-retention-apply-password-123",
        )
        .await;

        let (missing_status, _) =
            post_retention(app.clone(), cookie.clone(), json!({ "dryRun": false })).await;
        assert_eq!(missing_status, StatusCode::BAD_REQUEST);

        let (preview_status, preview) =
            post_retention(app.clone(), cookie.clone(), json!({ "dryRun": true })).await;
        assert_eq!(preview_status, StatusCode::OK);
        let token = preview["previewToken"].as_str().unwrap().to_owned();
        assert!(preview["previewExpiresAtMs"].as_u64().unwrap() > now_millis());

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let (apply_status, applied) = post_retention(
            app.clone(),
            cookie.clone(),
            json!({ "dryRun": false, "previewToken": token }),
        )
        .await;
        assert_eq!(apply_status, StatusCode::OK);
        assert_eq!(applied["applied"], true);
        assert_eq!(applied["evaluatedAtMs"], preview["evaluatedAtMs"]);
        assert_eq!(
            applied["requestDetailCutoffMs"],
            preview["requestDetailCutoffMs"]
        );
        assert_eq!(applied["userUsageCutoffMs"], preview["userUsageCutoffMs"]);
        assert_eq!(applied["auditCutoffMs"], preview["auditCutoffMs"]);
        assert_eq!(applied["policy"], preview["policy"]);
        assert!(applied["previewToken"].is_null());
        assert!(applied["previewExpiresAtMs"].is_null());

        let (replay_status, _) = post_retention(
            app,
            cookie,
            json!({ "dryRun": false, "previewToken": token }),
        )
        .await;
        assert_eq!(replay_status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn retention_preview_is_actor_bound_without_cross_actor_consumption() {
        let state = test_state("http://127.0.0.1:9".to_owned(), 1024 * 1024);
        for username in ["retention-owner", "retention-other"] {
            state
                .auth
                .create_user(CreateUserInput {
                    username: username.to_owned(),
                    email: format!("{username}@modelport.local"),
                    password: format!("strong-{username}-password-123"),
                    role: Some("admin".to_owned()),
                    status: Some("active".to_owned()),
                })
                .unwrap();
        }
        let app = router(state);
        let owner_cookie = login_cookie(
            app.clone(),
            "retention-owner",
            "strong-retention-owner-password-123",
        )
        .await;
        let other_cookie = login_cookie(
            app.clone(),
            "retention-other",
            "strong-retention-other-password-123",
        )
        .await;
        let (_, preview) =
            post_retention(app.clone(), owner_cookie.clone(), json!({ "dryRun": true })).await;
        let token = preview["previewToken"].as_str().unwrap().to_owned();

        let (wrong_actor_status, _) = post_retention(
            app.clone(),
            other_cookie,
            json!({ "dryRun": false, "previewToken": token }),
        )
        .await;
        assert_eq!(wrong_actor_status, StatusCode::CONFLICT);

        let (owner_status, owner_apply) = post_retention(
            app,
            owner_cookie,
            json!({ "dryRun": false, "previewToken": token }),
        )
        .await;
        assert_eq!(owner_status, StatusCode::OK);
        assert_eq!(owner_apply["applied"], true);
    }

    #[tokio::test]
    async fn retention_apply_rejects_expired_preview() {
        let mut state = test_state("http://127.0.0.1:9".to_owned(), 1024 * 1024);
        state.retention_previews = Arc::new(RetentionPreviewStore::with_ttl_ms(0));
        state
            .auth
            .create_user(CreateUserInput {
                username: "retention-expired-admin".to_owned(),
                email: "retention-expired-admin@modelport.local".to_owned(),
                password: "strong-retention-expired-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let app = router(state);
        let cookie = login_cookie(
            app.clone(),
            "retention-expired-admin",
            "strong-retention-expired-password-123",
        )
        .await;
        let (_, preview) =
            post_retention(app.clone(), cookie.clone(), json!({ "dryRun": true })).await;
        let token = preview["previewToken"].as_str().unwrap();

        let (status, _) = post_retention(
            app,
            cookie,
            json!({ "dryRun": false, "previewToken": token }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn retention_legal_hold_consumes_preview_without_applying() {
        let state = test_state("http://127.0.0.1:9".to_owned(), 1024 * 1024);
        let admin = state
            .auth
            .create_user(CreateUserInput {
                username: "retention-held-admin".to_owned(),
                email: "retention-held-admin@modelport.local".to_owned(),
                password: "strong-retention-held-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let evaluated_at_ms = 1_900_000_000_000;
        let policy = RetentionPolicy {
            request_detail_days: 30,
            user_usage_days: 90,
            audit_days: 395,
            legal_hold: true,
            content_persistence: false,
        };
        let (token, _) =
            state
                .retention_previews
                .issue(&admin.id, policy, evaluated_at_ms, now_millis());
        let app = router(state);
        let cookie = login_cookie(
            app.clone(),
            "retention-held-admin",
            "strong-retention-held-password-123",
        )
        .await;

        let (status, held) = post_retention(
            app.clone(),
            cookie.clone(),
            json!({ "dryRun": false, "previewToken": token }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(held["applied"], false);
        assert_eq!(held["skippedReason"], "legal_hold");
        assert_eq!(held["evaluatedAtMs"], evaluated_at_ms);

        let (replay_status, _) = post_retention(
            app,
            cookie,
            json!({ "dryRun": false, "previewToken": token }),
        )
        .await;
        assert_eq!(replay_status, StatusCode::CONFLICT);
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
                    .header("x-modelport-csrf", "1")
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
            r#"modelport_message_requests_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 1"#
        ));
    }

    #[tokio::test]
    async fn non_admin_catalog_is_key_and_team_scoped_and_redacted() {
        let mut state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let mut config = state.config.snapshot();
        let mut other = config.providers["mimo"].clone();
        other.display_name = "Other".to_owned();
        other.default_model = "other-model".to_owned();
        other.models = vec!["other-model".to_owned()];
        other.pricing = Some(crate::pricing::ModelPricing {
            input_per_million: 1.0,
            output_per_million: 2.0,
            cache_read_per_million: 0.5,
            cache_write_per_million: 1.5,
        });
        config.provider_order.push("other".to_owned());
        config.providers.insert("other".to_owned(), other);
        config
            .aliases
            .insert("fast".to_owned(), "mimo:mimo-v2.5-pro".to_owned());
        config
            .aliases
            .insert("Fast".to_owned(), "mimo:mimo-v2.5-pro".to_owned());
        config
            .aliases
            .insert("other".to_owned(), "other:other-model".to_owned());
        state.config = Arc::new(RuntimeConfig::new(config));

        let no_key_user = state
            .auth
            .create_user(CreateUserInput {
                username: "catalog-no-key".to_owned(),
                email: "catalog-no-key@example.com".to_owned(),
                password: "strong-catalog-no-key-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let scoped_user = state
            .auth
            .create_user(CreateUserInput {
                username: "catalog-scoped".to_owned(),
                email: "catalog-scoped@example.com".to_owned(),
                password: "strong-catalog-scoped-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let expired_user = state
            .auth
            .create_user(CreateUserInput {
                username: "catalog-expired".to_owned(),
                email: "catalog-expired@example.com".to_owned(),
                password: "strong-catalog-expired-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let admin = state
            .auth
            .create_user(CreateUserInput {
                username: "catalog-admin".to_owned(),
                email: "catalog-admin@example.com".to_owned(),
                password: "strong-catalog-admin-password-123".to_owned(),
                role: Some("admin".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        assert_eq!(no_key_user.role, "user");
        assert_eq!(admin.role, "admin");

        let team = state
            .control
            .upsert_team(UpsertTeamInput {
                id: Some("team_catalog".to_owned()),
                name: "Catalog Team".to_owned(),
                slug: Some("catalog-team".to_owned()),
                description: None,
                status: Some("active".to_owned()),
                daily_limit_usd: None,
                monthly_limit_usd: None,
                allowed_models: Some(vec!["fast".to_owned()]),
                allowed_providers: Some(vec!["mimo".to_owned()]),
            })
            .unwrap();
        let team_id = team["id"].as_str().unwrap().to_owned();
        let scoped_key = state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: scoped_user.id.clone(),
                username: Some(scoped_user.username.clone()),
                name: "catalog scoped key".to_owned(),
                principal_type: Some("user".to_owned()),
                purpose: None,
                group: None,
                team_id: Some(team_id),
                // The key permits both providers, but the team must remain an
                // independent upper bound. `fast` is deliberately lowercase.
                allowed_models: Some(vec!["fast".to_owned(), "other-model".to_owned()]),
                allowed_providers: Some(vec!["mimo".to_owned(), "other".to_owned()]),
                expires_at: None,
            })
            .unwrap();
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .saturating_add(10);
        state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: expired_user.id.clone(),
                username: Some(expired_user.username.clone()),
                name: "catalog expiring key".to_owned(),
                principal_type: Some("user".to_owned()),
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: None,
                allowed_providers: None,
                expires_at: Some(expires_at.to_string()),
            })
            .unwrap();

        let app = router(state);
        let no_key_cookie = login_cookie(
            app.clone(),
            "catalog-no-key",
            "strong-catalog-no-key-password-123",
        )
        .await;
        let scoped_cookie = login_cookie(
            app.clone(),
            "catalog-scoped",
            "strong-catalog-scoped-password-123",
        )
        .await;
        let expired_cookie = login_cookie(
            app.clone(),
            "catalog-expired",
            "strong-catalog-expired-password-123",
        )
        .await;
        let admin_cookie = login_cookie(
            app.clone(),
            "catalog-admin",
            "strong-catalog-admin-password-123",
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        for uri in ["/admin/providers", "/admin/aliases"] {
            let no_key = get_console_json(app.clone(), uri, no_key_cookie.clone()).await;
            assert_eq!(no_key, json!([]), "no-key catalog leaked at {uri}");
            let expired = get_console_json(app.clone(), uri, expired_cookie.clone()).await;
            assert_eq!(expired, json!([]), "expired-key catalog leaked at {uri}");
        }

        let providers =
            get_console_json(app.clone(), "/admin/providers", scoped_cookie.clone()).await;
        let providers = providers.as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["id"], "mimo");
        assert_eq!(providers[0]["models"], json!(["mimo-v2.5-pro"]));
        assert_eq!(providers[0]["aliases"], json!(["fast"]));
        assert_catalog_is_redacted(&providers[0]);

        let aliases = get_console_json(app.clone(), "/admin/aliases", scoped_cookie).await;
        assert_eq!(aliases.as_array().unwrap().len(), 1);
        assert_eq!(aliases[0]["alias"], "fast");
        assert_eq!(aliases[0]["resolvedProvider"], "mimo");
        assert_eq!(aliases[0]["resolvedModel"], "mimo-v2.5-pro");
        assert_catalog_is_redacted(&aliases);
        assert!(
            aliases
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["alias"] != "Fast")
        );
        assert!(
            aliases
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["alias"] != "other")
        );

        let models_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .extension(ConnectInfo(
                        "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                    ))
                    .header("x-api-key", scoped_key.key)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(models_response.status(), StatusCode::OK);
        let models: Value = serde_json::from_slice(
            &to_bytes(models_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let model_ids = models["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(model_ids, vec!["fast"]);

        // Administrators retain the complete management contract.
        let admin_providers = get_console_json(app, "/admin/providers", admin_cookie).await;
        assert!(admin_providers[0].get("baseUrl").is_some());
        assert!(admin_providers[0].get("apiKeyEnv").is_some());
        assert!(admin_providers[0].get("credentials").is_some());
        assert!(admin_providers[0].get("pricing").is_some());
    }

    #[tokio::test]
    async fn smart_alias_catalog_uses_real_candidates_instead_of_the_default_provider() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl_smart_catalog",
                "choices": [{
                    "message": {"role": "assistant", "content": "candidate b"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 2, "completion_tokens": 2}
            }"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        let mut config = state.config.snapshot();
        let mut candidate_b = config.providers["mimo"].clone();
        candidate_b.display_name = "Candidate B".to_owned();
        candidate_b.default_model = "candidate-b-model".to_owned();
        candidate_b.models = vec!["candidate-b-model".to_owned()];
        candidate_b.model_prefixes.clear();
        config.provider_order.push("candidate_b".to_owned());
        config
            .providers
            .insert("candidate_b".to_owned(), candidate_b);
        config.smart_routing = SmartRoutingConfig {
            mode: SmartRoutingMode::Active,
            default_profile: RoutingProfile::Balanced,
            policy_version: "catalog-candidate-v1".to_owned(),
            activation_percent: 100,
            groups: HashMap::from([(
                "smart-catalog".to_owned(),
                RouteGroupConfig {
                    aliases: vec!["smart-model".to_owned()],
                    default_profile: None,
                    candidates: vec![RouteCandidateConfig {
                        provider: "candidate_b".to_owned(),
                        model: "candidate-b-model".to_owned(),
                        quality: 1.0,
                        latency_hint_ms: 1,
                        enabled: true,
                    }],
                },
            )]),
        };
        state.config = Arc::new(RuntimeConfig::new(config));

        let candidate_user = state
            .auth
            .create_user(CreateUserInput {
                username: "smart-candidate-user".to_owned(),
                email: "smart-candidate-user@example.com".to_owned(),
                password: "strong-smart-candidate-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let default_user = state
            .auth
            .create_user(CreateUserInput {
                username: "smart-default-user".to_owned(),
                email: "smart-default-user@example.com".to_owned(),
                password: "strong-smart-default-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let candidate_key = state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: candidate_user.id,
                username: Some(candidate_user.username),
                name: "candidate B only".to_owned(),
                principal_type: Some("user".to_owned()),
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: Some(vec!["smart-model".to_owned()]),
                allowed_providers: Some(vec!["candidate_b".to_owned()]),
                expires_at: None,
            })
            .unwrap();
        let default_key = state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: default_user.id,
                username: Some(default_user.username),
                name: "default A only".to_owned(),
                principal_type: Some("user".to_owned()),
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: Some(vec!["smart-model".to_owned()]),
                allowed_providers: Some(vec!["mimo".to_owned()]),
                expires_at: None,
            })
            .unwrap();

        let app = router(state);
        let candidate_cookie = login_cookie(
            app.clone(),
            "smart-candidate-user",
            "strong-smart-candidate-password-123",
        )
        .await;
        let default_cookie = login_cookie(
            app.clone(),
            "smart-default-user",
            "strong-smart-default-password-123",
        )
        .await;

        let candidate_models = get_client_models(app.clone(), &candidate_key.key).await;
        assert!(client_model_ids(&candidate_models).contains(&"smart-model"));
        let candidate_aliases =
            get_console_json(app.clone(), "/admin/aliases", candidate_cookie).await;
        assert_eq!(candidate_aliases[0]["alias"], "smart-model");
        assert_eq!(candidate_aliases[0]["resolvedProvider"], "modelport-router");

        // The logical alias happens to resolve through the default provider in
        // static routing. That must not make it visible when every real smart
        // candidate is outside this key's provider policy.
        let default_models = get_client_models(app.clone(), &default_key.key).await;
        assert!(!client_model_ids(&default_models).contains(&"smart-model"));
        assert_eq!(
            get_console_json(app.clone(), "/admin/aliases", default_cookie).await,
            json!([])
        );

        let mut request = message_body(false);
        request["model"] = json!("smart-model");
        let (status, body) = post_message_with_key(app, &candidate_key.key, request).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["content"][0]["text"],
            "candidate b"
        );
    }

    #[tokio::test]
    async fn catalogs_respect_credential_pool_route_readiness() {
        const PRIMARY_ENV: &str = "MODELPORT_CATALOG_POOL_PRIMARY_UNSET";
        const SECONDARY_ENV: &str = "MODELPORT_CATALOG_POOL_SECONDARY_READY";
        unsafe {
            env::remove_var(PRIMARY_ENV);
            env::set_var(SECONDARY_ENV, "secondary-key");
        }

        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let user = state
            .auth
            .create_user(CreateUserInput {
                username: "credential-catalog-user".to_owned(),
                email: "credential-catalog-user@example.com".to_owned(),
                password: "strong-credential-catalog-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let key = create_test_api_key(&state, &user, "credential catalog");
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "catalog-primary".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Unavailable primary".to_owned(),
                api_key_env: PRIMARY_ENV.to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "catalog-secondary".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Ready secondary".to_owned(),
                api_key_env: SECONDARY_ENV.to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();

        let app = router(state);
        let cookie = login_cookie(
            app.clone(),
            "credential-catalog-user",
            "strong-credential-catalog-password-123",
        )
        .await;
        let ready_models = get_client_models(app.clone(), &key.key).await;
        assert!(client_model_ids(&ready_models).contains(&"mimo:mimo-v2.5-pro"));
        let ready_providers =
            get_console_json(app.clone(), "/admin/providers", cookie.clone()).await;
        assert_eq!(ready_providers[0]["id"], "mimo");

        // Once the only usable pool member disappears, the static key on the
        // provider must not override the explicit (now-unready) pool state.
        unsafe {
            env::remove_var(SECONDARY_ENV);
        }
        let unready_models = get_client_models(app.clone(), &key.key).await;
        assert!(!client_model_ids(&unready_models).contains(&"mimo:mimo-v2.5-pro"));
        assert_eq!(
            get_console_json(app, "/admin/providers", cookie).await,
            json!([])
        );
    }

    #[tokio::test]
    async fn admin_reload_config_updates_runtime_snapshot() {
        let upstream = spawn_openai_upstream(StatusCode::OK, "{}", "application/json").await;
        let mut state = test_state_with_admin(upstream, 1024 * 1024);
        let initial_config = state.config.snapshot();
        let mut reloaded_config = initial_config.clone();
        if let Some(provider) = reloaded_config.providers.get_mut("mimo") {
            provider.base_url = "https://api.xiaomimimo.com/v1".to_owned();
        }
        let custom_provider = reloaded_config
            .providers
            .get("mimo")
            .cloned()
            .map(|mut provider| {
                provider.display_name = "Custom".to_owned();
                provider.default_model = "custom-model".to_owned();
                provider.models = vec!["custom-model".to_owned()];
                provider.model_prefixes = vec!["custom-".to_owned()];
                provider
            })
            .unwrap();
        reloaded_config
            .providers
            .insert("custom".to_owned(), custom_provider);
        reloaded_config.provider_order.push("custom".to_owned());
        let loader_config = reloaded_config.clone();
        state.config = Arc::new(RuntimeConfig::with_loader(initial_config, move || {
            Ok(loader_config.clone())
        }));
        let app = router(state);

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

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/settings/reload-config")
                    .header("x-modelport-csrf", "1")
                    .header(COOKIE, session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["providerCount"], json!(2));
        assert_eq!(body["settings"]["gateway"]["providerOrder"][1], "custom");
        assert_eq!(body["reloadScope"]["requiresRestart"][0], "bind address");
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
    async fn admin_provider_models_post_discovers_with_csrf() {
        let upstream = spawn_openai_models_upstream(
            r#"{"data":[{"id":"mimo-v2.5-pro"},{"id":"mimo-v2.6-pro"}]}"#,
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

        let missing_csrf_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/providers/mimo/models")
                    .header(COOKIE, session_cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_csrf_response.status(), StatusCode::FORBIDDEN);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/providers/mimo/models")
                    .header("x-modelport-csrf", "1")
                    .header(COOKIE, session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["success"], json!(true));
        assert_eq!(body["modelCount"], json!(2));
        assert_eq!(body["testedCredentialId"], Value::Null);

        let models_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .extension(ConnectInfo(
                        "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                    ))
                    .header("x-api-key", CLIENT_TOKEN)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(models_response.status(), StatusCode::OK);
        let body = to_bytes(models_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let ids = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(!ids.contains(&"mimo:mimo-v2.6-pro"));
    }

    #[tokio::test]
    async fn admin_provider_test_uses_failover_pool_credential_and_records_its_id() {
        const PRIMARY_ENV: &str = "MODELPORT_PROBE_FAILOVER_PRIMARY_UNSET";
        const SECONDARY_ENV: &str = "MODELPORT_PROBE_FAILOVER_SECONDARY";
        unsafe {
            env::remove_var(PRIMARY_ENV);
            env::set_var(SECONDARY_ENV, "probe-failover-secondary-key");
        }
        let (upstream, hits) =
            spawn_credential_probe_upstream("Bearer probe-failover-secondary-key").await;
        let state = test_state_with_admin(upstream, 1024 * 1024);
        upsert_probe_credential(&state, "probe-primary", PRIMARY_ENV);
        upsert_probe_credential(&state, "probe-secondary", SECONDARY_ENV);
        let control = state.control.clone();
        let app = router(state);
        let cookie = login_cookie(app.clone(), "admin", "strong-password-123").await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/settings/test-provider")
                    .header("x-modelport-csrf", "1")
                    .header(COOKIE, cookie)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"providerId": "mimo"}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["success"], true);
        assert_eq!(body["testedCredentialId"], "probe-secondary");
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        let test_rows = control.provider_test_rows();
        assert_eq!(test_rows["mimo"]["testedCredentialId"], "probe-secondary");
        let health_rows = control.provider_credential_health_rows();
        assert_eq!(health_rows["mimo"]["probe-secondary"]["status"], "healthy");

        unsafe {
            env::remove_var(SECONDARY_ENV);
        }
    }

    #[tokio::test]
    async fn admin_provider_discovery_uses_round_robin_pool_credential() {
        const PRIMARY_ENV: &str = "MODELPORT_PROBE_ROUND_ROBIN_PRIMARY_UNSET";
        const SECONDARY_ENV: &str = "MODELPORT_PROBE_ROUND_ROBIN_SECONDARY";
        unsafe {
            env::remove_var(PRIMARY_ENV);
            env::set_var(SECONDARY_ENV, "probe-round-robin-secondary-key");
        }
        let (upstream, hits) =
            spawn_credential_probe_upstream("Bearer probe-round-robin-secondary-key").await;
        let state = test_state_with_admin(upstream, 1024 * 1024);
        upsert_probe_credential(&state, "round-robin-primary", PRIMARY_ENV);
        upsert_probe_credential(&state, "round-robin-secondary", SECONDARY_ENV);
        state
            .control
            .set_provider_credential_pool_mode("mimo", "round_robin")
            .unwrap();
        let control = state.control.clone();
        let app = router(state);
        let cookie = login_cookie(app.clone(), "admin", "strong-password-123").await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/providers/mimo/models")
                    .header("x-modelport-csrf", "1")
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["success"], true);
        assert_eq!(body["testedCredentialId"], "round-robin-secondary");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(
            control.provider_test_rows()["mimo"]["testedCredentialId"],
            "round-robin-secondary"
        );

        unsafe {
            env::remove_var(SECONDARY_ENV);
        }
    }

    #[tokio::test]
    async fn management_probe_fails_closed_for_unusable_pool_but_static_provider_still_works() {
        const MISSING_ENV: &str = "MODELPORT_PROBE_POOL_ALL_UNSET";
        unsafe {
            env::remove_var(MISSING_ENV);
        }
        let (upstream, hits) = spawn_credential_probe_upstream("Bearer upstream-key").await;
        let pooled_state = test_state(upstream.clone(), 1024 * 1024);
        upsert_probe_credential(&pooled_state, "missing-pool-key", MISSING_ENV);
        pooled_state
            .control
            .set_provider_credential_pool_mode("mimo", "round_robin")
            .unwrap();
        let pooled_provider = management_config(&pooled_state).providers["mimo"].clone();

        let pooled_probe = run_provider_management_probe(&pooled_state, "mimo", &pooled_provider)
            .await
            .unwrap();
        assert_eq!(pooled_probe.tested_credential_id, None);
        assert!(pooled_probe.result.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        let static_state = test_state(upstream, 1024 * 1024);
        let static_provider = management_config(&static_state).providers["mimo"].clone();
        let static_probe = run_provider_management_probe(&static_state, "mimo", &static_provider)
            .await
            .unwrap();
        assert_eq!(static_probe.tested_credential_id, None);
        assert_eq!(static_probe.result.unwrap(), vec!["mimo-v2.5-pro"]);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn discovered_models_remain_unroutable_until_added_to_the_organization_catalog() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        state
            .control
            .record_provider_test(
                "mimo".to_owned(),
                true,
                "discovered 2 model(s)".to_owned(),
                vec!["mimo-v2.5-pro".to_owned(), "mimo-v2.6-pro".to_owned()],
            )
            .unwrap();

        let config = effective_config(&state);
        let provider = config.providers.get("mimo").unwrap();
        assert_eq!(
            provider
                .models
                .iter()
                .filter(|model| model.as_str() == "mimo-v2.5-pro")
                .count(),
            1
        );
        assert!(!provider.models.contains(&"mimo-v2.6-pro".to_owned()));

        let rows = client_api::public_model_rows(&config);
        let ids = rows
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(!ids.contains(&"mimo:mimo-v2.6-pro"));
        assert_ne!(
            config.resolve("mimo-v2.6-pro").unwrap().model,
            "mimo-v2.6-pro"
        );
    }

    #[test]
    fn active_provider_credential_overrides_key_env_and_base_url() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        unsafe {
            env::set_var("MIMO_TEST_ACCOUNT_A", "account-a-key");
            env::set_var("MIMO_TEST_ACCOUNT_B", "account-b-key");
        }
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-a".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account A".to_owned(),
                api_key_env: "MIMO_TEST_ACCOUNT_A".to_owned(),
                base_url: Some("https://account-a.local/v1".to_owned()),
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-b".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account B".to_owned(),
                api_key_env: "MIMO_TEST_ACCOUNT_B".to_owned(),
                base_url: Some("https://account-b.local/v1".to_owned()),
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .set_active_provider_credential("mimo", "account-b")
            .unwrap();

        let config = effective_config(&state);
        let provider = config.providers.get("mimo").unwrap();
        assert_eq!(provider.api_key_env.as_deref(), Some("MIMO_TEST_ACCOUNT_B"));
        assert_eq!(provider.api_key().unwrap(), Some("account-b-key"));
        assert_eq!(provider.base_url, "https://account-b.local/v1");

        unsafe {
            env::remove_var("MIMO_TEST_ACCOUNT_A");
            env::remove_var("MIMO_TEST_ACCOUNT_B");
        }
    }

    #[test]
    fn rate_limit_rotates_provider_credential_and_clears_cooldown() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        unsafe {
            env::set_var("MIMO_ROTATE_ACCOUNT_A", "account-a-key");
            env::set_var("MIMO_ROTATE_ACCOUNT_B", "account-b-key");
        }
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-a".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account A".to_owned(),
                api_key_env: "MIMO_ROTATE_ACCOUNT_A".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-b".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account B".to_owned(),
                api_key_env: "MIMO_ROTATE_ACCOUNT_B".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();

        state
            .control
            .record_provider_outcome_for_credential(
                "mimo",
                Some("account-a"),
                false,
                429,
                Some("rate limit"),
                true,
            )
            .unwrap();

        let controls = state.control.provider_control_snapshot();
        assert_eq!(
            controls
                .active_provider_credentials
                .get("mimo")
                .map(String::as_str),
            Some("account-b")
        );
        assert!(!state.control.provider_in_cooldown("mimo"));

        unsafe {
            env::remove_var("MIMO_ROTATE_ACCOUNT_A");
            env::remove_var("MIMO_ROTATE_ACCOUNT_B");
        }
    }

    #[test]
    fn disabled_active_provider_credential_selects_next_enabled_account() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-a".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account A".to_owned(),
                api_key_env: "MIMO_DISABLE_ACCOUNT_A".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-b".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account B".to_owned(),
                api_key_env: "MIMO_DISABLE_ACCOUNT_B".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-a".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account A".to_owned(),
                api_key_env: "MIMO_DISABLE_ACCOUNT_A".to_owned(),
                base_url: None,
                status: "disabled".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();

        let controls = state.control.provider_control_snapshot();
        assert_eq!(
            controls
                .active_provider_credentials
                .get("mimo")
                .map(String::as_str),
            Some("account-b")
        );
    }

    #[test]
    fn round_robin_provider_credential_selection_rotates_available_accounts() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        unsafe {
            env::set_var("MIMO_POOL_ACCOUNT_A", "account-a-key");
            env::set_var("MIMO_POOL_ACCOUNT_B", "account-b-key");
        }
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-a".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account A".to_owned(),
                api_key_env: "MIMO_POOL_ACCOUNT_A".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-b".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account B".to_owned(),
                api_key_env: "MIMO_POOL_ACCOUNT_B".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .set_provider_credential_pool_mode("mimo", "round_robin")
            .unwrap();

        let first = state
            .control
            .select_provider_credential_for_request("mimo")
            .unwrap();
        let second = state
            .control
            .select_provider_credential_for_request("mimo")
            .unwrap();
        assert_eq!(first.id, "account-b");
        assert_eq!(second.id, "account-a");

        unsafe {
            env::remove_var("MIMO_POOL_ACCOUNT_A");
            env::remove_var("MIMO_POOL_ACCOUNT_B");
        }
    }

    #[test]
    fn automatic_credential_pool_fails_closed_when_no_account_is_usable() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "missing-account".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Missing Account".to_owned(),
                api_key_env: "MIMO_POOL_MISSING_ACCOUNT".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .set_provider_credential_pool_mode("mimo", "round_robin")
            .unwrap();
        let mut provider = state.config.snapshot().providers["mimo"].clone();

        let error = state
            .control
            .apply_selected_provider_credential_for_request("mimo", &mut provider)
            .unwrap_err();

        assert!(
            matches!(error, AppError::NotReady(message) if message.contains("no usable credential"))
        );
    }

    #[test]
    fn manual_provider_credential_pool_does_not_auto_rotate() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        unsafe {
            env::set_var("MIMO_MANUAL_ACCOUNT_A", "account-a-key");
            env::set_var("MIMO_MANUAL_ACCOUNT_B", "account-b-key");
        }
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-a".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account A".to_owned(),
                api_key_env: "MIMO_MANUAL_ACCOUNT_A".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: "account-b".to_owned(),
                provider_id: "mimo".to_owned(),
                name: "Account B".to_owned(),
                api_key_env: "MIMO_MANUAL_ACCOUNT_B".to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        state
            .control
            .set_provider_credential_pool_mode("mimo", "manual")
            .unwrap();

        state
            .control
            .record_provider_outcome_for_credential(
                "mimo",
                Some("account-a"),
                false,
                429,
                Some("rate limit"),
                true,
            )
            .unwrap();

        let controls = state.control.provider_control_snapshot();
        assert_eq!(
            controls
                .active_provider_credentials
                .get("mimo")
                .map(String::as_str),
            Some("account-a")
        );
        assert!(state.control.provider_in_cooldown("mimo"));

        unsafe {
            env::remove_var("MIMO_MANUAL_ACCOUNT_A");
            env::remove_var("MIMO_MANUAL_ACCOUNT_B");
        }
    }

    #[test]
    fn unreviewed_dynamic_provider_override_never_enters_effective_routing() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        state
            .control
            .upsert_provider_override(ProviderOverrideRecord {
                id: "local_custom".to_owned(),
                display_name: "Local Custom".to_owned(),
                protocol: "openai-compat".to_owned(),
                base_url: "http://127.0.0.1:11434/v1".to_owned(),
                api_key_env: None,
                api_key_required: false,
                default_model: "qwen-local".to_owned(),
                models: vec!["qwen-local".to_owned()],
                model_prefixes: vec!["qwen-".to_owned()],
                passthrough_unknown_models: true,
                max_tokens_field: "max_tokens".to_owned(),
                deduplicate_stream_text: false,
                buffer_stream_text: false,
                fidelity_mode: "strict".to_owned(),
                tool_use: ToolUseConfig::default_for_provider(
                    "local_custom",
                    ProviderProtocol::OpenaiCompat,
                    false,
                ),
                model_profile_defaults: Default::default(),
                model_profiles: Default::default(),
                reasoning: Default::default(),
                sampling: Default::default(),
                token_counting: Default::default(),
                static_headers: Default::default(),
                request_timeout_ms: None,
                stream_idle_timeout_ms: None,
                retry: Default::default(),
                pricing: None,
                model_pricing: Default::default(),
                trust_upstream_cost: false,
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();

        let config = effective_config(&state);
        assert_eq!(config.resolve("qwen-local").unwrap().provider_id, "mimo");
        assert!(!client_api::public_model_rows(&config).iter().any(|row| {
            row.get("id").and_then(Value::as_str) == Some("local_custom:qwen-local")
        }));

        state
            .control
            .set_provider_disabled("local_custom", true)
            .unwrap();
        assert!(
            !effective_config(&state)
                .providers
                .contains_key("local_custom")
        );
        assert!(
            !management_config(&state)
                .providers
                .contains_key("local_custom")
        );
    }

    #[test]
    fn provider_model_override_disables_discovered_model() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        state
            .control
            .record_provider_test(
                "mimo".to_owned(),
                true,
                "discovered 2 model(s)".to_owned(),
                vec!["mimo-v2.5-pro".to_owned(), "mimo-v2.6-pro".to_owned()],
            )
            .unwrap();
        state
            .control
            .upsert_provider_model_override(ProviderModelOverrideRecord {
                provider_id: "mimo".to_owned(),
                model: "mimo-v2.6-pro".to_owned(),
                status: "disabled".to_owned(),
                display_name: None,
                family: Some("小米 MiMo".to_owned()),
                context_window: Some(128_000),
                profile: Default::default(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();

        let config = effective_config(&state);
        let provider = config.providers.get("mimo").unwrap();
        assert!(!provider.models.contains(&"mimo-v2.6-pro".to_owned()));
        assert!(
            !client_api::public_model_rows(&config)
                .iter()
                .any(|row| row.get("id").and_then(Value::as_str) == Some("mimo-v2.6-pro"))
        );
    }

    #[test]
    fn provider_delete_dependencies_include_routes_aliases_and_policies() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        state
            .control
            .upsert_alias("fast".to_owned(), "mimo:mimo-v2.5-pro".to_owned())
            .unwrap();
        state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: "usr_test".to_owned(),
                username: Some("test-user".to_owned()),
                name: "mimo only".to_owned(),
                principal_type: None,
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: None,
                allowed_providers: Some(vec!["mimo".to_owned()]),
                expires_at: None,
            })
            .unwrap();

        let dependencies = provider_delete_dependencies(&state, &management_config(&state), "mimo");
        assert!(
            dependencies
                .iter()
                .any(|row| row.get("type").and_then(Value::as_str) == Some("defaultProvider"))
        );
        assert!(
            dependencies
                .iter()
                .any(|row| row.get("type").and_then(Value::as_str) == Some("providerOrder"))
        );
        assert!(
            dependencies.iter().any(
                |row| row.get("type").and_then(Value::as_str) == Some("alias")
                    && row.get("id").and_then(Value::as_str) == Some("fast")
            )
        );
        assert!(
            dependencies
                .iter()
                .any(|row| row.get("type").and_then(Value::as_str) == Some("apiKey"))
        );
    }

    #[test]
    fn deployment_network_values_reject_invalid_proxies_and_origins() {
        assert!(TrustedProxyConfig::from_value(Some("10.0.0.0/8,192.0.2.10")).is_ok());
        assert!(TrustedProxyConfig::from_value(Some("not-a-network")).is_err());

        assert!(
            validate_allowed_origins(Some("https://console.example.com,http://127.0.0.1:5173"))
                .is_ok()
        );
        assert!(validate_allowed_origins(Some("console.example.com")).is_err());
        assert!(validate_allowed_origins(Some("https://user@example.com")).is_err());
        assert!(validate_allowed_origins(Some("https://example.com/admin")).is_err());
    }

    #[test]
    fn client_ip_uses_peer_when_forwarded_header_is_untrusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.9"));
        let trusted = TrustedProxyConfig::for_tests();

        assert_eq!(
            client_ip(
                &headers,
                Some("203.0.113.10:48178".parse().unwrap()),
                &trusted,
            ),
            Some("203.0.113.10".to_owned())
        );
    }

    #[test]
    fn client_ip_uses_forwarded_header_from_trusted_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.9"));
        let trusted = TrustedProxyConfig::for_tests();

        assert_eq!(
            client_ip(&headers, Some("127.0.0.1:48178".parse().unwrap()), &trusted,),
            Some("198.51.100.9".to_owned())
        );
    }

    #[test]
    fn client_ip_ignores_spoofed_leftmost_forwarded_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("10.0.0.7, 198.51.100.9"),
        );
        let trusted = TrustedProxyConfig::for_tests();

        assert_eq!(
            client_ip(&headers, Some("127.0.0.1:48178".parse().unwrap()), &trusted,),
            Some("198.51.100.9".to_owned())
        );
    }

    #[tokio::test]
    async fn discover_anthropic_models_checks_required_api_key() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let provider = ProviderConfig {
            display_name: "Anthropic".to_owned(),
            protocol: ProviderProtocol::Anthropic,
            // Keep the missing-secret assertion independent of external DNS.
            base_url: "http://127.0.0.1:1".to_owned(),
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
            tool_use: ToolUseConfig::default_for_provider(
                "anthropic",
                ProviderProtocol::Anthropic,
                false,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: Default::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };

        let err = discover_provider_models(&state, "anthropic", &provider)
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::MissingSecret(name) if name == "ANTHROPIC_API_KEY"));
    }

    #[tokio::test]
    async fn discover_anthropic_models_records_only_real_network_probe_success() {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler_hits = hits.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move |Json(body): Json<Value>| {
                let hits = handler_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(body["max_tokens"], json!(1));
                    assert_eq!(body["messages"][0]["content"], json!("Reply OK."));
                    Json(json!({
                        "id": "msg_probe",
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "text", "text": "OK"}],
                    }))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let provider = ProviderConfig {
            display_name: "Anthropic-compatible".to_owned(),
            protocol: ProviderProtocol::Anthropic,
            base_url: format!("http://{address}"),
            api_key_env: Some("ANTHROPIC_API_KEY".to_owned()),
            api_key: Some("test-secret".to_owned()),
            api_key_required: true,
            default_model: "claude-test".to_owned(),
            models: vec!["claude-test".to_owned()],
            model_prefixes: vec!["claude-".to_owned()],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::Strict,
            tool_use: ToolUseConfig::default_for_provider(
                "anthropic",
                ProviderProtocol::Anthropic,
                false,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: Default::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };

        let models = discover_provider_models(&state, "local_anthropic", &provider)
            .await
            .unwrap();

        assert_eq!(models, vec!["claude-test"]);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn discover_openai_models_does_not_treat_catalog_only_as_production_ready() {
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data": [{"id": "mimo-v2.5-pro"}]})) }),
            )
            .route(
                "/v1/chat/completions",
                post(|| async {
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": {"message": "probe rejected"}})),
                    )
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let state = test_state(format!("http://{address}/v1"), 1024 * 1024);
        let provider = state.config.snapshot().providers["mimo"].clone();

        let error = discover_provider_models(&state, "mimo", &provider)
            .await
            .unwrap_err();

        assert!(matches!(error, AppError::Upstream { status: 401, .. }));
    }

    #[tokio::test]
    async fn discover_cpa_claude_models_through_shared_compatibility_catalog() {
        let upstream = spawn_openai_models_upstream(
            r#"{"data":[{"id":"claude-sonnet-4-6"},{"id":"claude-opus-4-7"}]}"#,
        )
        .await;
        let state = test_state(upstream.clone(), 1024 * 1024);
        let provider = ProviderConfig {
            display_name: "CPA · Claude Code".to_owned(),
            protocol: ProviderProtocol::Anthropic,
            base_url: upstream.trim_end_matches("/v1").to_owned(),
            api_key_env: Some("CPA_CLAUDE_API_KEY".to_owned()),
            api_key: Some("test-cpa-client-key".to_owned()),
            api_key_required: true,
            default_model: "claude-sonnet-4-6".to_owned(),
            models: vec!["claude-sonnet-4-6".to_owned()],
            model_prefixes: Vec::new(),
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::BestEffort,
            tool_use: ToolUseConfig::default_for_provider(
                "cpa_claude",
                ProviderProtocol::Anthropic,
                false,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: Default::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };

        let models = discover_provider_models(&state, "cpa_claude", &provider)
            .await
            .unwrap();

        assert_eq!(models, vec!["claude-sonnet-4-6", "claude-opus-4-7"]);
    }

    #[tokio::test]
    async fn cpa_codex_stream_uses_openai_adapter_and_keeps_provider_evidence() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"data: {"choices":[{"delta":{"content":"hello from CPA"},"finish_reason":null,"index":0}]}
data: {"choices":[{"delta":{},"finish_reason":"stop","index":0}]}
data: [DONE]

"#,
            "text/event-stream",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        retarget_test_provider(
            &mut state,
            "mimo",
            "cpa_codex",
            "CPA · OpenAI Codex",
            "gpt-5.3-codex",
        );
        let ledger = state.ledger.clone();
        let mut body = message_body(true);
        body["model"] = json!("cpa_codex:gpt-5.3-codex");

        let (status, response) = post_message(router(state), body).await;

        assert_eq!(status, StatusCode::OK);
        assert!(response.contains("hello from CPA"));
        assert!(!response.contains("event: error"));
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows[0]["provider"], "cpa_codex");
        assert_eq!(rows[0]["model"], "cpa_codex:gpt-5.3-codex");
    }

    #[tokio::test]
    async fn cpa_codex_upstream_error_is_attributed_to_the_cpa_channel() {
        let upstream = spawn_openai_upstream(
            StatusCode::SERVICE_UNAVAILABLE,
            r#"{"error":{"message":"CPA account unavailable"}}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        retarget_test_provider(
            &mut state,
            "mimo",
            "cpa_codex",
            "CPA · OpenAI Codex",
            "gpt-5.3-codex",
        );
        let ledger = state.ledger.clone();
        let mut body = message_body(false);
        body["model"] = json!("cpa_codex:gpt-5.3-codex");

        let (status, _) = post_message(router(state), body).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows[0]["provider"], "cpa_codex");
        assert_eq!(rows[0]["status"], "error");
        assert_eq!(rows[0]["terminalReason"], "failed_before_response");
    }

    #[tokio::test]
    async fn cpa_claude_tool_use_keeps_anthropic_protocol_and_provider_evidence() {
        let upstream = spawn_anthropic_upstream(
            StatusCode::OK,
            r#"{
                "id":"msg_cpa",
                "type":"message",
                "role":"assistant",
                "model":"claude-sonnet-4-6",
                "content":[{
                    "type":"tool_use",
                    "id":"toolu_cpa",
                    "name":"weather",
                    "input":{"city":"Shanghai"}
                }],
                "stop_reason":"tool_use",
                "usage":{"input_tokens":8,"output_tokens":4}
            }"#,
            "application/json",
        )
        .await;
        let mut state = test_anthropic_state(upstream, 1024 * 1024);
        retarget_test_provider(
            &mut state,
            "anthropic",
            "cpa_claude",
            "CPA · Claude Code",
            "claude-sonnet-4-6",
        );
        let ledger = state.ledger.clone();
        let mut body = message_body(false);
        body["model"] = json!("cpa_claude:claude-sonnet-4-6");
        body["tools"] = json!([{
            "name":"weather",
            "description":"Look up weather",
            "input_schema":{
                "type":"object",
                "properties":{"city":{"type":"string"}},
                "required":["city"]
            }
        }]);

        let (status, response) = post_message(router(state), body).await;

        assert_eq!(status, StatusCode::OK, "{response}");
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["content"][0]["type"], "tool_use");
        assert_eq!(response["content"][0]["input"]["city"], "Shanghai");
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows[0]["provider"], "cpa_claude");
        assert_eq!(rows[0]["toolOutcome"], "tool_called");
    }

    #[test]
    fn settings_update_rejects_read_only_runtime_fields() {
        assert!(
            validate_settings_update_body(&json!({
                "gateway": { "defaultProvider": "deepseek" }
            }))
            .is_ok()
        );
        assert!(
            validate_settings_update_body(&json!({
                "server": { "bindAddress": "0.0.0.0:17878" }
            }))
            .is_err()
        );
        assert!(
            validate_settings_update_body(&json!({
                "gateway": { "requestTimeoutSecs": 1 }
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn generated_request_id_is_persisted_with_usage() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();
        let response =
            post_message_response(router(state), CLIENT_TOKEN, message_body(false)).await;
        let status = response.status();
        let response_request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap()
            .to_owned();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response.headers()["x-modelport-logical-model"],
            "mimo-v2.5-pro"
        );
        assert_eq!(response.headers()["x-modelport-resolved-provider"], "mimo");
        assert_eq!(
            response.headers()["x-modelport-resolved-model"],
            "mimo-v2.5-pro"
        );
        assert_eq!(
            response.headers()["x-modelport-routing-policy"],
            "local_strict"
        );
        assert_eq!(response.headers()["x-modelport-cloud-egress"], "false");
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0]["requestId"]
                .as_str()
                .is_some_and(|request_id| !request_id.is_empty())
        );
        assert_eq!(rows[0]["requestId"], response_request_id);
        assert!(
            rows[0]["attemptId"]
                .as_str()
                .is_some_and(|attempt_id| attempt_id.starts_with("att_"))
        );
        assert_eq!(rows[0]["terminalReason"], "completed");
    }

    #[tokio::test]
    async fn error_contract_includes_safe_request_id_retryability_and_action() {
        let app = router(test_state("http://127.0.0.1:9".to_owned(), 1024 * 1024));
        let response = post_message_response(
            app,
            CLIENT_TOKEN,
            json!({
                "model": "mimo-v2.5-pro",
                "messages": [{"role": "user", "content": "hello"}]
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap()
            .to_owned();
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["error"]["code"], "invalid_request");
        assert_eq!(body["error"]["request_id"], request_id);
        assert_eq!(body["error"]["retryable"], false);
        assert!(
            body["error"]["action"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[tokio::test]
    async fn routes_openai_chat_completions_and_records_client_protocol() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl-direct",
                "object": "chat.completion",
                "created": 1,
                "model": "provider-physical-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "hello from OpenAI"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
            }"#,
            "application/json",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();

        let (status, body) = post_chat_completion(router(state), chat_body(false)).await;

        assert_eq!(status, StatusCode::OK);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["model"], "mimo-v2.5-pro");
        assert_eq!(
            body["choices"][0]["message"]["content"],
            "hello from OpenAI"
        );
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["clientProtocol"], "openai-chat-completions");
        assert_eq!(rows[0]["requestPath"], "/v1/chat/completions");
        assert_eq!(rows[0]["billingMode"], "upstream-returned");
        assert_eq!(rows[0]["inputTokens"], 7);
        assert_eq!(rows[0]["outputTokens"], 3);
    }

    #[tokio::test]
    async fn idempotency_key_prevents_duplicate_provider_calls() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{
                "id": "chatcmpl-idempotent",
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "only once"},
                    "finish_reason": "stop"
                }]
            }"#,
            "application/json",
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();
        let app = router(state);
        let request = chat_body(false);

        let (first_status, _) =
            post_chat_completion_idempotent(app.clone(), request.clone(), "retry-claim-1").await;
        let (second_status, second_body) =
            post_chat_completion_idempotent(app, request, "retry-claim-1").await;

        assert_eq!(first_status, StatusCode::OK);
        assert_eq!(second_status, StatusCode::CONFLICT);
        assert!(second_body.contains("idempotency_conflict"));
        assert!(second_body.contains("response replay is not available"));
        assert_eq!(ledger.usage_rows().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn invalid_idempotency_key_is_rejected_before_provider_routing() {
        let state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let ledger = state.ledger.clone();

        let (status, body) =
            post_chat_completion_idempotent(router(state), chat_body(false), "contains whitespace")
                .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("visible ASCII characters without whitespace"));
        assert!(ledger.usage_rows().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn concurrent_idempotency_claim_allows_exactly_one_request() {
        let upstream = spawn_delayed_openai_upstream(
            r#"{"id":"chatcmpl-race","choices":[{"message":{"role":"assistant","content":"once"},"finish_reason":"stop"}]}"#,
        )
        .await;
        let state = test_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();
        let app = router(state);
        let request = chat_body(false);

        let (first, second) = tokio::join!(
            post_chat_completion_idempotent(app.clone(), request.clone(), "concurrent-claim"),
            post_chat_completion_idempotent(app, request, "concurrent-claim")
        );
        let statuses = [first.0, second.0];

        assert!(statuses.contains(&StatusCode::OK));
        assert!(statuses.contains(&StatusCode::CONFLICT));
        assert_eq!(ledger.usage_rows().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn openai_chat_stream_preserves_usage_chunk_and_reconciles_actual_usage() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"data: {"id":"chatcmpl-stream","object":"chat.completion.chunk","created":1,"model":"provider-physical-model","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}

data: {"id":"chatcmpl-stream","object":"chat.completion.chunk","created":1,"model":"provider-physical-model","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-stream","object":"chat.completion.chunk","created":1,"model":"provider-physical-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: {"id":"chatcmpl-stream","object":"chat.completion.chunk","created":1,"model":"provider-physical-model","choices":[],"usage":{"prompt_tokens":11,"completion_tokens":5,"total_tokens":16}}

data: [DONE]

"#,
            "text/event-stream",
        )
        .await;
        let state = test_state_with_flags(upstream, 1024 * 1024, false, false);
        let ledger = state.ledger.clone();
        let mut request = chat_body(true);
        request["stream_options"] = json!({ "include_usage": true });

        let (status, body) = post_chat_completion(router(state), request).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""object":"chat.completion.chunk""#));
        assert!(body.contains(r#""model":"mimo-v2.5-pro""#));
        assert!(!body.contains("provider-physical-model"));
        assert!(body.contains(r#""choices":[]"#));
        assert!(body.contains(r#""completion_tokens":5"#));
        assert!(body.contains("data: [DONE]"));
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["status"], "success");
        assert_eq!(rows[0]["terminalReason"], "completed");
        assert_eq!(rows[0]["billingMode"], "upstream-returned");
        assert_eq!(rows[0]["inputTokens"], 11);
        assert_eq!(rows[0]["outputTokens"], 5);
    }

    #[tokio::test]
    async fn converts_openai_chat_request_and_anthropic_response_across_protocols() {
        let upstream = spawn_anthropic_upstream(
            StatusCode::OK,
            r#"{
                "id": "msg_cross_protocol",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-6",
                "content": [{"type": "text", "text": "hello from Anthropic"}],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {"input_tokens": 9, "output_tokens": 4}
            }"#,
            "application/json",
        )
        .await;
        let state = test_anthropic_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();
        let request = json!({
            "model": "claude-sonnet-4-6",
            "messages": [
                {"role": "developer", "content": "Be concise"},
                {"role": "user", "content": "hello"}
            ]
        });

        let (status, body) = post_chat_completion(router(state), request).await;

        assert_eq!(status, StatusCode::OK);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(
            body["choices"][0]["message"]["content"],
            "hello from Anthropic"
        );
        assert_eq!(body["usage"]["prompt_tokens"], 9);
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows[0]["provider"], "anthropic");
        assert_eq!(rows[0]["protocol"], "anthropic");
        assert_eq!(rows[0]["clientProtocol"], "openai-chat-completions");
    }

    #[tokio::test]
    async fn converts_anthropic_stream_to_openai_chunks_with_usage() {
        let upstream = spawn_anthropic_upstream(
            StatusCode::OK,
            r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_stream_cross","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[],"stop_reason":null,"usage":{"input_tokens":6,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello cross stream"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}}

event: message_stop
data: {"type":"message_stop"}

"#,
            "text/event-stream",
        )
        .await;
        let state = test_anthropic_state(upstream, 1024 * 1024);
        let ledger = state.ledger.clone();
        let request = json!({
            "model": "claude-sonnet-4-6",
            "stream": true,
            "stream_options": {"include_usage": true},
            "messages": [{"role": "user", "content": "hello"}]
        });

        let (status, body) = post_chat_completion(router(state), request).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""object":"chat.completion.chunk""#));
        assert!(body.contains(r#""content":"hello cross stream""#));
        assert!(body.contains(r#""finish_reason":"stop""#));
        assert!(body.contains(r#""choices":[]"#));
        assert!(body.contains(r#""prompt_tokens":6"#));
        assert!(body.contains(r#""completion_tokens":2"#));
        assert!(body.contains("data: [DONE]"));
        let rows = wait_for_usage_rows(&ledger, 1).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["status"], "success");
        assert_eq!(rows[0]["billingMode"], "upstream-returned");
        assert_eq!(rows[0]["inputTokens"], 6);
        assert_eq!(rows[0]["outputTokens"], 2);
    }

    #[tokio::test]
    async fn rejects_unsupported_openai_chat_fields_instead_of_dropping_them() {
        let app = router(test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024));
        let mut request = chat_body(false);
        request["modalities"] = json!(["text"]);

        let (status, body) = post_chat_completion(app, request).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("unsupported Chat Completions field(s): modalities"));
    }

    #[tokio::test]
    async fn rejects_openai_parameters_that_anthropic_cannot_preserve() {
        let app = router(test_anthropic_state(
            "http://127.0.0.1:1".to_owned(),
            1024 * 1024,
        ));
        let mut request = json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role": "user", "content": "hello"}]
        });
        request["presence_penalty"] = json!(0.5);

        let (status, body) = post_chat_completion(app, request).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("presence_penalty cannot be preserved"));
    }

    #[tokio::test]
    async fn proxies_anthropic_count_tokens_to_openai_compatible_capability() {
        let upstream = Router::new().route(
            "/v1/messages/count_tokens",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["authorization"], "Bearer upstream-key");
                assert_eq!(body["model"], "mimo-v2.5-pro");
                assert_eq!(body["messages"][0]["content"], "你好，world");
                assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
                Json(json!({"input_tokens": 13, "ignored": true}))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        let app = router(test_state(format!("http://{addr}/v1"), 1024 * 1024));

        let (status, body) = post_count_tokens(
            app,
            json!({
                "model": "mimo-v2.5-pro",
                "thinking": {"type": "disabled"},
                "messages": [{"role": "user", "content": "你好，world"}]
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!({"input_tokens": 13})
        );
    }

    #[tokio::test]
    async fn proxies_count_tokens_to_native_anthropic_capability() {
        let upstream = Router::new().route(
            "/v1/messages/count_tokens",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers["x-api-key"], "upstream-key");
                assert_eq!(headers["anthropic-version"], "2023-06-01");
                assert_eq!(body["model"], "claude-sonnet-4-6");
                Json(json!({"input_tokens": 9}))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        let app = router(test_anthropic_state(format!("http://{addr}"), 1024 * 1024));

        let (status, body) = post_count_tokens(
            app,
            json!({
                "model": "claude-sonnet-4-6",
                "messages": [{"role": "user", "content": "hello"}]
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!({"input_tokens": 9})
        );
    }

    #[tokio::test]
    async fn count_tokens_requires_explicit_provider_capability() {
        let mut state = test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024);
        let mut config = state.config.snapshot();
        config.providers.get_mut("mimo").unwrap().token_counting = Default::default();
        state.config = Arc::new(RuntimeConfig::new(config));

        let (status, body) = post_count_tokens(
            router(state),
            json!({
                "model": "mimo-v2.5-pro",
                "messages": [{"role": "user", "content": "hello"}]
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("does not enable Anthropic token counting"));
    }

    #[tokio::test]
    async fn count_tokens_reuses_anthropic_input_guardrails() {
        let app = router(test_state("http://127.0.0.1:1/v1".to_owned(), 1024 * 1024));

        let (status, body) =
            post_count_tokens(app, json!({"model": "mimo-v2.5-pro", "messages": []})).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("messages must not be empty"));
    }

    async fn post_message(app: Router, body: Value) -> (StatusCode, String) {
        post_message_with_key(app, CLIENT_TOKEN, body).await
    }

    async fn login_cookie(app: Router, username: &str, password: &str) -> HeaderValue {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/auth/login")
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(
                        json!({ "username": username, "password": password }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response
            .headers()
            .get(SET_COOKIE)
            .expect("login should set session cookie")
            .clone()
    }

    async fn post_retention(app: Router, cookie: HeaderValue, input: Value) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/retention/run")
                    .header(COOKIE, cookie)
                    .header("x-modelport-csrf", "1")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(input.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        (status, body)
    }

    async fn get_console_json(app: Router, uri: &str, cookie: HeaderValue) -> Value {
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "GET {uri}");
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    async fn get_client_models(app: Router, key: &str) -> Value {
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .extension(ConnectInfo(
                        "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                    ))
                    .header("x-api-key", key)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    fn client_model_ids(models: &Value) -> Vec<&str> {
        models["data"]
            .as_array()
            .expect("model catalog data")
            .iter()
            .filter_map(|row| row["id"].as_str())
            .collect()
    }

    fn assert_catalog_is_redacted(value: &Value) {
        match value {
            Value::Array(values) => values.iter().for_each(assert_catalog_is_redacted),
            Value::Object(object) => {
                for (key, value) in object {
                    let normalized = key.to_ascii_lowercase();
                    assert!(
                        !matches!(
                            normalized.as_str(),
                            "baseurl"
                                | "apikeyenv"
                                | "credentials"
                                | "health"
                                | "error"
                                | "lasterror"
                                | "errormessage"
                                | "pricing"
                        ) && !normalized.contains("path"),
                        "sensitive catalog field {key} was returned"
                    );
                    assert_catalog_is_redacted(value);
                }
            }
            _ => {}
        }
    }

    fn create_test_api_key(
        state: &AppState,
        user: &PublicUser,
        name: &str,
    ) -> crate::control::CreatedApiKey {
        state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: user.id.clone(),
                username: Some(user.username.clone()),
                name: name.to_owned(),
                principal_type: Some("user".to_owned()),
                purpose: None,
                group: None,
                team_id: None,
                allowed_models: None,
                allowed_providers: None,
                expires_at: None,
            })
            .unwrap()
    }

    async fn wait_for_usage_rows(
        ledger: &EnterpriseLedger,
        expected: usize,
    ) -> Vec<serde_json::Value> {
        for _ in 0..100 {
            let rows = ledger.usage_rows().await.unwrap();
            if rows.len() >= expected {
                return rows;
            }
            tokio::task::yield_now().await;
        }
        panic!("timed out waiting for {expected} terminal usage row(s)");
    }

    async fn post_chat_completion(app: Router, body: Value) -> (StatusCode, String) {
        let response = post_json_response(
            app,
            "/v1/chat/completions",
            "authorization",
            &format!("Bearer {CLIENT_TOKEN}"),
            body,
        )
        .await;
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn post_count_tokens(app: Router, body: Value) -> (StatusCode, String) {
        let response = post_json_response(
            app,
            "/v1/messages/count_tokens",
            "x-api-key",
            CLIENT_TOKEN,
            body,
        )
        .await;
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn post_chat_completion_idempotent(
        app: Router,
        body: Value,
        idempotency_key: &str,
    ) -> (StatusCode, String) {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .extension(ConnectInfo(
                        "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                    ))
                    .header("authorization", format!("Bearer {CLIENT_TOKEN}"))
                    .header("idempotency-key", idempotency_key)
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn post_message_with_key(app: Router, key: &str, body: Value) -> (StatusCode, String) {
        let response = post_message_response(app, key, body).await;

        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn post_message_response(app: Router, key: &str, body: Value) -> Response {
        post_json_response(app, "/v1/messages", "x-api-key", key, body).await
    }

    async fn post_json_response(
        app: Router,
        uri: &str,
        auth_header: &str,
        auth_value: &str,
        body: Value,
    ) -> Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .extension(ConnectInfo(
                    "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                ))
                .header(auth_header, auth_value)
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn post_scoped_json_response(
        app: Router,
        uri: &str,
        auth_header: &str,
        auth_value: &str,
        scope: (&str, &str, &str),
        body: Value,
    ) -> Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .extension(ConnectInfo(
                    "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                ))
                .header(auth_header, auth_value)
                .header(&ORGANIZATION_ID, scope.0)
                .header(&PROJECT_ID, scope.1)
                .header(&ENVIRONMENT_ID, scope.2)
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn response_body(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(body.to_vec()).unwrap()
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

    async fn spawn_delayed_openai_upstream(body: &'static str) -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || async move {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                (StatusCode::OK, [(CONTENT_TYPE, "application/json")], body)
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{addr}/v1")
    }

    async fn spawn_anthropic_upstream(
        status: StatusCode,
        body: &'static str,
        content_type: &'static str,
    ) -> String {
        let app = Router::new().route(
            "/v1/messages",
            post(move || async move { (status, [(CONTENT_TYPE, content_type)], body) }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{addr}")
    }

    async fn spawn_openai_models_upstream(body: &'static str) -> String {
        let app =
            Router::new()
                .route(
                    "/v1/models",
                    get(move || async move {
                        (StatusCode::OK, [(CONTENT_TYPE, "application/json")], body)
                    }),
                )
                .route(
                    "/v1/chat/completions",
                    post(|| async {
                        Json(json!({
                            "id": "chatcmpl_probe",
                            "choices": [{"message": {"role": "assistant", "content": "OK"}}],
                        }))
                    }),
                )
                .route(
                    "/v1/messages",
                    post(|| async {
                        Json(json!({
                            "id": "msg_probe",
                            "type": "message",
                            "content": [{"type": "text", "text": "OK"}],
                        }))
                    }),
                );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{addr}/v1")
    }

    async fn spawn_credential_probe_upstream(
        expected_bearer: &'static str,
    ) -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let model_hits = hits.clone();
        let chat_hits = hits.clone();
        let app = Router::new()
            .route(
                "/v1/models",
                get(move |headers: HeaderMap| async move {
                    model_hits.fetch_add(1, Ordering::SeqCst);
                    if headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        != Some(expected_bearer)
                    {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({"error": {"message": "wrong test credential"}})),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({"data": [{"id": "mimo-v2.5-pro"}]})),
                    )
                }),
            )
            .route(
                "/v1/chat/completions",
                post(move |headers: HeaderMap| async move {
                    chat_hits.fetch_add(1, Ordering::SeqCst);
                    if headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        != Some(expected_bearer)
                    {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({"error": {"message": "wrong test credential"}})),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "id": "chatcmpl_probe",
                            "choices": [{"message": {"role": "assistant", "content": "OK"}}],
                        })),
                    )
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{addr}/v1"), hits)
    }

    fn upsert_probe_credential(state: &AppState, credential_id: &str, api_key_env: &str) {
        state
            .control
            .upsert_provider_credential(ProviderCredentialRecord {
                id: credential_id.to_owned(),
                provider_id: "mimo".to_owned(),
                name: credential_id.to_owned(),
                api_key_env: api_key_env.to_owned(),
                base_url: None,
                status: "active".to_owned(),
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
    }

    #[test]
    fn dual_approval_is_optional_for_small_team_mode_and_required_when_enabled() {
        let mut state = test_state("http://127.0.0.1:9/v1".to_owned(), 1024);
        let headers = HeaderMap::new();
        let payload = json!({ "role": "user" });

        assert_eq!(
            require_high_risk_change(
                &state,
                &headers,
                "identity.permission",
                "user:new:test-user",
                &payload,
            )
            .unwrap(),
            None,
        );

        Arc::get_mut(&mut state.security)
            .expect("test security policy should be uniquely owned")
            .require_dual_approval = true;
        assert!(matches!(
            require_high_risk_change(
                &state,
                &headers,
                "identity.permission",
                "user:new:test-user",
                &payload,
            ),
            Err(AppError::Forbidden(_))
        ));
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
            tool_use: ToolUseConfig::default_for_provider(
                "mimo",
                ProviderProtocol::OpenaiCompat,
                deduplicate_stream_text,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: ReasoningConfig {
                mode: ReasoningMode::LlamaCpp,
                default_enabled: None,
                model_enabled: Default::default(),
                default_effort: None,
                model_effort: Default::default(),
                default_budget_tokens: None,
                model_budget_tokens: Default::default(),
            },
            sampling: Default::default(),
            token_counting: TokenCountingConfig {
                mode: TokenCountingMode::Anthropic,
                context_tokens: None,
                recommended_reasoning_input_tokens: None,
                model_recommended_input_tokens: Default::default(),
                max_output_tokens: None,
                model_max_output_tokens: Default::default(),
            },
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };

        AppState {
            config: Arc::new(RuntimeConfig::new(AppConfig {
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                max_request_body_bytes,
                max_concurrent_requests: 16,
                auth_token: Some(CLIENT_TOKEN.to_owned()),
                default_provider: "mimo".to_owned(),
                provider_order: vec!["mimo".to_owned()],
                providers: HashMap::from([("mimo".to_owned(), provider)]),
                aliases: HashMap::new(),
                smart_routing: Default::default(),
                runtime_adapters: Default::default(),
            })),
            auth: Arc::new(AuthStore::for_tests()),
            oidc: Arc::new(OidcService::disabled()),
            control: Arc::new(ControlStore::for_tests()),
            security: Arc::new(GatewaySecurityPolicy::for_tests()),
            rate_limiter: Arc::new(RateLimiter::disabled()),
            stream_permits: Arc::new(tokio::sync::Semaphore::new(16)),
            trusted_proxies: Arc::new(TrustedProxyConfig::for_tests()),
            transport: HttpTransport::new().unwrap(),
            metrics: Arc::new(Metrics::new()),
            smart_router: Arc::new(SmartRouter::new()),
            governance: Arc::new(GovernanceStore::for_tests()),
            local_scheduler: LocalScheduler::new(
                crate::governance::LocalSchedulerConfig::for_tests(),
            ),
            ledger: Arc::new(EnterpriseLedger::memory()),
            finalizers: Arc::new(FinalizationTracker::default()),
            draining: Arc::new(AtomicBool::new(false)),
            retention_previews: Arc::new(RetentionPreviewStore::default()),
        }
    }

    fn retarget_test_provider(
        state: &mut AppState,
        current_id: &str,
        provider_id: &str,
        display_name: &str,
        model: &str,
    ) {
        let mut config = state.config.snapshot();
        let mut provider = config.providers.remove(current_id).expect("test provider");
        provider.display_name = display_name.to_owned();
        provider.api_key_env = Some(
            match provider_id {
                "cpa_codex" => "CPA_CODEX_API_KEY",
                "cpa_claude" => "CPA_CLAUDE_API_KEY",
                _ => panic!("unsupported CPA test provider"),
            }
            .to_owned(),
        );
        provider.default_model = model.to_owned();
        provider.models = vec![model.to_owned()];
        provider.model_prefixes.clear();
        provider.passthrough_unknown_models = false;
        provider.tool_use =
            ToolUseConfig::default_for_provider(provider_id, provider.protocol, false);
        config.default_provider = provider_id.to_owned();
        config.provider_order = vec![provider_id.to_owned()];
        config.providers.insert(provider_id.to_owned(), provider);
        state.config = Arc::new(RuntimeConfig::new(config));
    }

    fn test_anthropic_state(base_url: String, max_request_body_bytes: usize) -> AppState {
        let provider = ProviderConfig {
            display_name: "Anthropic".to_owned(),
            protocol: ProviderProtocol::Anthropic,
            base_url,
            api_key_env: None,
            api_key: Some("upstream-key".to_owned()),
            api_key_required: true,
            default_model: "claude-sonnet-4-6".to_owned(),
            models: vec!["claude-sonnet-4-6".to_owned()],
            model_prefixes: vec!["claude-".to_owned()],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::Strict,
            tool_use: ToolUseConfig::default_for_provider(
                "anthropic",
                ProviderProtocol::Anthropic,
                false,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: TokenCountingConfig {
                mode: TokenCountingMode::Anthropic,
                context_tokens: None,
                recommended_reasoning_input_tokens: None,
                model_recommended_input_tokens: Default::default(),
                max_output_tokens: None,
                model_max_output_tokens: Default::default(),
            },
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: Default::default(),
            trust_upstream_cost: false,
        };

        AppState {
            config: Arc::new(RuntimeConfig::new(AppConfig {
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                max_request_body_bytes,
                max_concurrent_requests: 16,
                auth_token: Some(CLIENT_TOKEN.to_owned()),
                default_provider: "anthropic".to_owned(),
                provider_order: vec!["anthropic".to_owned()],
                providers: HashMap::from([("anthropic".to_owned(), provider)]),
                aliases: HashMap::new(),
                smart_routing: Default::default(),
                runtime_adapters: Default::default(),
            })),
            auth: Arc::new(AuthStore::for_tests()),
            oidc: Arc::new(OidcService::disabled()),
            control: Arc::new(ControlStore::for_tests()),
            security: Arc::new(GatewaySecurityPolicy::for_tests()),
            rate_limiter: Arc::new(RateLimiter::disabled()),
            stream_permits: Arc::new(tokio::sync::Semaphore::new(16)),
            trusted_proxies: Arc::new(TrustedProxyConfig::for_tests()),
            transport: HttpTransport::new().unwrap(),
            metrics: Arc::new(Metrics::new()),
            smart_router: Arc::new(SmartRouter::new()),
            governance: Arc::new(GovernanceStore::for_tests()),
            local_scheduler: LocalScheduler::new(
                crate::governance::LocalSchedulerConfig::for_tests(),
            ),
            ledger: Arc::new(EnterpriseLedger::memory()),
            finalizers: Arc::new(FinalizationTracker::default()),
            draining: Arc::new(AtomicBool::new(false)),
            retention_previews: Arc::new(RetentionPreviewStore::default()),
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

    fn chat_body(stream: bool) -> Value {
        json!({
            "model": "mimo-v2.5-pro",
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
