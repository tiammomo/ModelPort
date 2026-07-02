use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{
        HeaderMap, HeaderValue,
        header::{HeaderName, SET_COOKIE},
    },
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tower::{ServiceBuilder, limit::ConcurrencyLimitLayer};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::warn;

use crate::{
    auth::{AuthStore, CreateUserInput, LoginInput, PublicUser, UpdateUserInput},
    config::{
        AppConfig, ConfigIssueSeverity, FidelityMode, MaxTokensField, ProviderConfig,
        ProviderProtocol, RuntimeConfig,
    },
    control::{
        ActivityInput, ClientIdentity, ControlStore, CreateApiKeyInput, ProviderCredentialRecord,
        ProviderModelOverrideRecord, ProviderOverrideRecord, UpdateApiKeyInput, UpsertQuotaInput,
        UpsertTeamInput, provider_credential_row, provider_credential_rows,
    },
    error::AppError,
    http::{Header, HttpTransport},
    metrics::Metrics,
};

mod admin_providers;
mod client_api;
mod ops;

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const CSRF_HEADER: HeaderName = HeaderName::from_static("x-modelport-csrf");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
const HOUR_MS: u64 = 60 * 60 * 1_000;
const DAY_MS: u64 = 24 * HOUR_MS;
const MAX_DASHBOARD_TREND_MS: u64 = 90 * DAY_MS;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RuntimeConfig>,
    pub auth: Arc<AuthStore>,
    pub control: Arc<ControlStore>,
    pub security: Arc<GatewaySecurityPolicy>,
    pub rate_limiter: Arc<RateLimiter>,
    pub trusted_proxies: Arc<TrustedProxyConfig>,
    pub transport: HttpTransport,
    pub metrics: Arc<Metrics>,
}

#[derive(Debug, Clone)]
pub struct GatewaySecurityPolicy {
    allow_legacy_client_auth: bool,
    expose_detailed_public_health: bool,
    allow_private_provider_urls: bool,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardQuery {
    range: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Clone)]
struct DashboardTrendWindow {
    range: String,
    start_ms: u64,
    end_ms: u64,
    bucket_ms: u64,
}

impl TrustedProxyConfig {
    pub fn from_env() -> Result<Self, AppError> {
        let mut rules = vec![
            IpRule::Exact(IpAddr::from([127, 0, 0, 1])),
            IpRule::Exact(IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])),
        ];

        if let Ok(value) = env::var("MODELPORT_TRUSTED_PROXIES") {
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

impl GatewaySecurityPolicy {
    pub fn from_env() -> Self {
        Self {
            allow_legacy_client_auth: !env_flag("MODELPORT_REQUIRE_CONTROL_API_KEYS"),
            expose_detailed_public_health: env_flag("MODELPORT_EXPOSE_DETAILED_HEALTH"),
            allow_private_provider_urls: env_flag("MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS"),
        }
    }

    #[cfg(test)]
    fn for_tests() -> Self {
        Self {
            allow_legacy_client_auth: true,
            expose_detailed_public_health: false,
            allow_private_provider_urls: true,
        }
    }

    #[cfg(test)]
    fn require_control_api_keys_for_tests() -> Self {
        Self {
            allow_legacy_client_auth: false,
            expose_detailed_public_health: false,
            allow_private_provider_urls: true,
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

        let now = now_millis();
        let window_start = now.saturating_sub(self.config.window_ms);
        let mut inner = self.inner.lock().expect("rate limiter lock poisoned");
        let mut rules = vec![
            (
                "global:messages".to_owned(),
                self.config.global_per_minute,
                "global request rate limit exceeded",
            ),
            (
                format!(
                    "api-key:{}",
                    scope
                        .identity
                        .api_key_id
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
        .route("/livez", get(ops::livez))
        .route("/readyz", get(ops::readyz))
        .route("/health", get(ops::health))
        .route("/metrics", get(ops::metrics))
        .route("/v1/models", get(client_api::models))
        .route("/v1/messages", post(client_api::messages))
        .route("/admin/auth/login", post(admin_login))
        .route("/admin/auth/logout", post(admin_logout))
        .route("/admin/auth/me", get(admin_me))
        .route("/admin/dashboard", get(admin_dashboard))
        .route(
            "/admin/providers",
            get(admin_providers::admin_providers).post(admin_providers::admin_create_provider),
        )
        .route(
            "/admin/providers/{provider_id}",
            put(admin_providers::admin_update_provider)
                .delete(admin_providers::admin_delete_provider),
        )
        .route(
            "/admin/providers/{provider_id}/disable",
            post(admin_providers::admin_set_provider_disabled),
        )
        .route(
            "/admin/providers/{provider_id}/models",
            get(admin_providers::admin_provider_models)
                .post(admin_providers::admin_provider_models)
                .put(admin_providers::admin_upsert_provider_model)
                .delete(admin_providers::admin_delete_provider_model),
        )
        .route(
            "/admin/providers/{provider_id}/credentials",
            post(admin_providers::admin_create_provider_credential),
        )
        .route(
            "/admin/providers/{provider_id}/credential-pool",
            put(admin_providers::admin_set_provider_credential_pool_mode),
        )
        .route(
            "/admin/providers/{provider_id}/credentials/{credential_id}",
            put(admin_providers::admin_update_provider_credential)
                .delete(admin_providers::admin_delete_provider_credential),
        )
        .route(
            "/admin/providers/{provider_id}/credentials/{credential_id}/select",
            post(admin_providers::admin_select_provider_credential),
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
        .route("/admin/settings/reload-config", post(admin_reload_config))
        .route("/admin/settings/test-provider", post(admin_test_provider))
        .route("/admin/audit", get(admin_audit))
        .route("/admin/backup", get(admin_backup))
        .route("/admin/logs", get(admin_logs))
        .route("/admin/latency", get(admin_latency))
        .route("/admin/teams", get(admin_teams).post(admin_upsert_team))
        .route(
            "/admin/teams/{team_id}",
            put(admin_update_team).delete(admin_delete_team),
        )
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
    response
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
    if !state.security.allow_legacy_client_auth {
        return Err(AppError::Forbidden(
            "control-plane API key is required; legacy router token auth is disabled".to_owned(),
        ));
    }
    state.config.snapshot().validate_client_auth(headers)?;
    Ok(ControlStore::legacy_identity())
}

fn client_ip(
    headers: &HeaderMap,
    peer_addr: Option<SocketAddr>,
    trusted_proxies: &TrustedProxyConfig,
) -> Option<String> {
    if peer_addr.is_some_and(|peer| trusted_proxies.is_trusted(peer.ip()))
        && let Some(ip) = forwarded_client_ip(headers)
    {
        return Some(ip.to_string());
    }

    peer_addr.map(|peer| peer.ip().to_string())
}

fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    for name in ["x-forwarded-for", "x-real-ip", "cf-connecting-ip"] {
        let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) else {
            continue;
        };
        let candidate = value.split(',').next().unwrap_or(value).trim();
        if let Some(ip) = parse_ip_with_optional_port(candidate) {
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
    require_console_user(&state, &headers)?;
    let trend_window = dashboard_trend_window(&query)?;

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
    let (usage_request_series, usage_error_series) = state.control.usage_time_series(
        trend_window.start_ms,
        trend_window.end_ms,
        trend_window.bucket_ms,
    );
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
    let config = effective_config(&state);
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
        "totalModels": config.model_list().len(),
        "avgLatencyMs": avg_latency_ms,
        "apiKeysTotal": usage_summary.api_keys_total,
        "apiKeysActive": usage_summary.api_keys_active,
        "todayRequests": usage_summary.total_requests,
        "todayInputTokens": usage_summary.total_input_tokens,
        "todayOutputTokens": usage_summary.total_output_tokens,
        "todayCacheWriteTokens": usage_summary.total_cache_write_tokens,
        "todayCacheReadTokens": usage_summary.total_cache_read_tokens,
        "todayCostEstimate": usage_summary.total_cost_estimate,
        "trendRange": {
            "range": trend_window.range,
            "from": trend_window.start_ms.to_string(),
            "to": trend_window.end_ms.to_string(),
            "bucketMs": trend_window.bucket_ms,
        },
        "requestTimeSeries": if usage_series_has_data { usage_request_series } else { time_series(total_requests, &trend_window) },
        "errorTimeSeries": if usage_series_has_data { usage_error_series } else { time_series(total_failures, &trend_window) },
        "topModels": if metric_top_models.is_empty() { usage_top_models } else { metric_top_models },
        "providerHealth": providers.iter().map(|provider| {
            let id = provider.get("id").and_then(Value::as_str).unwrap_or("");
            let provider_messages = snapshot.messages.iter().filter(|message| message.provider == id).collect::<Vec<_>>();
            let metric_requests = provider_messages.iter().map(|message| message.requests_total).sum::<u64>();
            let metric_successes = provider_messages.iter().map(|message| message.successes_total).sum::<u64>();
            let metric_duration = provider_messages.iter().map(|message| message.duration_ms_total).sum::<u64>();
            let persisted = persisted_provider_usage.get(id);
            let has_persisted_usage = persisted.is_some_and(|stats| stats.requests_total > 0);
            let requests = if has_persisted_usage { persisted.map(|stats| stats.requests_total).unwrap_or(0) } else { metric_requests };
            let successes = if has_persisted_usage { persisted.map(|stats| stats.successes_total).unwrap_or(0) } else { metric_successes };
            let duration = if has_persisted_usage { persisted.map(|stats| stats.duration_ms_total).unwrap_or(0) } else { metric_duration };
            let input_tokens = if has_persisted_usage {
                persisted.map(|stats| stats.input_tokens_total).unwrap_or(0)
            } else {
                provider_messages.iter().map(|message| message.input_tokens_total).sum::<u64>()
            };
            let output_tokens = if has_persisted_usage {
                persisted.map(|stats| stats.output_tokens_total).unwrap_or(0)
            } else {
                provider_messages.iter().map(|message| message.output_tokens_total).sum::<u64>()
            };
            let cache_write_tokens = if has_persisted_usage {
                persisted.map(|stats| stats.cache_write_tokens_total).unwrap_or(0)
            } else {
                provider_messages.iter().map(|message| message.cache_write_tokens_total).sum::<u64>()
            };
            let cache_read_tokens = if has_persisted_usage {
                persisted.map(|stats| stats.cache_read_tokens_total).unwrap_or(0)
            } else {
                provider_messages.iter().map(|message| message.cache_read_tokens_total).sum::<u64>()
            };
            let cost_estimate = if has_persisted_usage {
                persisted.map(|stats| stats.cost_estimate_usd_total).unwrap_or(0.0)
            } else {
                provider_messages.iter().map(|message| message.cost_estimate_usd_total).sum::<f64>()
            };
            let success_rate = percent(successes, requests);
            let provider_status = provider.get("status").and_then(Value::as_str).unwrap_or("inactive");
            let runtime_status = provider.get("runtimeStatus").and_then(Value::as_str).unwrap_or("healthy");
            let provider_health = provider.get("health").unwrap_or(&Value::Null);
            let health_status = if provider_status != "active" {
                "down"
            } else if runtime_status == "cooldown" {
                "cooldown"
            } else if runtime_status == "degraded" || (requests > 0 && success_rate < 99.0) {
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
                "inputTokensTotal": input_tokens,
                "outputTokensTotal": output_tokens,
                "cacheWriteTokensTotal": cache_write_tokens,
                "cacheReadTokensTotal": cache_read_tokens,
                "costEstimateUsdTotal": cost_estimate,
                "accountIssue": provider_health.get("accountIssue").cloned().unwrap_or_else(|| json!("none")),
                "rechargeRequired": provider_health.get("rechargeRequired").and_then(Value::as_bool).unwrap_or(false),
                "rechargeBadge": provider_health.get("rechargeBadge").cloned().unwrap_or(Value::Null),
            })
        }).collect::<Vec<_>>(),
        "recentActivity": if recent_activity.is_empty() { fallback_activity } else { recent_activity },
    })))
}

async fn admin_aliases(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_console_user(&state, &headers)?;
    Ok(Json(Value::Array(alias_rows(&state))))
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
    );
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
    );
    Ok(Json(json!({ "ok": true })))
}

async fn admin_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_console_user(&state, &headers)?;
    Ok(Json(settings_row(&state)))
}

async fn admin_reload_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let config = state.config.reload()?;
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
    );

    Ok(Json(json!({
        "ok": true,
        "settings": settings_row(&state),
        "providerCount": config.providers.len(),
        "defaultProvider": config.default_provider,
        "providerOrder": config.provider_order,
        "issues": issues,
        "reloadScope": {
            "applied": ["providers", "provider credentials", "base urls", "model lists", "aliases", "legacy client auth token"],
            "requiresRestart": ["bind address", "request body limit", "concurrency layer", "HTTP client timeouts", "trusted proxies", "admin bootstrap account"],
        },
    })))
}

async fn admin_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
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
        );
    }

    Ok(Json(settings_row(&state)))
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

async fn admin_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_console_user(&state, &headers)?;
    let events = state.control.activity_rows(100);
    Ok(Json(json!({
        "events": events,
        "total": state.control.activity_count(),
    })))
}

async fn admin_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_user(&state, &headers)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        "backup",
        "导出控制面备份",
        "info",
    );

    Ok(Json(json!({
        "schemaVersion": 1,
        "service": "model-port",
        "generatedAt": now_millis_string(),
        "containsSecrets": false,
        "containsPersonalData": true,
        "settings": settings_row(&state),
        "users": state.auth.list_users(0),
        "control": state.control.export_snapshot(),
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
    require_console_user(&state, &headers)?;
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
    require_console_user(&state, &headers)?;
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

async fn admin_teams(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_console_user(&state, &headers)?;
    Ok(Json(json!(state.control.list_teams())))
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
    );
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
    );
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
    );
    Ok(Json(json!({ "ok": true })))
}

async fn admin_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    let requests = state
        .metrics
        .snapshot()
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    let mut users = state.auth.list_users(requests);
    if actor.role == "user" {
        users.retain(|user| user.id == actor.id);
    }
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
    let current_user = require_admin_write_user(&state, &headers)?;
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
    let current_user = require_admin_write_user(&state, &headers)?;
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
    let actor = require_console_user(&state, &headers)?;
    if actor.role == "user" {
        Ok(Json(json!(state.control.list_user_api_keys(&actor.id))))
    } else {
        Ok(Json(json!(state.control.list_api_keys())))
    }
}

async fn admin_user_api_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    if actor.role == "user" && actor.id != user_id {
        return Err(AppError::Forbidden(
            "cannot read another user's API keys".to_owned(),
        ));
    }
    Ok(Json(json!(state.control.list_user_api_keys(&user_id))))
}

async fn admin_create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<CreateApiKeyInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_api_key_write_user(&state, &headers)?;
    if actor.role != "admin" {
        if body.user_id != actor.id {
            return Err(AppError::Forbidden(
                "cannot create API keys for another user".to_owned(),
            ));
        }
        body.username = Some(actor.username.clone());
        body.team_id = None;
    }
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
    let actor = require_api_key_write_user(&state, &headers)?;
    ensure_api_key_access(&state, &actor, &key_id)?;
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
    let actor = require_api_key_write_user(&state, &headers)?;
    ensure_api_key_access(&state, &actor, &key_id)?;
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
    let actor = require_api_key_write_user(&state, &headers)?;
    ensure_api_key_access(&state, &actor, &key_id)?;
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
    require_console_user(&state, &headers)?;
    Ok(Json(json!(state.control.list_quotas()?)))
}

async fn admin_create_quota(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpsertQuotaInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
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
    let actor = require_admin_write_user(&state, &headers)?;
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
    let actor = require_admin_write_user(&state, &headers)?;
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
    let config = management_config(state);
    let controls = state.control.provider_control_snapshot();
    let provider_tests = state.control.provider_test_rows();
    let provider_health = state.control.provider_health_rows();
    let credential_health = state.control.provider_credential_health_rows();
    config
        .provider_order
        .iter()
        .filter_map(|id| {
            let provider = config.providers.get(id)?;
            let has_api_key = provider.api_key().ok().flatten().is_some();
            let health = provider_health.get(id).cloned();
            let active_credential_id = controls
                .active_provider_credentials
                .get(id)
                .map(String::as_str);
            let credential_pool_mode = controls
                .provider_credential_pool_modes
                .get(id)
                .map(String::as_str)
                .unwrap_or("failover");
            let credentials = provider_credential_rows(
                controls.provider_credentials.get(id),
                active_credential_id,
                credential_health.get(id),
            );
            let runtime_status = health
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("healthy");
            let config_status = if has_api_key || !provider.api_key_required {
                "active"
            } else {
                "inactive"
            };
            let status = if controls.disabled_providers.contains(id) {
                "disabled"
            } else {
                config_status
            };
            Some(json!({
                "id": id,
                "displayName": provider.display_name,
                "source": if controls.provider_overrides.contains_key(id) { "control" } else { "config" },
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
                "status": status,
                "runtimeStatus": runtime_status,
                "hasApiKey": has_api_key,
                "credentials": credentials,
                "activeCredentialId": active_credential_id,
                "credentialPoolMode": credential_pool_mode,
                "lastTest": provider_tests.get(id).cloned(),
                "health": health,
                "modelInventory": provider_inventory_rows(id, provider, &controls),
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
        "runtime": runtime_row(state, &config),
        "setup": setup_row(state, &config),
    })
}

fn runtime_row(state: &AppState, config: &AppConfig) -> Value {
    let base_url = local_base_url(config);
    json!({
        "apiEndpoint": format!("{base_url}/v1/messages"),
        "modelsEndpoint": format!("{base_url}/v1/models"),
        "adminEndpoint": format!("{base_url}/admin"),
        "controlDataPath": state.control.data_path(),
        "authDataPath": state.auth.data_path(),
    })
}

fn setup_row(state: &AppState, config: &AppConfig) -> Value {
    let providers = provider_rows(state);
    let active_provider_count = providers
        .iter()
        .filter(|provider| provider.get("status").and_then(Value::as_str) == Some("active"))
        .count();
    let default_provider = providers.iter().find(|provider| {
        provider
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == config.default_provider)
    });
    let default_provider_active = default_provider
        .and_then(|provider| provider.get("status").and_then(Value::as_str))
        == Some("active");
    let validation_issues = config_issues_json(config);
    let validation_errors = validation_issues
        .iter()
        .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("error"))
        .count();
    let validation_warnings = validation_issues
        .iter()
        .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
        .count();
    let checks = vec![
        setup_check(
            "admin",
            "管理员账号",
            state.auth.active_admin_count() > 0,
            "至少一个活跃管理员",
            "没有活跃管理员",
        ),
        setup_check(
            "auth",
            "API 认证",
            config.auth_token.is_some(),
            "已启用请求认证",
            "未配置 MODELPORT_AUTH_TOKEN",
        ),
        setup_check(
            "providers",
            "供应商凭证",
            active_provider_count > 0,
            format!("{active_provider_count} 个供应商可用"),
            "没有可用供应商",
        ),
        setup_check(
            "defaultProvider",
            "默认供应商",
            default_provider_active,
            format!("{} 可用", config.default_provider),
            format!("{} 不可用", config.default_provider),
        ),
        setup_check(
            "persistence",
            "控制面数据",
            state.control.data_path().is_some() && state.auth.data_path().is_some(),
            "已启用本地持久化",
            "当前运行未配置数据文件",
        ),
        setup_check(
            "config",
            "配置校验",
            validation_errors == 0,
            if validation_warnings == 0 {
                "无配置告警".to_owned()
            } else {
                format!("{validation_warnings} 条配置告警")
            },
            format!("{validation_errors} 条配置错误"),
        ),
    ];
    let ready = checks
        .iter()
        .all(|check| check.get("status").and_then(Value::as_str) != Some("error"));

    json!({
        "ready": ready,
        "activeProviderCount": active_provider_count,
        "defaultProviderReady": default_provider_active,
        "checks": checks,
        "issues": validation_issues,
    })
}

fn config_issues_json(config: &AppConfig) -> Vec<Value> {
    config
        .validation_issues()
        .into_iter()
        .map(|issue| {
            json!({
                "severity": match issue.severity {
                    ConfigIssueSeverity::Error => "error",
                    ConfigIssueSeverity::Warning => "warning",
                },
                "message": issue.message,
            })
        })
        .collect()
}

fn setup_check(
    id: &str,
    label: &str,
    ok: bool,
    ok_detail: impl Into<String>,
    error_detail: impl Into<String>,
) -> Value {
    json!({
        "id": id,
        "label": label,
        "status": if ok { "ok" } else { "error" },
        "detail": if ok { ok_detail.into() } else { error_detail.into() },
    })
}

fn local_base_url(config: &AppConfig) -> String {
    let ip = if config.bind_addr.ip().is_unspecified() {
        match config.bind_addr.ip() {
            IpAddr::V4(_) => "127.0.0.1".to_owned(),
            IpAddr::V6(_) => "[::1]".to_owned(),
        }
    } else {
        match config.bind_addr.ip() {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        }
    };
    format!("http://{ip}:{}", config.bind_addr.port())
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
        let Ok(provider) = provider_record_to_config(record) else {
            continue;
        };
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
        let mut seen = provider.models.iter().cloned().collect::<BTreeSet<_>>();
        for model in models {
            if seen.insert(model.clone()) {
                provider.models.push(model);
            }
        }
    }

    apply_provider_credentials(&mut config, &controls);
    apply_provider_model_overrides(&mut config, &controls.provider_model_overrides);
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

fn provider_row_by_id(state: &AppState, provider_id: &str) -> Result<Value, AppError> {
    provider_rows(state)
        .into_iter()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(provider_id))
        .ok_or_else(|| AppError::ProviderNotFound(provider_id.to_owned()))
}

fn provider_inventory_rows(
    provider_id: &str,
    provider: &ProviderConfig,
    controls: &crate::control::ProviderControlSnapshot,
) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    let overrides = controls.provider_model_overrides.get(provider_id);
    for model in &provider.models {
        seen.insert(model.clone());
        let override_record = overrides.and_then(|models| models.get(model));
        rows.push(json!({
            "model": model,
            "status": override_record.map(|record| record.status.as_str()).unwrap_or("active"),
            "displayName": override_record.and_then(|record| record.display_name.as_deref()),
            "family": override_record.and_then(|record| record.family.as_deref()),
            "contextWindow": override_record.and_then(|record| record.context_window),
            "default": model == &provider.default_model,
        }));
    }
    if let Some(overrides) = overrides {
        for record in overrides.values() {
            if seen.insert(record.model.clone()) {
                rows.push(provider_model_row(record));
            }
        }
    }
    rows
}

fn provider_model_row(record: &ProviderModelOverrideRecord) -> Value {
    json!({
        "providerId": record.provider_id,
        "model": record.model,
        "status": record.status,
        "displayName": record.display_name,
        "family": record.family,
        "contextWindow": record.context_window,
        "createdAt": record.created_at_ms.to_string(),
        "updatedAt": record.updated_at_ms.to_string(),
    })
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

fn log_rows(state: &AppState) -> Vec<Value> {
    let config = effective_config(state);
    state
        .metrics
        .snapshot()
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let protocol = config
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

fn dashboard_trend_window(query: &DashboardQuery) -> Result<DashboardTrendWindow, AppError> {
    let now = now_millis();
    let range = query.range.as_deref().unwrap_or("1d");
    let (range, start_ms, end_ms) = match range {
        "custom" => {
            let start_ms = query
                .from
                .as_deref()
                .and_then(parse_dashboard_time)
                .ok_or_else(|| {
                    AppError::InvalidRequest("custom dashboard range requires from".to_owned())
                })?;
            let end_ms = query
                .to
                .as_deref()
                .and_then(parse_dashboard_time)
                .ok_or_else(|| {
                    AppError::InvalidRequest("custom dashboard range requires to".to_owned())
                })?;
            if start_ms >= end_ms {
                return Err(AppError::InvalidRequest(
                    "custom dashboard range requires from before to".to_owned(),
                ));
            }
            ("custom".to_owned(), start_ms, end_ms.min(now))
        }
        "3d" => ("3d".to_owned(), now.saturating_sub(3 * DAY_MS), now),
        "7d" => ("7d".to_owned(), now.saturating_sub(7 * DAY_MS), now),
        _ => ("1d".to_owned(), now.saturating_sub(DAY_MS), now),
    };
    let duration_ms = end_ms.saturating_sub(start_ms).max(HOUR_MS);
    if duration_ms > MAX_DASHBOARD_TREND_MS {
        return Err(AppError::InvalidRequest(
            "dashboard range cannot exceed 90 days".to_owned(),
        ));
    }

    Ok(DashboardTrendWindow {
        range,
        start_ms,
        end_ms,
        bucket_ms: dashboard_bucket_ms(duration_ms),
    })
}

fn parse_dashboard_time(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

fn dashboard_bucket_ms(duration_ms: u64) -> u64 {
    if duration_ms <= DAY_MS {
        HOUR_MS
    } else if duration_ms <= 3 * DAY_MS {
        3 * HOUR_MS
    } else if duration_ms <= 7 * DAY_MS {
        6 * HOUR_MS
    } else if duration_ms <= 31 * DAY_MS {
        DAY_MS
    } else {
        7 * DAY_MS
    }
}

fn time_series(value: u64, window: &DashboardTrendWindow) -> Vec<Value> {
    let bucket_count = bucket_count(window.start_ms, window.end_ms, window.bucket_ms);
    (0..bucket_count)
        .map(|offset| {
            let timestamp = window
                .start_ms
                .saturating_add(offset.saturating_mul(window.bucket_ms));
            json!({
                "timestamp": timestamp.to_string(),
                "value": if offset + 1 == bucket_count { value } else { 0 },
            })
        })
        .collect()
}

fn bucket_count(start_ms: u64, end_ms: u64, bucket_ms: u64) -> u64 {
    if bucket_ms == 0 || end_ms <= start_ms {
        return 1;
    }
    end_ms.saturating_sub(start_ms) / bucket_ms + 1
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
    use std::{collections::HashMap, sync::Arc};

    use axum::{
        body::{Body, to_bytes},
        extract::connect_info::ConnectInfo,
        http::{
            Request, StatusCode,
            header::{CONTENT_TYPE, COOKIE, HOST, HeaderValue, ORIGIN, SET_COOKIE},
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
        };

        let rows = client_api::public_model_rows(&config);
        let ids = rows
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"mimo-v2.5-pro"));
        assert!(ids.contains(&"fast-chat"));
        assert!(!ids.contains(&"deepseek-v4-flash"));
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
        };

        let rows = client_api::public_model_rows(&config);
        let display_name = |id: &str| {
            rows.iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .and_then(|row| row.get("display_name").and_then(Value::as_str))
                .unwrap()
        };

        assert_eq!(display_name("gpt-5.2"), "第三方 · OpenAI");
        assert_eq!(display_name("mimo-v2.5-pro"), "第三方 · 小米 MiMo");
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
    async fn control_api_key_mode_rejects_legacy_token_but_accepts_api_key_records() {
        let upstream = spawn_openai_upstream(
            StatusCode::OK,
            r#"{"id":"ok","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
            "application/json",
        )
        .await;
        let mut state = test_state(upstream, 1024 * 1024);
        state.security = Arc::new(GatewaySecurityPolicy::require_control_api_keys_for_tests());
        let created = state
            .control
            .create_api_key(CreateApiKeyInput {
                user_id: "usr_test".to_owned(),
                username: Some("test-user".to_owned()),
                name: "Claude Code".to_owned(),
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
    async fn viewer_can_read_dashboard_but_not_create_users() {
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
        assert_eq!(dashboard_response.status(), StatusCode::OK);

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
            r#"modelport_message_requests_total{provider="mimo",model="mimo-v2.5-pro",stream="false"} 1"#
        ));
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

        let models_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
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
        assert!(ids.contains(&"mimo-v2.6-pro"));
    }

    #[test]
    fn discovered_provider_models_are_runtime_routable() {
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
        assert!(provider.models.contains(&"mimo-v2.6-pro".to_owned()));

        let rows = client_api::public_model_rows(&config);
        let ids = rows
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(ids.contains(&"mimo-v2.6-pro"));
        assert_eq!(config.resolve("mimo-v2.6-pro").unwrap().provider_id, "mimo");
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
                base_url: Some("http://account-a.local/v1".to_owned()),
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
                base_url: Some("http://account-b.local/v1".to_owned()),
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
        assert_eq!(provider.base_url, "http://account-b.local/v1");

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
    fn provider_override_is_routable_until_disabled() {
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
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();

        let config = effective_config(&state);
        assert_eq!(
            config.resolve("qwen-local").unwrap().provider_id,
            "local_custom"
        );
        assert!(
            client_api::public_model_rows(&config)
                .iter()
                .any(|row| row.get("id").and_then(Value::as_str) == Some("qwen-local"))
        );

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
            management_config(&state)
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
        post_message_with_key(app, CLIENT_TOKEN, body).await
    }

    async fn post_message_with_key(app: Router, key: &str, body: Value) -> (StatusCode, String) {
        let response = post_message_response(app, key, body).await;

        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn post_message_response(app: Router, key: &str, body: Value) -> Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .extension(ConnectInfo(
                    "127.0.0.1:48178".parse::<SocketAddr>().unwrap(),
                ))
                .header("x-api-key", key)
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

    async fn spawn_openai_models_upstream(body: &'static str) -> String {
        let app = Router::new().route(
            "/v1/models",
            get(
                move || async move { (StatusCode::OK, [(CONTENT_TYPE, "application/json")], body) },
            ),
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
            config: Arc::new(RuntimeConfig::new(AppConfig {
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                max_request_body_bytes,
                max_concurrent_requests: 16,
                auth_token: Some(CLIENT_TOKEN.to_owned()),
                default_provider: "mimo".to_owned(),
                provider_order: vec!["mimo".to_owned()],
                providers: HashMap::from([("mimo".to_owned(), provider)]),
                aliases: HashMap::new(),
            })),
            auth: Arc::new(AuthStore::for_tests()),
            control: Arc::new(ControlStore::for_tests()),
            security: Arc::new(GatewaySecurityPolicy::for_tests()),
            rate_limiter: Arc::new(RateLimiter::disabled()),
            trusted_proxies: Arc::new(TrustedProxyConfig::for_tests()),
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
