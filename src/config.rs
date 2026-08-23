use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env, fmt, fs,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use axum::http::HeaderMap;
use reqwest::Url;
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    model_catalog::{
        ModelProfile, ModelProfileOverride, ReasoningEffort, resolve_model_profile,
        validate_model_profile_override,
    },
    pricing::{ModelPricing, ModelPricingCard, PricingServiceTier, PricingSource},
    runtime_adapter::RuntimeAdapterClientConfig,
};

const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 64;
const MAX_PROVIDER_TIMER_MS: u64 = 2_147_483_647;
const MAX_RUNTIME_ADAPTERS: usize = 64;
const DEFAULT_RUNTIME_ADAPTER_POLL_INTERVAL_SECONDS: u64 = 30;
const DEFAULT_RUNTIME_ADAPTER_STALE_AFTER_SECONDS: u64 = 90;
const MIN_RUNTIME_ADAPTER_INTERVAL_SECONDS: u64 = 5;
const MAX_RUNTIME_ADAPTER_POLL_INTERVAL_SECONDS: u64 = 3_600;
const MAX_RUNTIME_ADAPTER_STALE_AFTER_SECONDS: u64 = 86_400;

#[derive(Clone)]
pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub max_request_body_bytes: usize,
    pub max_concurrent_requests: usize,
    pub auth_token: Option<String>,
    pub default_provider: String,
    pub provider_order: Vec<String>,
    pub providers: HashMap<String, ProviderConfig>,
    pub aliases: HashMap<String, String>,
    pub smart_routing: SmartRoutingConfig,
    pub runtime_adapters: BTreeMap<String, RuntimeAdapterConfig>,
}

#[derive(Clone)]
pub struct RuntimeAdapterConfig {
    pub client_config: RuntimeAdapterClientConfig,
    pub credential_env: String,
    pub poll_interval: Duration,
    pub stale_after: Duration,
}

impl fmt::Debug for RuntimeAdapterConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeAdapterConfig")
            .field("client_config", &self.client_config)
            .field("credential_env", &self.credential_env)
            .field("poll_interval", &self.poll_interval)
            .field("stale_after", &self.stale_after)
            .finish()
    }
}

pub struct RuntimeConfig {
    inner: RwLock<AppConfig>,
    loader: Arc<dyn Fn() -> Result<AppConfig, AppError> + Send + Sync>,
}

#[derive(Clone)]
pub struct ProviderConfig {
    pub display_name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub api_key_env: Option<String>,
    pub api_key: Option<String>,
    pub api_key_required: bool,
    pub default_model: String,
    pub models: Vec<String>,
    pub model_prefixes: Vec<String>,
    pub passthrough_unknown_models: bool,
    pub max_tokens_field: MaxTokensField,
    pub deduplicate_stream_text: bool,
    pub buffer_stream_text: bool,
    pub fidelity_mode: FidelityMode,
    pub tool_use: ToolUseConfig,
    pub model_profile_defaults: ModelProfileOverride,
    pub model_profiles: HashMap<String, ModelProfileOverride>,
    pub reasoning: ReasoningConfig,
    pub sampling: SamplingConfig,
    pub token_counting: TokenCountingConfig,
    pub static_headers: BTreeMap<String, String>,
    pub request_timeout_ms: Option<u64>,
    pub stream_idle_timeout_ms: Option<u64>,
    pub retry: ProviderRetryConfig,
    /// Legacy Provider-wide estimate-only pricing.
    pub pricing: Option<ModelPricing>,
    /// Exact-model, evidence-bearing rate cards eligible for settlement.
    pub model_pricing: HashMap<String, ModelPricingCard>,
    /// Trust a non-negative `usage.cost` value returned by this Provider as USD.
    pub trust_upstream_cost: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderProtocol {
    Anthropic,
    OpenaiCompat,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SmartRoutingMode {
    #[default]
    Off,
    Shadow,
    Active,
}

impl SmartRoutingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Shadow => "shadow",
            Self::Active => "active",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingProfile {
    Quality,
    #[default]
    Balanced,
    Economy,
    Latency,
}

impl RoutingProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Quality => "quality",
            Self::Balanced => "balanced",
            Self::Economy => "economy",
            Self::Latency => "latency",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "quality" => Some(Self::Quality),
            "balanced" => Some(Self::Balanced),
            "economy" => Some(Self::Economy),
            "latency" => Some(Self::Latency),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmartRoutingConfig {
    #[serde(default)]
    pub mode: SmartRoutingMode,
    #[serde(default)]
    pub default_profile: RoutingProfile,
    #[serde(default = "default_routing_policy_version")]
    pub policy_version: String,
    #[serde(default = "default_routing_activation_percent")]
    pub activation_percent: u8,
    #[serde(default)]
    pub groups: HashMap<String, RouteGroupConfig>,
}

impl Default for SmartRoutingConfig {
    fn default() -> Self {
        Self {
            mode: SmartRoutingMode::Off,
            default_profile: RoutingProfile::Balanced,
            policy_version: default_routing_policy_version(),
            activation_percent: default_routing_activation_percent(),
            groups: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteGroupConfig {
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub default_profile: Option<RoutingProfile>,
    #[serde(default)]
    pub candidates: Vec<RouteCandidateConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteCandidateConfig {
    pub provider: String,
    pub model: String,
    #[serde(default = "default_route_quality")]
    pub quality: f64,
    #[serde(default = "default_route_latency_ms")]
    pub latency_hint_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_routing_policy_version() -> String {
    "builtin-v1".to_owned()
}

fn default_route_quality() -> f64 {
    0.8
}

fn default_route_latency_ms() -> u64 {
    1_000
}

fn default_routing_activation_percent() -> u8 {
    0
}

fn apply_smart_routing_env_override(config: &mut SmartRoutingConfig) -> Result<(), AppError> {
    if let Some(value) = env_value("MODELPORT_SMART_ROUTING_MODE") {
        config.mode = match value.trim().to_ascii_lowercase().as_str() {
            "off" => SmartRoutingMode::Off,
            "shadow" => SmartRoutingMode::Shadow,
            "active" => SmartRoutingMode::Active,
            _ => {
                return Err(AppError::Config(
                    "MODELPORT_SMART_ROUTING_MODE must be off, shadow, or active".to_owned(),
                ));
            }
        };
    }
    if let Some(value) = env_value("MODELPORT_SMART_ROUTING_PROFILE") {
        config.default_profile = match value.trim().to_ascii_lowercase().as_str() {
            "quality" => RoutingProfile::Quality,
            "balanced" => RoutingProfile::Balanced,
            "economy" => RoutingProfile::Economy,
            "latency" => RoutingProfile::Latency,
            _ => {
                return Err(AppError::Config(
                    "MODELPORT_SMART_ROUTING_PROFILE must be quality, balanced, economy, or latency"
                        .to_owned(),
                ));
            }
        };
    }
    if let Some(value) = env_value("MODELPORT_SMART_ROUTING_ACTIVATION_PERCENT") {
        config.activation_percent = value.parse::<u8>().map_err(|_| {
            AppError::Config(
                "MODELPORT_SMART_ROUTING_ACTIVATION_PERCENT must be an integer from 0 to 100"
                    .to_owned(),
            )
        })?;
        if config.activation_percent > 100 {
            return Err(AppError::Config(
                "MODELPORT_SMART_ROUTING_ACTIVATION_PERCENT must be from 0 to 100".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_smart_routing(config: &AppConfig, issues: &mut Vec<ConfigIssue>) {
    let routing = &config.smart_routing;
    if routing.groups.len() > 128 {
        issues.push(ConfigIssue::error(
            "smart routing supports at most 128 routing groups",
        ));
    }
    if routing.policy_version.trim().is_empty()
        || routing.policy_version.len() > 64
        || routing.policy_version.chars().any(char::is_control)
    {
        issues.push(ConfigIssue::error(
            "routing.policy_version must contain 1-64 non-control bytes",
        ));
    }
    if routing.mode != SmartRoutingMode::Off && routing.groups.is_empty() {
        issues.push(ConfigIssue::error(
            "smart routing shadow/active mode requires at least one routing group",
        ));
    }
    if routing.activation_percent > 100 {
        issues.push(ConfigIssue::error(
            "routing.activation_percent must be from 0 to 100",
        ));
    }
    if routing.mode == SmartRoutingMode::Active && routing.activation_percent == 0 {
        issues.push(ConfigIssue::warning(
            "smart routing active mode has activation_percent=0; requests remain on the configured baseline order",
        ));
    }
    let mut aliases = BTreeSet::new();
    let mut total_aliases = 0usize;
    let mut total_candidates = 0usize;
    for (group_id, group) in &routing.groups {
        if group_id.trim().is_empty()
            || group_id.len() > 80
            || group_id.chars().any(char::is_control)
        {
            issues.push(ConfigIssue::error(
                "routing group IDs must contain 1-80 non-control bytes",
            ));
        }
        if group.aliases.is_empty() {
            issues.push(ConfigIssue::error(format!(
                "routing group `{group_id}` requires at least one alias"
            )));
        }
        if group.aliases.len() > 64 {
            issues.push(ConfigIssue::error(format!(
                "routing group `{group_id}` supports at most 64 aliases"
            )));
        }
        total_aliases = total_aliases.saturating_add(group.aliases.len());
        for alias in &group.aliases {
            if alias.trim().is_empty()
                || alias.len() > 160
                || alias.contains(':')
                || alias.chars().any(char::is_control)
            {
                issues.push(ConfigIssue::error(format!(
                    "routing group `{group_id}` contains an invalid alias"
                )));
            } else if !aliases.insert(alias.clone()) {
                issues.push(ConfigIssue::error(format!(
                    "smart routing alias `{alias}` is assigned more than once"
                )));
            } else if config.aliases.contains_key(alias) {
                issues.push(ConfigIssue::error(format!(
                    "smart routing alias `{alias}` conflicts with a static model alias"
                )));
            } else if config
                .providers
                .values()
                .any(|provider| provider.models.iter().any(|model| model == alias))
            {
                issues.push(ConfigIssue::error(format!(
                    "smart routing alias `{alias}` conflicts with a configured Provider model"
                )));
            }
        }

        let mut candidates = BTreeSet::new();
        let mut enabled_candidates = 0usize;
        total_candidates = total_candidates.saturating_add(group.candidates.len());
        if group.candidates.len() > 256 {
            issues.push(ConfigIssue::error(format!(
                "routing group `{group_id}` supports at most 256 candidates"
            )));
        }
        for candidate in &group.candidates {
            if !candidates.insert((candidate.provider.clone(), candidate.model.clone())) {
                issues.push(ConfigIssue::error(format!(
                    "routing group `{group_id}` repeats candidate `{}:{}`",
                    candidate.provider, candidate.model
                )));
            }
            let Some(provider) = config.providers.get(&candidate.provider) else {
                issues.push(ConfigIssue::error(format!(
                    "routing group `{group_id}` references missing provider `{}`",
                    candidate.provider
                )));
                continue;
            };
            if candidate.model.trim().is_empty() || candidate.model.len() > 240 {
                issues.push(ConfigIssue::error(format!(
                    "routing group `{group_id}` contains an invalid model"
                )));
            } else if !provider
                .models
                .iter()
                .any(|model| model == &candidate.model)
                && !provider
                    .model_prefixes
                    .iter()
                    .any(|prefix| candidate.model.starts_with(prefix))
                && !provider.passthrough_unknown_models
            {
                issues.push(ConfigIssue::error(format!(
                    "routing candidate `{}:{}` is not accepted by its provider",
                    candidate.provider, candidate.model
                )));
            }
            if !candidate.quality.is_finite() || !(0.0..=1.0).contains(&candidate.quality) {
                issues.push(ConfigIssue::error(format!(
                    "routing candidate `{}:{}` quality must be between 0 and 1",
                    candidate.provider, candidate.model
                )));
            }
            if !(1..=600_000).contains(&candidate.latency_hint_ms) {
                issues.push(ConfigIssue::error(format!(
                    "routing candidate `{}:{}` latency_hint_ms must be between 1 and 600000",
                    candidate.provider, candidate.model
                )));
            }
            enabled_candidates += usize::from(candidate.enabled);
        }
        if enabled_candidates == 0 {
            issues.push(ConfigIssue::error(format!(
                "routing group `{group_id}` requires at least one enabled candidate"
            )));
        }
    }
    if total_aliases > 1_024 {
        issues.push(ConfigIssue::error(
            "smart routing supports at most 1024 aliases in total",
        ));
    }
    if total_candidates > 1_024 {
        issues.push(ConfigIssue::error(
            "smart routing supports at most 1024 candidates in total",
        ));
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MaxTokensField {
    MaxCompletionTokens,
    MaxTokens,
    Both,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FidelityMode {
    Strict,
    BestEffort,
    Stability,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningMode {
    #[default]
    None,
    LlamaCpp,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReasoningConfig {
    #[serde(default)]
    pub mode: ReasoningMode,
    /// Provider policy used when the client protocol has no portable thinking
    /// control (notably OpenAI Chat Completions). `None` preserves the
    /// runtime's native default; an explicit boolean is rendered through the
    /// configured Provider extension.
    pub default_enabled: Option<bool>,
    /// Optional logical-model overrides. These let aliases such as `fast`,
    /// `code`, and `deep` share one runtime while carrying different default
    /// thinking policies. An explicit client control still wins.
    #[serde(default)]
    pub model_enabled: HashMap<String, bool>,
    pub default_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub model_effort: HashMap<String, ReasoningEffort>,
    pub default_budget_tokens: Option<u64>,
    #[serde(default)]
    pub model_budget_tokens: HashMap<String, u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ProviderRetryConfig {
    /// Total attempts for one provider candidate, including the first request.
    #[serde(default = "default_retry_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_retry_initial_delay_ms")]
    pub initial_delay_ms: u64,
    #[serde(default = "default_retry_max_delay_ms")]
    pub max_delay_ms: u64,
    #[serde(default = "default_retry_jitter_ratio")]
    pub jitter_ratio: f64,
}

impl Default for ProviderRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_retry_max_attempts(),
            initial_delay_ms: default_retry_initial_delay_ms(),
            max_delay_ms: default_retry_max_delay_ms(),
            jitter_ratio: default_retry_jitter_ratio(),
        }
    }
}

const fn default_retry_max_attempts() -> u32 {
    1
}

const fn default_retry_initial_delay_ms() -> u64 {
    250
}

const fn default_retry_max_delay_ms() -> u64 {
    5_000
}

const fn default_retry_jitter_ratio() -> f64 {
    0.1
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SamplingMode {
    #[default]
    None,
    LlamaCpp,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SamplingProfile {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<u64>,
    pub min_p: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub repeat_penalty: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SamplingConfig {
    #[serde(default)]
    pub mode: SamplingMode,
    #[serde(default)]
    pub profiles: HashMap<String, SamplingProfile>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountingMode {
    #[default]
    None,
    Anthropic,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TokenCountingConfig {
    #[serde(default)]
    pub mode: TokenCountingMode,
    pub context_tokens: Option<u64>,
    pub recommended_reasoning_input_tokens: Option<u64>,
    #[serde(default)]
    pub model_recommended_input_tokens: HashMap<String, u64>,
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub model_max_output_tokens: HashMap<String, u64>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolArgumentMode {
    Native,
    #[default]
    Delta,
    Cumulative,
    BestEffort,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResponseValidation {
    #[default]
    BestEffort,
    Strict,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseConfig {
    #[serde(default = "default_true", alias = "supported")]
    pub supported: bool,
    #[serde(default = "default_true", alias = "tool_choice")]
    pub tool_choice: bool,
    #[serde(default = "default_true", alias = "parallel_tool_calls")]
    pub parallel_tool_calls: bool,
    #[serde(default, alias = "streaming_arguments")]
    pub streaming_arguments: ToolArgumentMode,
    #[serde(default, alias = "response_validation")]
    pub response_validation: ToolResponseValidation,
    /// Retry one non-stream OpenAI-compatible request when strict schema
    /// validation rejects an upstream tool call. Disabled by default because
    /// this creates a second billable provider attempt.
    #[serde(default, alias = "repair_invalid_arguments")]
    pub repair_invalid_arguments: bool,
}

impl Default for ToolUseConfig {
    fn default() -> Self {
        Self {
            supported: true,
            tool_choice: true,
            parallel_tool_calls: true,
            streaming_arguments: ToolArgumentMode::Delta,
            response_validation: ToolResponseValidation::BestEffort,
            repair_invalid_arguments: false,
        }
    }
}

impl ToolUseConfig {
    pub fn default_for_provider(
        provider_id: &str,
        protocol: ProviderProtocol,
        deduplicate_stream_text: bool,
    ) -> Self {
        default_tool_use_config(provider_id, protocol, deduplicate_stream_text)
    }
}

#[derive(Clone)]
pub struct ResolvedProvider {
    pub provider_id: String,
    pub provider: ProviderConfig,
    pub model: String,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field("bind_addr", &self.bind_addr)
            .field("max_request_body_bytes", &self.max_request_body_bytes)
            .field("max_concurrent_requests", &self.max_concurrent_requests)
            .field("auth_enabled", &self.auth_token.is_some())
            .field("default_provider", &self.default_provider)
            .field("provider_order", &self.provider_order)
            .field("smart_routing_mode", &self.smart_routing.mode)
            .field(
                "smart_routing_groups",
                &self.smart_routing.groups.keys().collect::<Vec<_>>(),
            )
            .field("providers", &self.providers)
            .field("runtime_adapters", &self.runtime_adapters)
            .field("aliases", &self.aliases)
            .finish()
    }
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderConfig")
            .field("display_name", &self.display_name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("api_key_env", &self.api_key_env)
            .field("has_api_key", &self.api_key.is_some())
            .field("api_key_required", &self.api_key_required)
            .field("default_model", &self.default_model)
            .field("models", &self.models)
            .field("model_prefixes", &self.model_prefixes)
            .field(
                "passthrough_unknown_models",
                &self.passthrough_unknown_models,
            )
            .field("max_tokens_field", &self.max_tokens_field)
            .field("deduplicate_stream_text", &self.deduplicate_stream_text)
            .field("buffer_stream_text", &self.buffer_stream_text)
            .field("fidelity_mode", &self.fidelity_mode)
            .field("tool_use", &self.tool_use)
            .field("model_profile_defaults", &self.model_profile_defaults)
            .field("model_profiles", &self.model_profiles)
            .field("reasoning", &self.reasoning)
            .field("sampling", &self.sampling)
            .field("token_counting", &self.token_counting)
            .field(
                "static_header_names",
                &self.static_headers.keys().collect::<Vec<_>>(),
            )
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("stream_idle_timeout_ms", &self.stream_idle_timeout_ms)
            .field("retry", &self.retry)
            .field("pricing", &self.pricing)
            .field("model_pricing", &self.model_pricing)
            .field("trust_upstream_cost", &self.trust_upstream_cost)
            .finish()
    }
}

impl fmt::Debug for ResolvedProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedProvider")
            .field("provider_id", &self.provider_id)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigIssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy)]
enum NumericEnvRequirement {
    NonZeroU64,
    NonZeroUsize,
    U32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    pub severity: ConfigIssueSeverity,
    pub message: String,
}

pub fn validate_provider_base_url_for_request(
    provider_id: &str,
    base_url: &str,
    allow_private_provider_urls: bool,
) -> Result<(), AppError> {
    validate_provider_base_url_policy(provider_id, base_url, allow_private_provider_urls)
        .map_err(AppError::InvalidRequest)
}

/// A DNS answer set that has been checked against the Provider URL policy and
/// can be bound to the subsequent HTTP connection. Keeping the original DNS
/// name lets reqwest preserve the Host header and TLS SNI while preventing a
/// second, attacker-controlled resolution from changing the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderEndpointPin {
    pub(crate) dns_name: Option<String>,
    pub(crate) addresses: Vec<SocketAddr>,
}

/// Resolves a remote provider immediately before an outbound request and
/// rejects every answer set containing a non-public or special-use address.
/// Literal-only checks are insufficient for hostnames such as
/// attacker-controlled DNS records.
pub async fn validate_provider_base_url_dns_for_request(
    provider_id: &str,
    base_url: &str,
    allow_private_provider_urls: bool,
) -> Result<(), AppError> {
    resolve_provider_base_url_for_connection(provider_id, base_url, allow_private_provider_urls)
        .await
        .map(|_| ())
}

pub(crate) async fn resolve_provider_base_url_for_connection(
    provider_id: &str,
    base_url: &str,
    allow_private_provider_urls: bool,
) -> Result<ProviderEndpointPin, AppError> {
    validate_provider_base_url_for_request(provider_id, base_url, allow_private_provider_urls)?;

    let url = Url::parse(base_url)
        .map_err(|_| AppError::InvalidRequest("provider endpoint URL is invalid".to_owned()))?;
    let dns_host = url
        .host_str()
        .ok_or_else(|| AppError::InvalidRequest("provider endpoint host is missing".to_owned()))?
        .trim_matches(['[', ']']);
    let host = dns_host.trim_end_matches('.');
    let port = url.port_or_known_default().ok_or_else(|| {
        AppError::InvalidRequest("provider endpoint port could not be determined".to_owned())
    })?;
    let literal_ip = host.parse::<IpAddr>().ok();
    let mut addresses = if let Some(ip) = literal_ip {
        vec![SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| AppError::Transport("provider endpoint DNS resolution failed".to_owned()))?
            .collect::<Vec<_>>()
    };
    if addresses.is_empty() {
        return Err(AppError::Transport(
            "provider endpoint DNS returned no addresses".to_owned(),
        ));
    }
    addresses.sort_unstable();
    addresses.dedup();

    let private_addresses_allowed = allow_private_provider_urls
        || provider_allows_loopback_base_url(provider_id)
        || provider_allows_trusted_internal_http(provider_id, host);
    if !private_addresses_allowed {
        validate_provider_resolved_ips(addresses.iter().map(|address| address.ip()))?;
    }

    Ok(ProviderEndpointPin {
        dns_name: literal_ip.is_none().then(|| dns_host.to_owned()),
        addresses,
    })
}

fn validate_provider_resolved_ips(
    addresses: impl IntoIterator<Item = IpAddr>,
) -> Result<(), AppError> {
    if addresses.into_iter().any(private_or_metadata_ip) {
        Err(AppError::Forbidden(
            "provider endpoint DNS resolved to a non-public or special-use address".to_owned(),
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    server: Option<ServerSection>,
    auth: Option<AuthSection>,
    default_provider: Option<String>,
    provider_order: Option<Vec<String>>,
    providers: Option<HashMap<String, ProviderSection>>,
    aliases: Option<HashMap<String, String>>,
    routing: Option<SmartRoutingConfig>,
    runtime_adapters: Option<BTreeMap<String, RuntimeAdapterSection>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeAdapterSection {
    #[serde(default = "default_true")]
    enabled: bool,
    base_url: Option<String>,
    bearer_token_env: Option<String>,
    poll_interval_seconds: Option<u64>,
    stale_after_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ServerSection {
    bind: Option<String>,
    max_request_body_bytes: Option<usize>,
    max_concurrent_requests: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AuthSection {
    token_env: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProviderSection {
    display_name: Option<String>,
    protocol: ProviderProtocol,
    base_url: Option<String>,
    base_url_env: Option<String>,
    base_url_env_fallbacks: Option<Vec<String>>,
    api_key_env: Option<String>,
    api_key_required: Option<bool>,
    default_model: Option<String>,
    models: Option<Vec<String>>,
    model_prefixes: Option<Vec<String>>,
    passthrough_unknown_models: Option<bool>,
    max_tokens_field: Option<MaxTokensField>,
    deduplicate_stream_text: Option<bool>,
    buffer_stream_text: Option<bool>,
    fidelity_mode: Option<FidelityMode>,
    tool_use: Option<ToolUseConfig>,
    model_profile_defaults: Option<ModelProfileOverride>,
    model_profiles: Option<HashMap<String, ModelProfileOverride>>,
    reasoning: Option<ReasoningConfig>,
    sampling: Option<SamplingConfig>,
    token_counting: Option<TokenCountingConfig>,
    static_headers: Option<BTreeMap<String, String>>,
    request_timeout_ms: Option<u64>,
    stream_idle_timeout_ms: Option<u64>,
    retry: Option<ProviderRetryConfig>,
    pricing: Option<ModelPricing>,
    #[serde(default)]
    model_pricing: HashMap<String, ModelPricingCard>,
    #[serde(default)]
    trust_upstream_cost: bool,
}

struct ProviderSpec {
    id: &'static str,
    display_name: &'static str,
    protocol: ProviderProtocol,
    base_url_env: &'static str,
    base_url_env_fallbacks: &'static [&'static str],
    default_base_url: &'static str,
    api_key_env: Option<&'static str>,
    api_key_env_fallbacks: &'static [&'static str],
    api_key_required: bool,
    default_model_env: &'static str,
    default_model: &'static str,
    models_env: &'static str,
    models: &'static [&'static str],
    model_prefixes: &'static [&'static str],
    passthrough_unknown_models: bool,
    max_tokens_field: MaxTokensField,
    deduplicate_stream_text: bool,
}

const OPENAI_LEGACY_ENV_MIGRATIONS: &[(&str, &str)] = &[
    ("MODELPORT_OPENAI_BASE_URL", "OPENAI_BASE_URL"),
    ("MODELPORT_OPENAI_API_KEY", "OPENAI_API_KEY"),
    ("MODELPORT_OPENAI_MODEL", "OPENAI_MODEL"),
    ("MODELPORT_OPENAI_MODELS", "OPENAI_MODELS"),
];

impl AppConfig {
    pub fn load() -> Result<Self, AppError> {
        let path = config_path();
        if path.exists() {
            Self::from_file(&path)
        } else {
            Self::from_env_defaults()
        }
    }

    pub fn validate_client_auth(&self, headers: &HeaderMap) -> Result<(), AppError> {
        let Some(expected) = &self.auth_token else {
            return Ok(());
        };

        let x_api_key = headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok());
        let bearer = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));

        if x_api_key == Some(expected.as_str()) || bearer == Some(expected.as_str()) {
            Ok(())
        } else {
            Err(AppError::Auth)
        }
    }

    pub fn resolve(&self, requested_model: &str) -> Result<ResolvedProvider, AppError> {
        self.resolve_inner(requested_model.trim(), 0)
    }

    pub fn model_list(&self) -> Vec<(String, String)> {
        let mut seen = BTreeSet::new();
        let mut models = Vec::new();

        for id in &self.provider_order {
            let Some(provider) = self.providers.get(id) else {
                continue;
            };

            for model in &provider.models {
                if seen.insert(model.clone()) {
                    models.push((model.clone(), provider.display_name.clone()));
                }
            }
        }

        for (alias, target) in &self.aliases {
            if seen.contains(alias) {
                continue;
            }

            if let Some(display_name) = self.alias_display_name(target) {
                seen.insert(alias.clone());
                models.push((alias.clone(), display_name));
            }
        }

        if self.smart_routing.mode != SmartRoutingMode::Off {
            let mut routing_aliases = self
                .smart_routing
                .groups
                .values()
                .flat_map(|group| group.aliases.iter())
                .cloned()
                .collect::<Vec<_>>();
            routing_aliases.sort();
            for alias in routing_aliases {
                if seen.insert(alias.clone()) {
                    models.push((alias, "ModelPort Smart Router".to_owned()));
                }
            }
        }

        models
    }

    pub fn validation_issues(&self) -> Vec<ConfigIssue> {
        let mut issues = Vec::new();

        if self.auth_token.is_none() {
            issues.push(ConfigIssue::warning(
                "client authentication is disabled; only use MODELPORT_ALLOW_NO_AUTH=1 in isolated local testing",
            ));
        } else if self.auth_token.as_deref().is_some_and(is_placeholder_value) {
            issues.push(ConfigIssue::error(
                "MODELPORT_AUTH_TOKEN or ANTHROPIC_AUTH_TOKEN is still a placeholder",
            ));
        } else if self
            .auth_token
            .as_deref()
            .is_some_and(|token| token.len() < 16)
        {
            issues.push(ConfigIssue::warning(
                "client auth token is short; use a long random local token for production",
            ));
        }

        if !self.bind_addr.ip().is_loopback() {
            issues.push(ConfigIssue::warning(format!(
                "MODELPORT_BIND is {bind}; keep a reverse proxy or firewall in front when not binding loopback",
                bind = self.bind_addr
            )));
        }
        if self.max_request_body_bytes == 0 {
            issues.push(ConfigIssue::error(
                "MODELPORT_MAX_REQUEST_BODY_BYTES must be greater than 0",
            ));
        }
        if self.max_concurrent_requests == 0 {
            issues.push(ConfigIssue::error(
                "MODELPORT_MAX_CONCURRENT_REQUESTS must be greater than 0",
            ));
        }
        validate_runtime_guardrail_env(&mut issues);

        if self.providers.is_empty() {
            issues.push(ConfigIssue::error(
                "at least one provider must be configured",
            ));
        }

        if self.provider_order.is_empty() {
            issues.push(ConfigIssue::error(
                "provider_order is empty; at least one provider must be routable",
            ));
        } else if self.provider_order.len() > 256 {
            issues.push(ConfigIssue::error(
                "provider_order supports at most 256 routable providers",
            ));
        }

        if !self.providers.contains_key(&self.default_provider) {
            issues.push(ConfigIssue::error(format!(
                "default provider `{}` is not configured",
                self.default_provider
            )));
        }

        for id in &self.provider_order {
            if !self.providers.contains_key(id) {
                issues.push(ConfigIssue::error(format!(
                    "provider_order references missing provider `{id}`"
                )));
            }
        }

        let mut seen_models = HashMap::<String, String>::new();
        for (id, provider) in &self.providers {
            validate_provider(
                id,
                provider,
                id == &self.default_provider,
                &mut seen_models,
                &mut issues,
            );
            validate_cpa_provider(id, provider, &mut issues);
            if id == "openai"
                && openai_base_url_targets_modelport_listener(&provider.base_url, self.bind_addr)
            {
                issues.push(ConfigIssue::error(format!(
                    "provider `openai` base_url `{}` points back to this ModelPort listener; set server-side `MODELPORT_OPENAI_BASE_URL` to the upstream OpenAI API and reserve `OPENAI_BASE_URL` for client processes",
                    provider.base_url
                )));
            }
        }

        if self.providers.get("openai").is_some_and(|provider| {
            provider.api_key_env.as_deref() == Some("MODELPORT_OPENAI_API_KEY")
        }) {
            validate_openai_legacy_env_fallbacks(&mut issues);
        }

        for (alias, target) in &self.aliases {
            if alias.trim().is_empty() {
                issues.push(ConfigIssue::error("model alias name cannot be empty"));
                continue;
            }
            if target.trim().is_empty() {
                issues.push(ConfigIssue::error(format!(
                    "alias `{alias}` has an empty target"
                )));
                continue;
            }
            if let Err(err) = self.resolve(alias) {
                issues.push(ConfigIssue::error(format!(
                    "alias `{alias}` cannot resolve target `{target}`: {err}"
                )));
            }
        }
        validate_smart_routing(self, &mut issues);

        issues
    }

    pub(crate) fn smart_route_group(
        &self,
        requested_model: &str,
    ) -> Option<(&str, &RouteGroupConfig)> {
        let requested_model = requested_model.trim();
        self.smart_routing
            .groups
            .iter()
            .find(|(_, group)| group.aliases.iter().any(|alias| alias == requested_model))
            .map(|(id, group)| (id.as_str(), group))
    }

    fn resolve_inner(
        &self,
        requested_model: &str,
        depth: usize,
    ) -> Result<ResolvedProvider, AppError> {
        if depth > 8 {
            return Err(AppError::InvalidRequest(
                "model alias chain is too deep or cyclic".to_owned(),
            ));
        }

        if requested_model.is_empty() {
            return self.resolve_for_provider(&self.default_provider, None);
        }

        if let Some((provider_id, model)) = self.parse_provider_model(requested_model) {
            return self.resolve_for_provider(provider_id, Some(model));
        }

        if let Some(target) = self.aliases.get(requested_model) {
            return self.resolve_alias_target(requested_model, target, depth + 1);
        }

        if let Some((provider_id, provider)) = self.find_provider_by_exact_model(requested_model) {
            return Ok(ResolvedProvider {
                provider_id: provider_id.to_owned(),
                provider: provider.clone(),
                model: requested_model.to_owned(),
            });
        }

        if let Some((provider_id, provider)) = self.find_provider_by_model_prefix(requested_model) {
            return Ok(ResolvedProvider {
                provider_id: provider_id.to_owned(),
                provider: provider.clone(),
                model: requested_model.to_owned(),
            });
        }

        let provider = self
            .providers
            .get(&self.default_provider)
            .cloned()
            .ok_or_else(|| AppError::ProviderNotFound(self.default_provider.clone()))?;
        let model = if provider.passthrough_unknown_models {
            requested_model.to_owned()
        } else {
            provider.default_model.clone()
        };

        Ok(ResolvedProvider {
            provider_id: self.default_provider.clone(),
            provider,
            model,
        })
    }

    fn resolve_alias_target(
        &self,
        alias: &str,
        target: &str,
        depth: usize,
    ) -> Result<ResolvedProvider, AppError> {
        if let Some((provider_id, model)) = target.split_once(':') {
            if !self.providers.contains_key(provider_id) {
                return Err(AppError::ProviderNotFound(provider_id.to_owned()));
            }
            return self.resolve_for_provider(provider_id, Some(model));
        }

        if self.providers.contains_key(target) {
            let provider = self
                .providers
                .get(target)
                .cloned()
                .ok_or_else(|| AppError::ProviderNotFound(target.to_owned()))?;
            let model = if provider.models.iter().any(|model| model == alias) {
                alias.to_owned()
            } else {
                provider.default_model.clone()
            };
            return Ok(ResolvedProvider {
                provider_id: target.to_owned(),
                provider,
                model,
            });
        }

        self.resolve_inner(target, depth)
    }

    fn resolve_for_provider(
        &self,
        provider_id: &str,
        model: Option<&str>,
    ) -> Result<ResolvedProvider, AppError> {
        let provider = self
            .providers
            .get(provider_id)
            .cloned()
            .ok_or_else(|| AppError::ProviderNotFound(provider_id.to_owned()))?;
        let model = model
            .filter(|model| !model.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| provider.default_model.clone());

        Ok(ResolvedProvider {
            provider_id: provider_id.to_owned(),
            provider,
            model,
        })
    }

    fn parse_provider_model<'a>(&self, value: &'a str) -> Option<(&'a str, &'a str)> {
        let (provider_id, model) = value.split_once(':')?;
        self.providers
            .contains_key(provider_id)
            .then_some((provider_id, model.trim()))
    }

    fn alias_display_name(&self, target: &str) -> Option<String> {
        if let Some((provider_id, _)) = self.parse_provider_model(target) {
            return self
                .providers
                .get(provider_id)
                .map(|provider| provider.display_name.clone());
        }

        if self.providers.contains_key(target) {
            return self
                .providers
                .get(target)
                .map(|provider| provider.display_name.clone());
        }

        self.find_provider_by_exact_model(target)
            .or_else(|| self.find_provider_by_model_prefix(target))
            .map(|(_, provider)| provider.display_name.clone())
    }

    fn find_provider_by_exact_model(&self, model: &str) -> Option<(&str, &ProviderConfig)> {
        self.provider_order.iter().find_map(|id| {
            self.providers
                .get(id)
                .filter(|provider| {
                    provider
                        .models
                        .iter()
                        .any(|configured_model| configured_model == model)
                })
                .map(|provider| (id.as_str(), provider))
        })
    }

    fn find_provider_by_model_prefix(&self, model: &str) -> Option<(&str, &ProviderConfig)> {
        self.provider_order.iter().find_map(|id| {
            self.providers
                .get(id)
                .filter(|provider| {
                    provider
                        .model_prefixes
                        .iter()
                        .any(|prefix| model.starts_with(prefix))
                })
                .map(|provider| (id.as_str(), provider))
        })
    }

    fn from_file(path: &PathBuf) -> Result<Self, AppError> {
        let raw = fs::read_to_string(path)?;
        let file: FileConfig =
            toml::from_str(&raw).map_err(|err| AppError::Config(err.to_string()))?;
        let server = file.server.unwrap_or(ServerSection {
            bind: None,
            max_request_body_bytes: None,
            max_concurrent_requests: None,
        });

        let bind_addr = resolve_bind(server.bind)?;
        let max_request_body_bytes = resolve_usize_env(
            server.max_request_body_bytes,
            "MODELPORT_MAX_REQUEST_BODY_BYTES",
            DEFAULT_MAX_REQUEST_BODY_BYTES,
        );
        let max_concurrent_requests = resolve_usize_env(
            server.max_concurrent_requests,
            "MODELPORT_MAX_CONCURRENT_REQUESTS",
            DEFAULT_MAX_CONCURRENT_REQUESTS,
        );
        let auth_token = require_auth_token(
            file.auth
                .and_then(|auth| auth.token_env)
                .and_then(|name| env_value(&name))
                .or_else(default_auth_token),
        )?;

        let configured_default_provider = file.default_provider.clone();
        let mut providers = HashMap::new();
        let mut provider_order = Vec::new();
        let mut provider_sections = file.providers.unwrap_or_default();
        let mut ordered_provider_ids = Vec::new();

        for id in file.provider_order.unwrap_or_default() {
            if provider_sections.contains_key(&id) && !ordered_provider_ids.contains(&id) {
                ordered_provider_ids.push(id);
            }
        }

        let mut remaining_provider_ids = provider_sections.keys().cloned().collect::<Vec<_>>();
        remaining_provider_ids.sort();
        for id in remaining_provider_ids {
            if !ordered_provider_ids.contains(&id) {
                ordered_provider_ids.push(id);
            }
        }

        for id in ordered_provider_ids {
            let section = provider_sections.remove(&id).ok_or_else(|| {
                AppError::Config(format!("provider `{id}` disappeared while loading config"))
            })?;
            let base_url = section
                .base_url_env
                .as_deref()
                .and_then(env_value)
                .or_else(|| {
                    section
                        .base_url_env_fallbacks
                        .as_deref()
                        .and_then(first_env_owned)
                })
                .or(section.base_url)
                .ok_or_else(|| {
                    AppError::Config(format!("provider `{id}` needs base_url or base_url_env"))
                })?;

            let models = section.models.unwrap_or_default();
            let default_model = section
                .default_model
                .clone()
                .or_else(|| models.first().cloned())
                .unwrap_or_else(|| id.clone());
            let api_key = section.api_key_env.as_deref().and_then(env_value);
            let api_key_required = section
                .api_key_required
                .unwrap_or(section.api_key_env.is_some());

            if api_key_required
                && api_key.is_none()
                && configured_default_provider.as_deref() != Some(id.as_str())
                && !env_flag("MODELPORT_INCLUDE_UNAVAILABLE_PROVIDERS")
            {
                continue;
            }

            let deduplicate_stream_text = section.deduplicate_stream_text.unwrap_or(false);
            let buffer_stream_text = section.buffer_stream_text.unwrap_or(false);
            let tool_use = section.tool_use.unwrap_or_else(|| {
                default_tool_use_config(&id, section.protocol, deduplicate_stream_text)
            });

            insert_provider(
                &mut providers,
                &mut provider_order,
                id.clone(),
                ProviderConfig {
                    display_name: section.display_name.unwrap_or_else(|| id.clone()),
                    protocol: section.protocol,
                    base_url,
                    api_key,
                    api_key_required,
                    api_key_env: section.api_key_env,
                    default_model,
                    models,
                    model_prefixes: section.model_prefixes.unwrap_or_default(),
                    passthrough_unknown_models: section.passthrough_unknown_models.unwrap_or(false),
                    max_tokens_field: section
                        .max_tokens_field
                        .unwrap_or(MaxTokensField::MaxCompletionTokens),
                    deduplicate_stream_text,
                    buffer_stream_text,
                    fidelity_mode: section.fidelity_mode.unwrap_or_else(|| {
                        default_fidelity_mode(&id, deduplicate_stream_text, buffer_stream_text)
                    }),
                    tool_use,
                    model_profile_defaults: section.model_profile_defaults.unwrap_or_default(),
                    model_profiles: section.model_profiles.unwrap_or_default(),
                    reasoning: section.reasoning.unwrap_or_default(),
                    sampling: section.sampling.unwrap_or_default(),
                    token_counting: section.token_counting.unwrap_or_default(),
                    static_headers: section.static_headers.unwrap_or_default(),
                    request_timeout_ms: section.request_timeout_ms,
                    stream_idle_timeout_ms: section.stream_idle_timeout_ms,
                    retry: section.retry.unwrap_or_default(),
                    pricing: section.pricing,
                    model_pricing: section.model_pricing,
                    trust_upstream_cost: section.trust_upstream_cost,
                },
            );
        }

        let default_provider = file
            .default_provider
            .or_else(|| provider_order.first().cloned())
            .ok_or_else(|| AppError::Config("at least one provider is required".to_owned()))?;
        let mut smart_routing = file.routing.unwrap_or_default();
        apply_smart_routing_env_override(&mut smart_routing)?;
        let runtime_adapters = load_runtime_adapters(file.runtime_adapters.unwrap_or_default())?;

        Ok(Self {
            bind_addr,
            max_request_body_bytes,
            max_concurrent_requests,
            auth_token,
            default_provider,
            provider_order,
            providers,
            aliases: file.aliases.unwrap_or_default(),
            smart_routing,
            runtime_adapters,
        })
    }

    fn from_env_defaults() -> Result<Self, AppError> {
        let bind_addr = resolve_bind(service_env_value("MODELPORT_BIND"))?;
        let max_request_body_bytes = resolve_usize_env(
            None,
            "MODELPORT_MAX_REQUEST_BODY_BYTES",
            DEFAULT_MAX_REQUEST_BODY_BYTES,
        );
        let max_concurrent_requests = resolve_usize_env(
            None,
            "MODELPORT_MAX_CONCURRENT_REQUESTS",
            DEFAULT_MAX_CONCURRENT_REQUESTS,
        );
        let mut providers = HashMap::new();
        let mut provider_order = Vec::new();

        insert_spec(&mut providers, &mut provider_order, &DEEPSEEK_SPEC);

        if should_enable_provider(&MIMO_SPEC) {
            insert_spec(&mut providers, &mut provider_order, &MIMO_SPEC);
        }

        for spec in OPTIONAL_PROVIDER_SPECS {
            if should_enable_provider(spec) {
                insert_spec(&mut providers, &mut provider_order, spec);
            }
        }

        if should_enable_custom_openai_provider() {
            insert_spec(&mut providers, &mut provider_order, &CUSTOM_OPENAI_SPEC);
        }

        let aliases = default_aliases();
        let default_provider =
            env_value("MODELPORT_DEFAULT_PROVIDER").unwrap_or_else(|| "deepseek".to_owned());
        let mut smart_routing = SmartRoutingConfig::default();
        apply_smart_routing_env_override(&mut smart_routing)?;

        Ok(Self {
            bind_addr,
            max_request_body_bytes,
            max_concurrent_requests,
            auth_token: require_auth_token(default_auth_token())?,
            default_provider,
            provider_order,
            providers,
            aliases,
            smart_routing,
            runtime_adapters: BTreeMap::new(),
        })
    }
}

fn load_runtime_adapters(
    sections: BTreeMap<String, RuntimeAdapterSection>,
) -> Result<BTreeMap<String, RuntimeAdapterConfig>, AppError> {
    if sections.len() > MAX_RUNTIME_ADAPTERS {
        return Err(AppError::Config(format!(
            "runtime_adapters supports at most {MAX_RUNTIME_ADAPTERS} entries"
        )));
    }

    let mut adapters = BTreeMap::new();
    for (adapter_id, section) in sections {
        if !section.enabled {
            continue;
        }
        let base_url = section.base_url.ok_or_else(|| {
            AppError::Config(format!(
                "enabled Runtime Adapter `{adapter_id}` requires base_url"
            ))
        })?;
        let credential_env = section.bearer_token_env.ok_or_else(|| {
            AppError::Config(format!(
                "enabled Runtime Adapter `{adapter_id}` requires bearer_token_env"
            ))
        })?;
        validate_secret_env_name(&credential_env).map_err(|message| {
            AppError::Config(format!("Runtime Adapter `{adapter_id}` {message}"))
        })?;
        let bearer_token = env_value(&credential_env).ok_or_else(|| {
            AppError::Config(format!(
                "enabled Runtime Adapter `{adapter_id}` Bearer credential environment variable is unset or empty"
            ))
        })?;
        let poll_seconds = section
            .poll_interval_seconds
            .unwrap_or(DEFAULT_RUNTIME_ADAPTER_POLL_INTERVAL_SECONDS);
        let stale_seconds = section
            .stale_after_seconds
            .unwrap_or(DEFAULT_RUNTIME_ADAPTER_STALE_AFTER_SECONDS);
        if !(MIN_RUNTIME_ADAPTER_INTERVAL_SECONDS..=MAX_RUNTIME_ADAPTER_POLL_INTERVAL_SECONDS)
            .contains(&poll_seconds)
        {
            return Err(AppError::Config(format!(
                "Runtime Adapter `{adapter_id}` poll_interval_seconds must be from {MIN_RUNTIME_ADAPTER_INTERVAL_SECONDS} to {MAX_RUNTIME_ADAPTER_POLL_INTERVAL_SECONDS}"
            )));
        }
        if !(MIN_RUNTIME_ADAPTER_INTERVAL_SECONDS..=MAX_RUNTIME_ADAPTER_STALE_AFTER_SECONDS)
            .contains(&stale_seconds)
            || stale_seconds < poll_seconds
        {
            return Err(AppError::Config(format!(
                "Runtime Adapter `{adapter_id}` stale_after_seconds must be from {MIN_RUNTIME_ADAPTER_INTERVAL_SECONDS} to {MAX_RUNTIME_ADAPTER_STALE_AFTER_SECONDS} and cover at least one polling interval"
            )));
        }
        let client = RuntimeAdapterClientConfig::new(adapter_id.clone(), base_url, bearer_token)
            .map_err(|error| {
                AppError::Config(format!(
                    "Runtime Adapter `{adapter_id}` configuration is invalid: {error}"
                ))
            })?;
        adapters.insert(
            adapter_id,
            RuntimeAdapterConfig {
                client_config: client,
                credential_env,
                poll_interval: Duration::from_secs(poll_seconds),
                stale_after: Duration::from_secs(stale_seconds),
            },
        );
    }
    Ok(adapters)
}

fn validate_secret_env_name(name: &str) -> Result<(), &'static str> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 128
        || !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        || !bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        return Err("bearer_token_env must be a valid environment-variable name");
    }
    Ok(())
}

impl RuntimeConfig {
    pub fn new(config: AppConfig) -> Self {
        Self::with_loader(config, AppConfig::load)
    }

    pub fn with_loader(
        config: AppConfig,
        loader: impl Fn() -> Result<AppConfig, AppError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: RwLock::new(config),
            loader: Arc::new(loader),
        }
    }

    pub fn snapshot(&self) -> AppConfig {
        self.inner
            .read()
            .expect("runtime config lock poisoned")
            .clone()
    }

    pub fn reload(&self) -> Result<AppConfig, AppError> {
        let config = (self.loader)()?;
        let error_count = config
            .validation_issues()
            .iter()
            .filter(|issue| issue.severity == ConfigIssueSeverity::Error)
            .count();

        if error_count > 0 {
            return Err(AppError::Config(format!(
                "configuration reload rejected with {error_count} error(s); run `model-port config validate` for details"
            )));
        }

        *self.inner.write().expect("runtime config lock poisoned") = config.clone();
        Ok(config)
    }
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfig")
            .finish_non_exhaustive()
    }
}

impl ConfigIssue {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: ConfigIssueSeverity::Error,
            message: message.into(),
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: ConfigIssueSeverity::Warning,
            message: message.into(),
        }
    }
}

impl ProviderConfig {
    pub(crate) fn model_pricing_card(&self, model: &str) -> Option<&ModelPricingCard> {
        self.model_pricing.get(model)
    }

    pub(crate) fn effective_pricing(&self, model: &str) -> Option<ModelPricing> {
        self.model_pricing_card(model)
            .map(|card| card.rates)
            .or(self.pricing)
    }

    pub(crate) fn model_profile(&self, provider_id: &str, model: &str) -> ModelProfile {
        resolve_model_profile(
            provider_id,
            self.protocol,
            model,
            &self.tool_use,
            &self.model_profile_defaults,
            self.model_profiles.get(model),
        )
    }

    pub(crate) fn tool_use_for_model(&self, provider_id: &str, model: &str) -> ToolUseConfig {
        let profile = self.model_profile(provider_id, model);
        let mut tool_use = self.tool_use;
        tool_use.supported = profile.tool_use.is_supported();
        tool_use.tool_choice = profile.tool_choice.is_supported();
        tool_use.parallel_tool_calls = profile.parallel_tool_calls.is_supported();
        tool_use
    }

    pub(crate) fn request_timeout(&self) -> Option<Duration> {
        self.request_timeout_ms.map(Duration::from_millis)
    }

    pub(crate) fn stream_idle_timeout(&self) -> Option<Duration> {
        self.stream_idle_timeout_ms.map(Duration::from_millis)
    }

    pub fn api_key(&self) -> Result<Option<&str>, AppError> {
        if let Some(api_key) = self.api_key.as_deref() {
            return Ok(Some(api_key));
        }

        if self.api_key_required {
            let name = self
                .api_key_env
                .clone()
                .unwrap_or_else(|| format!("{}_API_KEY", self.display_name.to_uppercase()));
            Err(AppError::MissingSecret(name))
        } else {
            Ok(None)
        }
    }

    pub fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

const DEEPSEEK_SPEC: ProviderSpec = ProviderSpec {
    id: "deepseek",
    display_name: "DeepSeek",
    protocol: ProviderProtocol::Anthropic,
    base_url_env: "DEEPSEEK_ANTHROPIC_BASE_URL",
    base_url_env_fallbacks: &[],
    default_base_url: "https://api.deepseek.com/anthropic",
    api_key_env: Some("DEEPSEEK_ANTHROPIC_AUTH_TOKEN"),
    api_key_env_fallbacks: &["DEEPSEEK_API_KEY"],
    api_key_required: true,
    default_model_env: "DEEPSEEK_MODEL",
    default_model: "deepseek-v4-flash",
    models_env: "DEEPSEEK_MODELS",
    models: &["deepseek-v4-pro", "deepseek-v4-flash"],
    model_prefixes: &["deepseek-"],
    passthrough_unknown_models: false,
    max_tokens_field: MaxTokensField::MaxTokens,
    deduplicate_stream_text: true,
};

const MIMO_SPEC: ProviderSpec = ProviderSpec {
    id: "mimo",
    display_name: "小米 MiMo",
    protocol: ProviderProtocol::OpenaiCompat,
    base_url_env: "MIMO_OPENAI_BASE_URL",
    base_url_env_fallbacks: &["BASE_URL"],
    default_base_url: "https://api.xiaomimimo.com/v1",
    api_key_env: Some("MIMO_OPENAI_API_KEY"),
    api_key_env_fallbacks: &[],
    api_key_required: true,
    default_model_env: "MIMO_MODEL",
    default_model: "mimo-v2.5-pro",
    models_env: "MIMO_MODELS",
    models: &["mimo-v2.5-pro"],
    model_prefixes: &["mimo-"],
    passthrough_unknown_models: false,
    max_tokens_field: MaxTokensField::MaxCompletionTokens,
    deduplicate_stream_text: false,
};

const OPTIONAL_PROVIDER_SPECS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "cpa_codex",
        display_name: "CPA · OpenAI Codex",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "CPA_CODEX_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "http://127.0.0.1:8317/v1",
        api_key_env: Some("CPA_CODEX_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "CPA_CODEX_MODEL",
        default_model: "gpt-5.3-codex",
        models_env: "CPA_CODEX_MODELS",
        models: &["gpt-5.3-codex"],
        model_prefixes: &[],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "cpa_claude",
        display_name: "CPA · Claude Code",
        protocol: ProviderProtocol::Anthropic,
        base_url_env: "CPA_CLAUDE_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "http://127.0.0.1:8317",
        api_key_env: Some("CPA_CLAUDE_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "CPA_CLAUDE_MODEL",
        default_model: "claude-sonnet-4-6",
        models_env: "CPA_CLAUDE_MODELS",
        models: &["claude-sonnet-4-6"],
        model_prefixes: &[],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "deepseek_openai",
        display_name: "DeepSeek",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "DEEPSEEK_OPENAI_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://api.deepseek.com",
        api_key_env: Some("DEEPSEEK_OPENAI_API_KEY"),
        api_key_env_fallbacks: &["DEEPSEEK_API_KEY"],
        api_key_required: true,
        default_model_env: "DEEPSEEK_OPENAI_MODEL",
        default_model: "deepseek-v4-flash",
        models_env: "DEEPSEEK_OPENAI_MODELS",
        models: &["deepseek-v4-pro", "deepseek-v4-flash"],
        model_prefixes: &["deepseek-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "anthropic",
        display_name: "Anthropic Claude",
        protocol: ProviderProtocol::Anthropic,
        base_url_env: "ANTHROPIC_UPSTREAM_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://api.anthropic.com",
        api_key_env: Some("ANTHROPIC_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "ANTHROPIC_UPSTREAM_MODEL",
        default_model: "claude-fable-5",
        models_env: "ANTHROPIC_UPSTREAM_MODELS",
        models: &[
            "claude-fable-5",
            "claude-mythos-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-sonnet-4-6",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            "claude-3-5-haiku-20241022",
        ],
        model_prefixes: &["claude-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "openai",
        display_name: "OpenAI",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "MODELPORT_OPENAI_BASE_URL",
        base_url_env_fallbacks: &["OPENAI_BASE_URL"],
        default_base_url: "https://api.openai.com/v1",
        api_key_env: Some("MODELPORT_OPENAI_API_KEY"),
        api_key_env_fallbacks: &["OPENAI_API_KEY"],
        api_key_required: true,
        default_model_env: "MODELPORT_OPENAI_MODEL",
        default_model: "gpt-5.5",
        models_env: "MODELPORT_OPENAI_MODELS",
        models: &[
            "gpt-5.5",
            "gpt-5.5-pro",
            "gpt-5.4",
            "gpt-5.4-pro",
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            "gpt-5.3-codex",
            "gpt-5.2",
            "gpt-5",
            "gpt-5-mini",
            "gpt-4.1",
            "gpt-4.1-mini",
        ],
        model_prefixes: &["gpt-", "o1", "o3", "o4", "o5", "chatgpt-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "openrouter",
        display_name: "OpenRouter",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "OPENROUTER_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://openrouter.ai/api/v1",
        api_key_env: Some("OPENROUTER_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "OPENROUTER_MODEL",
        default_model: "openrouter/auto",
        models_env: "OPENROUTER_MODELS",
        models: &["openrouter/auto"],
        model_prefixes: &[
            "anthropic/",
            "deepseek/",
            "google/",
            "meta-llama/",
            "mistralai/",
            "moonshotai/",
            "openai/",
            "qwen/",
            "x-ai/",
            "z-ai/",
        ],
        passthrough_unknown_models: true,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "gemini",
        display_name: "Google Gemini",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "GEMINI_OPENAI_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        api_key_env: Some("GEMINI_API_KEY"),
        api_key_env_fallbacks: &["GOOGLE_API_KEY"],
        api_key_required: true,
        default_model_env: "GEMINI_MODEL",
        default_model: "gemini-3.5-flash",
        models_env: "GEMINI_MODELS",
        models: &[
            "gemini-3.5-flash",
            "gemini-3.5-pro",
            "gemini-2.5-pro",
            "gemini-2.5-flash",
            "gemini-2.5-flash-lite",
        ],
        model_prefixes: &["gemini-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "xai",
        display_name: "xAI Grok",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "XAI_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://api.x.ai/v1",
        api_key_env: Some("XAI_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "XAI_MODEL",
        default_model: "grok-3",
        models_env: "XAI_MODELS",
        models: &["grok-3", "grok-3-mini"],
        model_prefixes: &["grok-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "groq",
        display_name: "Groq",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "GROQ_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://api.groq.com/openai/v1",
        api_key_env: Some("GROQ_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "GROQ_MODEL",
        default_model: "llama-3.3-70b-versatile",
        models_env: "GROQ_MODELS",
        models: &["llama-3.3-70b-versatile", "llama-3.1-8b-instant"],
        model_prefixes: &["llama-", "mixtral-", "gemma-", "openai/"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "dashscope",
        display_name: "阿里云百炼 Qwen",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "DASHSCOPE_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        api_key_env: Some("DASHSCOPE_API_KEY"),
        api_key_env_fallbacks: &["QWEN_API_KEY"],
        api_key_required: true,
        default_model_env: "DASHSCOPE_MODEL",
        default_model: "qwen-plus",
        models_env: "DASHSCOPE_MODELS",
        models: &[
            "qwen-plus",
            "qwen-max",
            "qwen-turbo",
            "qwen3-plus",
            "qwen3-max",
            "qwq-plus",
            "qvq-max",
        ],
        model_prefixes: &["qwen-", "qwq-", "qvq-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "kimi",
        display_name: "Moonshot Kimi",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "KIMI_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://api.moonshot.cn/v1",
        api_key_env: Some("MOONSHOT_API_KEY"),
        api_key_env_fallbacks: &["KIMI_API_KEY"],
        api_key_required: true,
        default_model_env: "KIMI_MODEL",
        default_model: "kimi-k2.6",
        models_env: "KIMI_MODELS",
        models: &[
            "kimi-k2.6",
            "kimi-k2",
            "moonshot-v1-128k",
            "moonshot-v1-32k",
            "moonshot-v1-8k",
        ],
        model_prefixes: &["kimi-", "moonshot-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "zhipu",
        display_name: "智谱 GLM",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "ZHIPU_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://open.bigmodel.cn/api/paas/v4",
        api_key_env: Some("ZHIPU_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "ZHIPU_MODEL",
        default_model: "glm-4.7",
        models_env: "ZHIPU_MODELS",
        models: &["glm-4.7", "glm-4.6", "glm-4-flash", "glm-z1-flash"],
        model_prefixes: &["glm-", "charglm-", "codegeex-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "mistral",
        display_name: "Mistral AI",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "MISTRAL_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://api.mistral.ai/v1",
        api_key_env: Some("MISTRAL_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "MISTRAL_MODEL",
        default_model: "mistral-large-latest",
        models_env: "MISTRAL_MODELS",
        models: &["mistral-large-latest", "codestral-latest"],
        model_prefixes: &[
            "codestral-",
            "devstral-",
            "ministral-",
            "mistral-",
            "pixtral-",
        ],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "ark",
        display_name: "火山方舟 Doubao",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "ARK_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://ark.cn-beijing.volces.com/api/v3",
        api_key_env: Some("ARK_API_KEY"),
        api_key_env_fallbacks: &["VOLCENGINE_API_KEY"],
        api_key_required: true,
        default_model_env: "ARK_MODEL",
        default_model: "doubao-seed-1-6-250615",
        models_env: "ARK_MODELS",
        models: &["doubao-seed-1-6-250615", "doubao-seed-1-6-flash-250615"],
        model_prefixes: &["doubao-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "ollama",
        display_name: "Ollama",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "OLLAMA_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "http://127.0.0.1:11434/v1",
        api_key_env: Some("OLLAMA_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: false,
        default_model_env: "OLLAMA_MODEL",
        default_model: "llama3.1",
        models_env: "OLLAMA_MODELS",
        models: &["llama3.1", "qwen2.5-coder"],
        model_prefixes: &[],
        passthrough_unknown_models: true,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "local_sglang",
        display_name: "Local SGLang",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "SGLANG_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "http://127.0.0.1:30000/v1",
        api_key_env: Some("SGLANG_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: false,
        default_model_env: "SGLANG_MODEL",
        default_model: "local-model",
        models_env: "SGLANG_MODELS",
        models: &["local-model"],
        model_prefixes: &[],
        passthrough_unknown_models: true,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "local_vllm",
        display_name: "Local vLLM",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "VLLM_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "http://127.0.0.1:8000/v1",
        api_key_env: Some("VLLM_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: false,
        default_model_env: "VLLM_MODEL",
        default_model: "local-model",
        models_env: "VLLM_MODELS",
        models: &["local-model"],
        model_prefixes: &[],
        passthrough_unknown_models: true,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "local_llamacpp",
        display_name: "Local llama.cpp",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "LLAMACPP_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "http://127.0.0.1:8080/v1",
        api_key_env: Some("LLAMACPP_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: false,
        default_model_env: "LLAMACPP_MODEL",
        default_model: "local-model",
        models_env: "LLAMACPP_MODELS",
        models: &["local-model"],
        model_prefixes: &[],
        passthrough_unknown_models: true,
        max_tokens_field: MaxTokensField::MaxTokens,
        deduplicate_stream_text: false,
    },
];

const CUSTOM_OPENAI_SPEC: ProviderSpec = ProviderSpec {
    id: "custom",
    display_name: "自定义 OpenAI 兼容",
    protocol: ProviderProtocol::OpenaiCompat,
    base_url_env: "CUSTOM_OPENAI_BASE_URL",
    base_url_env_fallbacks: &[],
    default_base_url: "http://127.0.0.1:8000/v1",
    api_key_env: Some("CUSTOM_OPENAI_API_KEY"),
    api_key_env_fallbacks: &[],
    api_key_required: false,
    default_model_env: "CUSTOM_OPENAI_MODEL",
    default_model: "default",
    models_env: "CUSTOM_OPENAI_MODELS",
    models: &["default"],
    model_prefixes: &[],
    passthrough_unknown_models: true,
    max_tokens_field: MaxTokensField::MaxCompletionTokens,
    deduplicate_stream_text: false,
};

fn insert_spec(
    providers: &mut HashMap<String, ProviderConfig>,
    provider_order: &mut Vec<String>,
    spec: &ProviderSpec,
) {
    let default_model = provider_env_value(spec, spec.default_model_env).unwrap_or_else(|| {
        if spec.id == "mimo" {
            env_value("ANTHROPIC_MODEL")
                .filter(|model| model.starts_with("mimo-"))
                .unwrap_or_else(|| spec.default_model.to_owned())
        } else {
            spec.default_model.to_owned()
        }
    });
    let mut models = provider_env_list(spec, spec.models_env, spec.models);
    if !models.contains(&default_model) {
        models.insert(0, default_model.clone());
    }

    if spec.id == "mimo" {
        extend_mimo_models_from_claude_env(&mut models);
    }

    let api_key = spec
        .api_key_env
        .and_then(env_value)
        .or_else(|| first_env(spec.api_key_env_fallbacks));
    let buffer_stream_text = default_buffer_stream_text(spec.id);

    insert_provider(
        providers,
        provider_order,
        spec.id.to_owned(),
        ProviderConfig {
            display_name: spec.display_name.to_owned(),
            protocol: spec.protocol,
            base_url: env_value(spec.base_url_env)
                .or_else(|| first_env(spec.base_url_env_fallbacks))
                .unwrap_or_else(|| spec.default_base_url.to_owned()),
            api_key_env: spec.api_key_env.map(str::to_owned),
            api_key,
            api_key_required: spec.api_key_required,
            default_model,
            models,
            model_prefixes: spec
                .model_prefixes
                .iter()
                .map(|prefix| (*prefix).to_owned())
                .collect(),
            passthrough_unknown_models: spec.passthrough_unknown_models,
            max_tokens_field: spec.max_tokens_field,
            deduplicate_stream_text: spec.deduplicate_stream_text,
            buffer_stream_text,
            fidelity_mode: default_fidelity_mode(
                spec.id,
                spec.deduplicate_stream_text,
                buffer_stream_text,
            ),
            tool_use: default_tool_use_config(spec.id, spec.protocol, spec.deduplicate_stream_text),
            model_profile_defaults: ModelProfileOverride::default(),
            model_profiles: HashMap::new(),
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            static_headers: BTreeMap::new(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: ProviderRetryConfig::default(),
            pricing: None,
            model_pricing: default_model_pricing(spec.id),
            trust_upstream_cost: spec.id == "openrouter",
        },
    );
}

fn default_model_pricing(provider_id: &str) -> HashMap<String, ModelPricingCard> {
    if !matches!(provider_id, "deepseek" | "deepseek_openai") {
        return HashMap::new();
    }

    let card = |rates, model: &str| ModelPricingCard {
        rates,
        version: "deepseek-public-2026-04-24-v1".to_owned(),
        effective_at: "2026-04-24T00:00:00Z".to_owned(),
        currency: "USD".to_owned(),
        source: PricingSource::ProviderPublished,
        service_tier: PricingServiceTier::Standard,
        region: None,
        evidence: format!(
            "https://api-docs.deepseek.com/quick_start/pricing#{}",
            model
        ),
    };
    HashMap::from([
        (
            "deepseek-v4-flash".to_owned(),
            card(
                ModelPricing {
                    input_per_million: 0.14,
                    output_per_million: 0.28,
                    cache_write_per_million: 0.14,
                    cache_read_per_million: 0.0028,
                },
                "deepseek-v4-flash",
            ),
        ),
        (
            "deepseek-v4-pro".to_owned(),
            card(
                ModelPricing {
                    input_per_million: 0.435,
                    output_per_million: 0.87,
                    cache_write_per_million: 0.435,
                    cache_read_per_million: 0.003625,
                },
                "deepseek-v4-pro",
            ),
        ),
    ])
}

fn default_fidelity_mode(
    _provider_id: &str,
    deduplicate_stream_text: bool,
    buffer_stream_text: bool,
) -> FidelityMode {
    if deduplicate_stream_text || buffer_stream_text {
        FidelityMode::Stability
    } else {
        FidelityMode::BestEffort
    }
}

fn default_buffer_stream_text(provider_id: &str) -> bool {
    env_bool(
        &format!(
            "MODELPORT_{}_BUFFER_STREAM_TEXT",
            env_key_fragment(provider_id)
        ),
        false,
    )
}

fn default_tool_use_config(
    provider_id: &str,
    protocol: ProviderProtocol,
    deduplicate_stream_text: bool,
) -> ToolUseConfig {
    let streaming_arguments = match protocol {
        ProviderProtocol::Anthropic => ToolArgumentMode::Native,
        ProviderProtocol::OpenaiCompat if deduplicate_stream_text => ToolArgumentMode::Cumulative,
        ProviderProtocol::OpenaiCompat if is_unknown_tool_runtime(provider_id) => {
            ToolArgumentMode::BestEffort
        }
        ProviderProtocol::OpenaiCompat => ToolArgumentMode::Delta,
    };

    ToolUseConfig {
        supported: true,
        tool_choice: true,
        parallel_tool_calls: !is_single_tool_runtime(provider_id),
        streaming_arguments,
        response_validation: ToolResponseValidation::BestEffort,
        repair_invalid_arguments: false,
    }
}

fn is_single_tool_runtime(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "ollama" | "local_sglang" | "local_vllm" | "local_llamacpp"
    )
}

fn is_unknown_tool_runtime(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "custom" | "ollama" | "local_sglang" | "local_vllm" | "local_llamacpp"
    )
}

fn default_true() -> bool {
    true
}

fn insert_provider(
    providers: &mut HashMap<String, ProviderConfig>,
    provider_order: &mut Vec<String>,
    id: String,
    provider: ProviderConfig,
) {
    if !providers.contains_key(&id) {
        provider_order.push(id.clone());
    }
    providers.insert(id, provider);
}

fn should_enable_provider(spec: &ProviderSpec) -> bool {
    if env_flag(&format!("MODELPORT_ENABLE_{}", env_key_fragment(spec.id))) {
        return true;
    }

    if env_value(spec.base_url_env).is_some()
        || provider_env_value(spec, spec.default_model_env).is_some()
        || provider_env_value(spec, spec.models_env).is_some()
    {
        return true;
    }

    if first_env(spec.base_url_env_fallbacks).is_some() {
        return true;
    }

    spec.api_key_env.and_then(env_value).is_some()
        || first_env(spec.api_key_env_fallbacks).is_some()
}

fn should_enable_custom_openai_provider() -> bool {
    env_value(CUSTOM_OPENAI_SPEC.base_url_env).is_some()
        || env_value(CUSTOM_OPENAI_SPEC.default_model_env).is_some()
        || env_value("CUSTOM_OPENAI_API_KEY").is_some()
        || env_flag("MODELPORT_ENABLE_CUSTOM")
}

fn extend_mimo_models_from_claude_env(models: &mut Vec<String>) {
    for name in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ] {
        if let Some(value) = env_value(name)
            && value.starts_with("mimo-")
            && !models.contains(&value)
        {
            models.push(value);
        }
    }
}

fn provider_env_list(spec: &ProviderSpec, name: &str, defaults: &[&str]) -> Vec<String> {
    provider_env_value(spec, name)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .unwrap_or_else(|| defaults.iter().map(|value| (*value).to_owned()).collect())
}

fn provider_env_value(spec: &ProviderSpec, name: &str) -> Option<String> {
    let preferred = env_value(name);
    let fallback = openai_legacy_env_name(spec.id, name).and_then(env_value);
    preferred.or(fallback)
}

fn openai_legacy_env_name(provider_id: &str, preferred_name: &str) -> Option<&'static str> {
    if provider_id != "openai" {
        return None;
    }

    match preferred_name {
        "MODELPORT_OPENAI_MODEL" => Some("OPENAI_MODEL"),
        "MODELPORT_OPENAI_MODELS" => Some("OPENAI_MODELS"),
        _ => None,
    }
}

fn validate_openai_legacy_env_fallbacks(issues: &mut Vec<ConfigIssue>) {
    let active_fallbacks = OPENAI_LEGACY_ENV_MIGRATIONS
        .iter()
        .filter(|(preferred, legacy)| env_value(preferred).is_none() && env_value(legacy).is_some())
        .map(|(preferred, legacy)| format!("`{legacy}` -> `{preferred}`"))
        .collect::<Vec<_>>();

    if !active_fallbacks.is_empty() {
        issues.push(ConfigIssue::warning(format!(
            "provider `openai` is using legacy client-style environment fallback(s): {}; migrate the ModelPort server to `MODELPORT_OPENAI_*` names so client `OPENAI_*` settings cannot be mistaken for upstream configuration",
            active_fallbacks.join(", ")
        )));
    }
}

pub(crate) fn env_value(name: &str) -> Option<String> {
    let process_value = env::var(name).ok();
    if process_value.is_some() {
        return process_value;
    }
    let mut file_values = env_file_values();
    select_env_value(process_value, file_values.remove(name))
}

fn select_env_value(process_value: Option<String>, file_value: Option<String>) -> Option<String> {
    process_value.or(file_value)
}

fn validate_runtime_guardrail_env(issues: &mut Vec<ConfigIssue>) {
    for (name, requirement) in [
        (
            "MODELPORT_MAX_REQUEST_BODY_BYTES",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_MAX_CONCURRENT_REQUESTS",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_MAX_CONCURRENT_STREAMS",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_RATE_LIMIT_WINDOW_SECONDS",
            NumericEnvRequirement::NonZeroU64,
        ),
        (
            "MODELPORT_RATE_LIMIT_GLOBAL_PER_MINUTE",
            NumericEnvRequirement::U32,
        ),
        (
            "MODELPORT_RATE_LIMIT_API_KEY_PER_MINUTE",
            NumericEnvRequirement::U32,
        ),
        (
            "MODELPORT_RATE_LIMIT_IP_PER_MINUTE",
            NumericEnvRequirement::U32,
        ),
        (
            "MODELPORT_RATE_LIMIT_PROVIDER_PER_MINUTE",
            NumericEnvRequirement::U32,
        ),
        (
            "MODELPORT_RATE_LIMIT_MODEL_PER_MINUTE",
            NumericEnvRequirement::U32,
        ),
        (
            "MODELPORT_MAX_MODEL_NAME_CHARS",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_MAX_MESSAGES",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_MAX_MESSAGES_JSON_CHARS",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_MAX_SYSTEM_JSON_CHARS",
            NumericEnvRequirement::NonZeroUsize,
        ),
        ("MODELPORT_MAX_TOOLS", NumericEnvRequirement::NonZeroUsize),
        (
            "MODELPORT_MAX_TOOLS_JSON_CHARS",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_MAX_OUTPUT_TOKENS",
            NumericEnvRequirement::NonZeroU64,
        ),
        (
            "MODELPORT_HTTP_CONNECT_TIMEOUT_SECS",
            NumericEnvRequirement::NonZeroU64,
        ),
        (
            "MODELPORT_HTTP_REQUEST_TIMEOUT_SECS",
            NumericEnvRequirement::NonZeroU64,
        ),
        (
            "MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS",
            NumericEnvRequirement::NonZeroU64,
        ),
        (
            "MODELPORT_HTTP_MAX_RESPONSE_BYTES",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_HTTP_SSE_MAX_LINE_BYTES",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_HTTP_SSE_MAX_EVENT_BYTES",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_HTTP_SSE_MAX_STREAM_BYTES",
            NumericEnvRequirement::NonZeroUsize,
        ),
        (
            "MODELPORT_ADMIN_SESSION_TTL_SECONDS",
            NumericEnvRequirement::NonZeroU64,
        ),
    ] {
        if let Some(value) = env_value(name) {
            validate_numeric_env_value(name, &value, requirement, issues);
        }
    }
}

fn validate_numeric_env_value(
    name: &str,
    value: &str,
    requirement: NumericEnvRequirement,
    issues: &mut Vec<ConfigIssue>,
) {
    let value = value.trim();
    if value.is_empty() {
        issues.push(ConfigIssue::error(format!("{name} must not be empty")));
        return;
    }

    match requirement {
        NumericEnvRequirement::NonZeroU64 => match value.parse::<u64>() {
            Ok(parsed) if parsed > 0 => {}
            Ok(_) => issues.push(ConfigIssue::error(format!("{name} must be greater than 0"))),
            Err(_) => issues.push(ConfigIssue::error(format!(
                "{name} must be an unsigned integer"
            ))),
        },
        NumericEnvRequirement::NonZeroUsize => match value.parse::<usize>() {
            Ok(parsed) if parsed > 0 => {}
            Ok(_) => issues.push(ConfigIssue::error(format!("{name} must be greater than 0"))),
            Err(_) => issues.push(ConfigIssue::error(format!(
                "{name} must be an unsigned integer"
            ))),
        },
        NumericEnvRequirement::U32 => {
            if value.parse::<u32>().is_err() {
                issues.push(ConfigIssue::error(format!(
                    "{name} must be an unsigned 32-bit integer"
                )));
            }
        }
    }
}

fn service_env_value(name: &str) -> Option<String> {
    env_value(name)
}

fn env_file_values() -> HashMap<String, String> {
    let Some(path) = env_file_path() else {
        return HashMap::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    raw.lines().filter_map(parse_env_line).collect()
}

fn env_file_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("MODELPORT_ENV_FILE") {
        return Some(PathBuf::from(path));
    }

    let path = env::current_dir().ok()?.join(".env");
    path.exists().then_some(path)
}

fn parse_env_line(raw: &str) -> Option<(String, String)> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
    let index = line.find('=')?;
    let key = line[..index].trim();
    if key.is_empty() {
        return None;
    }
    let value = strip_env_quotes(line[index + 1..].trim()).to_owned();
    Some((key.to_owned(), value))
}

fn strip_env_quotes(value: &str) -> &str {
    if value.len() < 2 {
        return value;
    }
    let Some(first) = value.chars().next() else {
        return value;
    };
    let Some(last) = value.chars().last() else {
        return value;
    };
    if matches!(first, '\'' | '"') && first == last {
        let start = first.len_utf8();
        let end = value.len() - last.len_utf8();
        &value[start..end]
    } else {
        value
    }
}

fn first_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| env_value(name))
}

fn first_env_owned(names: &[String]) -> Option<String> {
    names.iter().find_map(|name| env_value(name))
}

pub(crate) fn env_flag(name: &str) -> bool {
    env_value(name)
        .map(|value| {
            matches!(
                value.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(false)
}

fn env_bool(name: &str, default: bool) -> bool {
    env_value(name)
        .map(|value| match value.as_str() {
            "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON" => true,
            "0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF" => false,
            _ => default,
        })
        .unwrap_or(default)
}

fn env_key_fragment(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch == '-' {
                '_'
            } else {
                ch.to_ascii_uppercase()
            }
        })
        .collect()
}

fn config_path() -> PathBuf {
    if let Some(path) = service_env_value("MODELPORT_CONFIG") {
        return PathBuf::from(path);
    }

    let home = env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".config/modelport/config.toml")
}

fn resolve_bind(value: Option<String>) -> Result<SocketAddr, AppError> {
    value
        .unwrap_or_else(|| "127.0.0.1:17878".to_owned())
        .parse()
        .map_err(|err| AppError::Config(format!("invalid bind address: {err}")))
}

fn resolve_usize_env(value: Option<usize>, env_name: &str, default: usize) -> usize {
    value
        .or_else(|| service_env_value(env_name).and_then(|value| value.parse::<usize>().ok()))
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn validate_provider(
    id: &str,
    provider: &ProviderConfig,
    is_default_provider: bool,
    seen_models: &mut HashMap<String, String>,
    issues: &mut Vec<ConfigIssue>,
) {
    if id.trim().is_empty() {
        issues.push(ConfigIssue::error("provider id cannot be empty"));
    }
    if provider.display_name.trim().is_empty() {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` display_name cannot be empty"
        )));
    }
    if provider.base_url.trim().is_empty() {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` base_url cannot be empty"
        )));
    } else if !provider.base_url.starts_with("http://")
        && !provider.base_url.starts_with("https://")
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` base_url must start with http:// or https://"
        )));
    } else if provider.base_url.contains(char::is_whitespace) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` base_url contains whitespace"
        )));
    }

    if provider.base_url.ends_with("/chat/completions") || provider.base_url.ends_with("/messages")
    {
        issues.push(ConfigIssue::warning(format!(
            "provider `{id}` base_url looks like a full endpoint; configure the API base URL instead"
        )));
    }
    if let Err(err) = validate_provider_base_url_policy(
        id,
        &provider.base_url,
        env_flag("MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS"),
    ) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` base_url is not allowed: {err}"
        )));
    }

    if provider.api_key_required && provider.api_key.is_none() {
        let name = provider
            .api_key_env
            .as_deref()
            .unwrap_or("<provider api key env>");
        if is_default_provider {
            issues.push(ConfigIssue::error(format!(
                "default provider `{id}` requires API key env `{name}`"
            )));
        } else {
            issues.push(ConfigIssue::warning(format!(
                "provider `{id}` requires API key env `{name}` and will fail if selected"
            )));
        }
    }

    if provider
        .api_key
        .as_deref()
        .is_some_and(is_placeholder_value)
    {
        let name = provider
            .api_key_env
            .as_deref()
            .unwrap_or("provider API key");
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` API key `{name}` is still a placeholder"
        )));
    }

    if provider.default_model.trim().is_empty() {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` default_model cannot be empty"
        )));
    }

    if provider.models.iter().any(|model| model.trim().is_empty()) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` models cannot contain empty values"
        )));
    }

    if !provider.models.is_empty()
        && !provider.models.contains(&provider.default_model)
        && !provider.passthrough_unknown_models
    {
        issues.push(ConfigIssue::warning(format!(
            "provider `{id}` default_model `{}` is not listed in models",
            provider.default_model
        )));
    }

    for model in &provider.models {
        seen_models
            .entry(model.clone())
            .or_insert_with(|| id.to_owned());
    }

    if provider
        .model_prefixes
        .iter()
        .any(|prefix| prefix.trim().is_empty())
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` model_prefixes cannot contain empty values"
        )));
    }

    if provider.fidelity_mode == FidelityMode::Strict
        && (provider.deduplicate_stream_text || provider.buffer_stream_text)
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` cannot use fidelity_mode=strict together with stream text rewriting"
        )));
    }

    if !provider.tool_use.supported
        && (provider.tool_use.tool_choice || provider.tool_use.parallel_tool_calls)
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` cannot enable tool_choice or parallel_tool_calls when tool_use.supported=false"
        )));
    }

    if provider.tool_use.repair_invalid_arguments
        && (provider.protocol != ProviderProtocol::OpenaiCompat
            || provider.tool_use.response_validation != ToolResponseValidation::Strict)
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` can enable tool_use.repair_invalid_arguments only for an OpenAI-compatible provider with strict response validation"
        )));
    }

    if provider.protocol == ProviderProtocol::Anthropic
        && provider.tool_use.streaming_arguments != ToolArgumentMode::Native
    {
        issues.push(ConfigIssue::warning(format!(
            "provider `{id}` uses Anthropic protocol; tool_use.streaming_arguments is normally native"
        )));
    }

    if provider.reasoning.mode == ReasoningMode::LlamaCpp
        && provider.protocol != ProviderProtocol::OpenaiCompat
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` reasoning.mode=llama_cpp requires protocol=openai-compat"
        )));
    }
    if provider.reasoning.mode == ReasoningMode::None
        && (provider.reasoning.default_budget_tokens.is_some()
            || !provider.reasoning.model_budget_tokens.is_empty())
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` reasoning defaults require a non-none reasoning.mode"
        )));
    }
    for model in provider.reasoning.model_effort.keys() {
        if model.trim().is_empty() {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` reasoning.model_effort requires non-empty model names"
            )));
        }
    }

    if let Err(reason) = validate_model_profile_override(&provider.model_profile_defaults) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` model_profile_defaults is invalid: {reason}"
        )));
    }

    for (model, profile_override) in &provider.model_profiles {
        if model.trim().is_empty() {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` model_profiles cannot contain an empty model name"
            )));
            continue;
        }
        if !provider.models.contains(model) {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` model profile `{model}` is not present in models"
            )));
        }
        if let Err(reason) = validate_model_profile_override(profile_override) {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` model profile `{model}` is invalid: {reason}"
            )));
        }
    }

    for model in &provider.models {
        let effective = provider.model_profile(id, model);
        if effective
            .max_output_tokens
            .zip(effective.context_window)
            .is_some_and(|(output, context)| output > context)
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` model profile `{model}` max_output_tokens exceeds context_window after merging"
            )));
        }
        if let Some(default_effort) = effective.default_reasoning_effort
            && !effective.reasoning_efforts.is_empty()
            && !effective.reasoning_efforts.contains(&default_effort)
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` model profile `{model}` default reasoning effort is not listed in reasoning_efforts"
            )));
        }
        if !effective.tool_use.is_supported()
            && (effective.tool_choice.is_supported()
                || effective.parallel_tool_calls.is_supported()
                || effective.strict_tool_schema.is_supported())
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` model profile `{model}` cannot support advanced tool features while tool_use is not supported"
            )));
        }
        if effective.reasoning.is_supported()
            && effective.reasoning_dialect == crate::model_catalog::ReasoningDialect::None
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` model profile `{model}` supports reasoning but has no reasoning_dialect"
            )));
        }
    }

    for (name, value) in &provider.static_headers {
        if let Err(reason) = validate_provider_static_header(name, value) {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` static header `{name}` is invalid: {reason}"
            )));
        }
    }
    for (field, value) in [
        ("request_timeout_ms", provider.request_timeout_ms),
        ("stream_idle_timeout_ms", provider.stream_idle_timeout_ms),
    ] {
        if value.is_some_and(|value| value == 0 || value > MAX_PROVIDER_TIMER_MS) {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` {field} must be between 1 and {MAX_PROVIDER_TIMER_MS}"
            )));
        }
    }
    if !(1..=5).contains(&provider.retry.max_attempts) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` retry.max_attempts must be between 1 and 5"
        )));
    }
    if provider.retry.initial_delay_ms == 0
        || provider.retry.max_delay_ms == 0
        || provider.retry.initial_delay_ms > provider.retry.max_delay_ms
        || provider.retry.max_delay_ms > 60_000
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` retry delays must be positive, initial_delay_ms <= max_delay_ms, and max_delay_ms <= 60000"
        )));
    }
    if !provider.retry.jitter_ratio.is_finite()
        || !(0.0..=1.0).contains(&provider.retry.jitter_ratio)
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` retry.jitter_ratio must be between 0 and 1"
        )));
    }
    if provider.reasoning.default_budget_tokens == Some(0) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` reasoning.default_budget_tokens must be positive"
        )));
    }
    for (model, budget) in &provider.reasoning.model_budget_tokens {
        if model.trim().is_empty() || *budget == 0 {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` reasoning.model_budget_tokens requires non-empty model names and positive budgets"
            )));
        }
    }

    if provider.sampling.mode == SamplingMode::LlamaCpp
        && provider.protocol != ProviderProtocol::OpenaiCompat
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` sampling.mode=llama_cpp requires protocol=openai-compat"
        )));
    }
    if provider.sampling.mode == SamplingMode::None && !provider.sampling.profiles.is_empty() {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` sampling profiles require a non-none sampling.mode"
        )));
    }
    for (model, profile) in &provider.sampling.profiles {
        if model.trim().is_empty() {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` sampling.profiles requires non-empty model names"
            )));
        }
        if profile
            .temperature
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` sampling profile `{model}` temperature must be between 0 and 2"
            )));
        }
        for (field, value) in [("top_p", profile.top_p), ("min_p", profile.min_p)] {
            if value.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
                issues.push(ConfigIssue::error(format!(
                    "provider `{id}` sampling profile `{model}` {field} must be between 0 and 1"
                )));
            }
        }
        if profile.top_k == Some(0) {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` sampling profile `{model}` top_k must be positive"
            )));
        }
        if profile
            .presence_penalty
            .is_some_and(|value| !value.is_finite() || !(-2.0..=2.0).contains(&value))
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` sampling profile `{model}` presence_penalty must be between -2 and 2"
            )));
        }
        if profile
            .repeat_penalty
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` sampling profile `{model}` repeat_penalty must be positive"
            )));
        }
    }

    if provider.token_counting.mode == TokenCountingMode::None
        && (provider.token_counting.context_tokens.is_some()
            || provider
                .token_counting
                .recommended_reasoning_input_tokens
                .is_some()
            || !provider
                .token_counting
                .model_recommended_input_tokens
                .is_empty()
            || provider.token_counting.max_output_tokens.is_some()
            || !provider.token_counting.model_max_output_tokens.is_empty())
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` context admission requires token_counting.mode=anthropic"
        )));
    }
    if provider.token_counting.context_tokens == Some(0) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` token_counting.context_tokens must be positive"
        )));
    }
    if provider.token_counting.recommended_reasoning_input_tokens == Some(0) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` token_counting.recommended_reasoning_input_tokens must be positive"
        )));
    }
    for (model, limit) in &provider.token_counting.model_recommended_input_tokens {
        if model.trim().is_empty() || *limit == 0 {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` token_counting.model_recommended_input_tokens requires non-empty model names and positive limits"
            )));
        }
        if provider
            .token_counting
            .context_tokens
            .is_some_and(|context| *limit > context)
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` recommended input limit for `{model}` cannot exceed token_counting.context_tokens"
            )));
        }
    }
    if provider.token_counting.max_output_tokens == Some(0) {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` token_counting.max_output_tokens must be positive"
        )));
    }
    if let (Some(maximum), Some(context)) = (
        provider.token_counting.max_output_tokens,
        provider.token_counting.context_tokens,
    ) && maximum > context
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` token_counting.max_output_tokens cannot exceed context_tokens"
        )));
    }
    for (model, limit) in &provider.token_counting.model_max_output_tokens {
        if model.trim().is_empty() || *limit == 0 {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` token_counting.model_max_output_tokens requires non-empty model names and positive limits"
            )));
        }
        if provider
            .token_counting
            .max_output_tokens
            .is_some_and(|maximum| *limit > maximum)
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` output limit for `{model}` cannot exceed token_counting.max_output_tokens"
            )));
        }
        if provider
            .token_counting
            .context_tokens
            .is_some_and(|context| *limit > context)
        {
            issues.push(ConfigIssue::error(format!(
                "provider `{id}` output limit for `{model}` cannot exceed token_counting.context_tokens"
            )));
        }
    }
    if let (Some(recommended), Some(context)) = (
        provider.token_counting.recommended_reasoning_input_tokens,
        provider.token_counting.context_tokens,
    ) && recommended > context
    {
        issues.push(ConfigIssue::error(format!(
            "provider `{id}` recommended reasoning input cannot exceed context_tokens"
        )));
    }

    if let Some(pricing) = provider.pricing {
        for (field, value) in [
            ("input_per_million", pricing.input_per_million),
            ("output_per_million", pricing.output_per_million),
            ("cache_write_per_million", pricing.cache_write_per_million),
            ("cache_read_per_million", pricing.cache_read_per_million),
        ] {
            if !value.is_finite() || value < 0.0 {
                issues.push(ConfigIssue::error(format!(
                    "provider `{id}` pricing.{field} must be a finite non-negative number"
                )));
            }
        }
    }
    for (model, card) in &provider.model_pricing {
        if let Err(message) =
            crate::pricing::validate_model_pricing_card(model, &provider.models, card)
        {
            issues.push(ConfigIssue::error(format!("provider `{id}` {message}")));
        }
    }
}

pub(crate) fn validate_provider_static_header(name: &str, value: &str) -> Result<(), &'static str> {
    let normalized = name.trim().to_ascii_lowercase();
    if normalized.is_empty() || HeaderName::from_bytes(normalized.as_bytes()).is_err() {
        return Err("header name is not a valid HTTP field name");
    }
    if HeaderValue::from_str(value).is_err() {
        return Err("header value contains invalid bytes");
    }
    let has_sensitive_segment = normalized.split(['-', '_']).any(|segment| {
        matches!(
            segment,
            "auth" | "authorization" | "token" | "secret" | "credential" | "cookie" | "signature"
        )
    });
    if has_sensitive_segment
        || normalized.contains("api-key")
        || normalized.contains("api_key")
        || matches!(
            normalized.as_str(),
            "authorization"
                | "proxy-authorization"
                | "proxy-authenticate"
                | "www-authenticate"
                | "x-api-key"
                | "cookie"
                | "set-cookie"
                | "host"
                | "accept"
                | "accept-encoding"
                | "content-encoding"
                | "content-type"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "keep-alive"
                | "proxy-connection"
                | "te"
                | "trailer"
                | "upgrade"
                | "forwarded"
                | "via"
                | "cf-connecting-ip"
                | "true-client-ip"
                | "x-real-ip"
                | "request-id"
                | "x-request-id"
                | "correlation-id"
                | "x-correlation-id"
                | "traceparent"
                | "tracestate"
                | "baggage"
                | "b3"
                | "sentry-trace"
                | "x-cloud-trace-context"
                | "grpc-trace-bin"
                | "user-agent"
                | "anthropic-version"
                | "anthropic-beta"
                | "openai-organization"
                | "openai-project"
        )
        || ["x-forwarded-", "x-b3-", "sec-", "x-modelport-"]
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
    {
        return Err("security, framing, authentication, and tracing headers are reserved");
    }
    Ok(())
}

fn validate_cpa_provider(
    provider_id: &str,
    provider: &ProviderConfig,
    issues: &mut Vec<ConfigIssue>,
) {
    let expected_protocol = match provider_id {
        "cpa_codex" => ProviderProtocol::OpenaiCompat,
        "cpa_claude" => ProviderProtocol::Anthropic,
        _ => return,
    };

    if provider.protocol != expected_protocol {
        issues.push(ConfigIssue::error(format!(
            "provider `{provider_id}` must use protocol={}",
            match expected_protocol {
                ProviderProtocol::OpenaiCompat => "openai-compat",
                ProviderProtocol::Anthropic => "anthropic",
            }
        )));
    }
    if !provider.api_key_required || provider.api_key_env.is_none() {
        issues.push(ConfigIssue::error(format!(
            "provider `{provider_id}` must require a dedicated CPA client API key"
        )));
    }
    if provider.models.is_empty() {
        issues.push(ConfigIssue::error(format!(
            "provider `{provider_id}` requires an explicit model allowlist"
        )));
    }
    if provider.passthrough_unknown_models {
        issues.push(ConfigIssue::error(format!(
            "provider `{provider_id}` cannot enable passthrough_unknown_models"
        )));
    }
    if !provider.model_prefixes.is_empty() {
        issues.push(ConfigIssue::error(format!(
            "provider `{provider_id}` cannot claim model prefixes; use provider-qualified model names"
        )));
    }

    let Ok(url) = Url::parse(&provider.base_url) else {
        return;
    };
    let path = url.path().trim_end_matches('/');
    if path.contains("/v0/management") {
        issues.push(ConfigIssue::error(format!(
            "provider `{provider_id}` must use CPA's client API, not its management API"
        )));
    }
    match provider_id {
        "cpa_codex" if !path.ends_with("/v1") => {
            issues.push(ConfigIssue::error(
                "provider `cpa_codex` base_url must end with `/v1`".to_owned(),
            ));
        }
        "cpa_claude" if path.ends_with("/v1") => {
            issues.push(ConfigIssue::error(
                "provider `cpa_claude` base_url must omit `/v1`; ModelPort appends `/v1/messages`"
                    .to_owned(),
            ));
        }
        _ => {}
    }
}

fn validate_provider_base_url_policy(
    provider_id: &str,
    base_url: &str,
    allow_private_provider_urls: bool,
) -> Result<(), String> {
    let url = Url::parse(base_url).map_err(|err| format!("invalid URL: {err}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("URL scheme must be http or https".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL userinfo is not allowed".to_owned());
    }
    if url.fragment().is_some() {
        return Err("URL fragments are not allowed".to_owned());
    }
    if url.query().is_some() {
        return Err(
            "URL query parameters are not allowed; provider credentials must use headers"
                .to_owned(),
        );
    }

    let Some(host) = url.host_str() else {
        return Err("URL host is required".to_owned());
    };
    let host = host.trim_matches(['[', ']']).trim_end_matches('.');
    if host.is_empty() {
        return Err("URL host is required".to_owned());
    }

    let private_literal_host = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(private_or_metadata_ip);
    if url.scheme() == "http"
        && !provider_allows_loopback_base_url(provider_id)
        && !provider_allows_trusted_internal_http(provider_id, host)
        && (!allow_private_provider_urls || !private_literal_host)
        && !env_flag("MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP")
    {
        return Err(
            "remote provider URLs must use https; set MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1 only for a trusted internal HTTP upstream"
                .to_owned(),
        );
    }

    if allow_private_provider_urls {
        return Ok(());
    }

    if host.eq_ignore_ascii_case("localhost") {
        if provider_allows_loopback_base_url(provider_id)
            || provider_allows_trusted_internal_http(provider_id, host)
        {
            return Ok(());
        }
        return Err(
            "localhost base URLs are only allowed for local/custom/CPA providers".to_owned(),
        );
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip.is_loopback()
            && (provider_allows_loopback_base_url(provider_id)
                || provider_allows_trusted_internal_http(provider_id, host))
        {
            return Ok(());
        }
        if private_or_metadata_ip(ip) {
            return Err(format!(
                "non-public or special-use IP `{ip}` requires MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS=1"
            ));
        }
    }

    Ok(())
}

fn openai_base_url_targets_modelport_listener(base_url: &str, bind_addr: SocketAddr) -> bool {
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    if url.port_or_known_default() != Some(bind_addr.port()) {
        return false;
    }
    if url.path().trim_matches('/') != "v1" {
        return false;
    }

    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']).trim_end_matches('.');
    if host.eq_ignore_ascii_case("localhost") {
        return bind_addr.ip().is_loopback() || bind_addr.ip().is_unspecified();
    }

    let Ok(host_ip) = host.parse::<IpAddr>() else {
        return false;
    };
    if !host_ip.is_loopback() && !host_ip.is_unspecified() {
        return false;
    }
    if bind_addr.ip().is_unspecified() {
        return true;
    }

    match (host_ip, bind_addr.ip()) {
        (IpAddr::V4(host), IpAddr::V4(bind)) => host.is_unspecified() || host == bind,
        (IpAddr::V6(host), IpAddr::V6(bind)) => host.is_unspecified() || host == bind,
        _ => false,
    }
}

fn provider_allows_loopback_base_url(provider_id: &str) -> bool {
    provider_id.starts_with("local_")
        || matches!(
            provider_id,
            "custom" | "ollama" | "local_sglang" | "local_vllm" | "local_llamacpp"
        )
}

fn provider_allows_trusted_internal_http(provider_id: &str, host: &str) -> bool {
    provider_id.starts_with("cpa_")
        && (host.eq_ignore_ascii_case("localhost")
            || host.eq_ignore_ascii_case("host.docker.internal")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
            || !host.contains('.'))
}

fn private_or_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => private_or_metadata_ipv4(ip),
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or_else(|| special_use_ipv6(ip), private_or_metadata_ipv4),
    }
}

fn private_or_metadata_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let [first, second, third, _] = ip.octets();
    first == 0
        || first == 10
        || (first == 100 && (64..=127).contains(&second))
        || first == 127
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 0 && third == 0)
        || (first == 192 && second == 0 && third == 2)
        || (first == 192 && second == 88 && third == 99)
        || (first == 192 && second == 168)
        || (first == 198 && matches!(second, 18 | 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113)
        || first >= 224
}

fn special_use_ipv6(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    let global_unicast = (segments[0] & 0xe000) == 0x2000;
    let ietf_special_use = segments[0] == 0x2001 && (segments[1] & 0xfe00) == 0;
    let documentation = (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || (segments[0] == 0x3fff && (segments[1] & 0xf000) == 0);

    // IPv4-compatible, NAT64 and other translation prefixes can otherwise
    // smuggle a private IPv4 target through an apparently IPv6 DNS answer.
    let ipv4_compatible = segments[..6].iter().all(|segment| *segment == 0);
    let nat64_well_known = segments[0] == 0x0064
        && segments[1] == 0xff9b
        && segments[2..6].iter().all(|segment| *segment == 0);
    let nat64_local = segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 1;

    !global_unicast
        || ietf_special_use
        || documentation
        || segments[0] == 0x2002
        || segments[0] == 0x5f00
        || ipv4_compatible
        || nat64_well_known
        || nat64_local
}

pub(crate) fn is_placeholder_value(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || value.starts_with("replace-with-")
        || value.contains("placeholder")
        || value.contains("your-")
        || value == "changeme"
        || value == "change-me"
}

fn default_auth_token() -> Option<String> {
    env_value("MODELPORT_AUTH_TOKEN").or_else(|| env_value("ANTHROPIC_AUTH_TOKEN"))
}

fn require_auth_token(auth_token: Option<String>) -> Result<Option<String>, AppError> {
    if auth_token.is_some() || env_flag("MODELPORT_ALLOW_NO_AUTH") {
        return Ok(auth_token);
    }

    Err(AppError::Config(
        "MODELPORT_AUTH_TOKEN or ANTHROPIC_AUTH_TOKEN is required; set MODELPORT_ALLOW_NO_AUTH=1 only for isolated local testing".to_owned(),
    ))
}

fn default_aliases() -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    for name in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ] {
        if let Some(model) = env_value(name) {
            if model.starts_with("deepseek-") {
                aliases.insert(model, "deepseek".to_owned());
            } else if model.starts_with("mimo-") {
                aliases.insert(model, "mimo".to_owned());
            }
        }
    }
    aliases
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AppConfig {
        let mimo = ProviderConfig {
            display_name: "Mimo".to_owned(),
            protocol: ProviderProtocol::OpenaiCompat,
            base_url: "https://api.xiaomimimo.com/v1".to_owned(),
            api_key_env: Some("MIMO_OPENAI_API_KEY".to_owned()),
            api_key: Some("test".to_owned()),
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
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: HashMap::new(),
            trust_upstream_cost: false,
        };
        let openrouter = ProviderConfig {
            display_name: "OpenRouter".to_owned(),
            protocol: ProviderProtocol::OpenaiCompat,
            base_url: "https://openrouter.ai/api/v1".to_owned(),
            api_key_env: Some("OPENROUTER_API_KEY".to_owned()),
            api_key: Some("test".to_owned()),
            api_key_required: true,
            default_model: "openrouter/auto".to_owned(),
            models: vec!["openrouter/auto".to_owned()],
            model_prefixes: vec!["anthropic/".to_owned()],
            passthrough_unknown_models: true,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::BestEffort,
            tool_use: ToolUseConfig::default_for_provider(
                "openrouter",
                ProviderProtocol::OpenaiCompat,
                false,
            ),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: HashMap::new(),
            trust_upstream_cost: true,
        };

        AppConfig {
            bind_addr: "127.0.0.1:17878".parse().unwrap(),
            max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
            max_concurrent_requests: DEFAULT_MAX_CONCURRENT_REQUESTS,
            auth_token: None,
            default_provider: "mimo".to_owned(),
            provider_order: vec!["mimo".to_owned(), "openrouter".to_owned()],
            providers: HashMap::from([
                ("mimo".to_owned(), mimo),
                ("openrouter".to_owned(), openrouter),
            ]),
            aliases: HashMap::from([(
                "sonnet-via-router".to_owned(),
                "openrouter:anthropic/claude-sonnet-4".to_owned(),
            )]),
            smart_routing: SmartRoutingConfig::default(),
            runtime_adapters: BTreeMap::new(),
        }
    }

    #[test]
    fn runtime_adapter_registry_loads_env_secret_with_defaults_and_redacts_debug() {
        const CREDENTIAL_ENV: &str = "MODELPORT_TEST_RUNTIME_ADAPTER_TOKEN_27";
        // SAFETY: this test owns a unique process variable that no other test reads.
        unsafe { env::set_var(CREDENTIAL_ENV, "adapter-secret-never-log") };
        let file: FileConfig = toml::from_str(&format!(
            r#"
            [runtime_adapters.edge_1]
            base_url = "http://127.0.0.1:19090"
            bearer_token_env = "{CREDENTIAL_ENV}"
            "#
        ))
        .unwrap();

        let registry = load_runtime_adapters(file.runtime_adapters.unwrap()).unwrap();
        // SAFETY: remove the unique variable after the synchronous load boundary.
        unsafe { env::remove_var(CREDENTIAL_ENV) };
        let adapter = &registry["edge_1"];
        assert_eq!(adapter.client_config.adapter_id(), "edge_1");
        assert_eq!(adapter.credential_env, CREDENTIAL_ENV);
        assert_eq!(adapter.poll_interval, Duration::from_secs(30));
        assert_eq!(adapter.stale_after, Duration::from_secs(90));
        let debug = format!("{registry:?}");
        assert!(!debug.contains("adapter-secret-never-log"));
        assert!(debug.contains("[redacted]"));
    }

    #[test]
    fn disabled_runtime_adapter_is_inert_without_endpoint_or_secret() {
        let file: FileConfig = toml::from_str(
            r#"
            [runtime_adapters.future]
            enabled = false
            "#,
        )
        .unwrap();

        assert!(
            load_runtime_adapters(file.runtime_adapters.unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn runtime_adapter_registry_rejects_inline_secret_and_invalid_policy() {
        let inline = toml::from_str::<FileConfig>(
            r#"
            [runtime_adapters.edge]
            base_url = "https://adapter.example"
            bearer_token = "must-not-be-accepted"
            "#,
        );
        assert!(inline.is_err());

        const CREDENTIAL_ENV: &str = "MODELPORT_TEST_RUNTIME_ADAPTER_POLICY_TOKEN_27";
        // SAFETY: this test owns a unique process variable that no other test reads.
        unsafe { env::set_var(CREDENTIAL_ENV, "valid-test-token") };
        let file: FileConfig = toml::from_str(&format!(
            r#"
            [runtime_adapters.edge]
            base_url = "https://adapter.example"
            bearer_token_env = "{CREDENTIAL_ENV}"
            poll_interval_seconds = 60
            stale_after_seconds = 30
            "#
        ))
        .unwrap();
        let error = load_runtime_adapters(file.runtime_adapters.unwrap()).unwrap_err();
        unsafe { env::remove_var(CREDENTIAL_ENV) };
        assert!(
            error
                .to_string()
                .contains("cover at least one polling interval")
        );
        assert!(!error.to_string().contains("valid-test-token"));
    }

    fn test_cpa_provider(provider_id: &str) -> ProviderConfig {
        let (display_name, protocol, base_url, api_key_env, default_model, max_tokens_field) =
            match provider_id {
                "cpa_codex" => (
                    "CPA · OpenAI Codex",
                    ProviderProtocol::OpenaiCompat,
                    "http://127.0.0.1:8317/v1",
                    "CPA_CODEX_API_KEY",
                    "gpt-5.3-codex",
                    MaxTokensField::MaxCompletionTokens,
                ),
                "cpa_claude" => (
                    "CPA · Claude Code",
                    ProviderProtocol::Anthropic,
                    "http://127.0.0.1:8317",
                    "CPA_CLAUDE_API_KEY",
                    "claude-sonnet-4-6",
                    MaxTokensField::MaxTokens,
                ),
                _ => panic!("unsupported test CPA provider"),
            };

        ProviderConfig {
            display_name: display_name.to_owned(),
            protocol,
            base_url: base_url.to_owned(),
            api_key_env: Some(api_key_env.to_owned()),
            api_key: Some("test-cpa-client-key".to_owned()),
            api_key_required: true,
            default_model: default_model.to_owned(),
            models: vec![default_model.to_owned()],
            model_prefixes: Vec::new(),
            passthrough_unknown_models: false,
            max_tokens_field,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::BestEffort,
            tool_use: ToolUseConfig::default_for_provider(provider_id, protocol, false),
            model_profile_defaults: Default::default(),
            model_profiles: Default::default(),
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            static_headers: Default::default(),
            request_timeout_ms: None,
            stream_idle_timeout_ms: None,
            retry: Default::default(),
            pricing: None,
            model_pricing: HashMap::new(),
            trust_upstream_cost: false,
        }
    }

    #[test]
    fn unknown_client_model_uses_default_provider_model_when_default_does_not_passthrough() {
        let resolved = test_config().resolve("claude-sonnet-4").unwrap();

        assert_eq!(resolved.provider.display_name, "Mimo");
        assert_eq!(resolved.model, "mimo-v2.5-pro");
    }

    #[test]
    fn provider_model_selector_preserves_arbitrary_model_name() {
        let resolved = test_config()
            .resolve("openrouter:anthropic/claude-sonnet-4")
            .unwrap();

        assert_eq!(resolved.provider.display_name, "OpenRouter");
        assert_eq!(resolved.model, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn parses_local_openai_compatible_provider_from_toml() {
        let file: FileConfig = toml::from_str(
            r#"
            default_provider = "local_vllm"
            provider_order = ["local_vllm"]

            [server]
            bind = "127.0.0.1:17878"

            [providers.local_vllm]
            display_name = "Local vLLM"
            protocol = "openai-compat"
            base_url = "http://127.0.0.1:8000/v1"
            api_key_required = false
            default_model = "qwen2.5-coder"
            models = ["qwen2.5-coder"]
            passthrough_unknown_models = true
            max_tokens_field = "max_tokens"
            fidelity_mode = "strict"

            [providers.local_vllm.tool_use]
            parallel_tool_calls = false
            streaming_arguments = "best_effort"

            [providers.local_vllm.reasoning]
            mode = "llama_cpp"
            default_enabled = false
            model_enabled = { "qwen-fast" = false, "qwen-deep" = true }
            default_budget_tokens = 4096
            model_budget_tokens = { "qwen-fast" = 512, "qwen-deep" = 16384 }

            [providers.local_vllm.sampling]
            mode = "llama_cpp"

            [providers.local_vllm.sampling.profiles."qwen-fast"]
            temperature = 0.7
            top_p = 0.8
            top_k = 20
            min_p = 0.0
            presence_penalty = 1.5
            repeat_penalty = 1.0

            [providers.local_vllm.token_counting]
            mode = "anthropic"
            context_tokens = 131072
            recommended_reasoning_input_tokens = 94208
            model_recommended_input_tokens = { "qwen-fast" = 24576, "qwen-deep" = 94208 }
            max_output_tokens = 32768
            model_max_output_tokens = { "qwen-fast" = 4096, "qwen-deep" = 32768 }

            [providers.local_vllm.pricing]
            input_per_million = 0
            output_per_million = 0
            cache_write_per_million = 0
            cache_read_per_million = 0
            "#,
        )
        .unwrap();

        let provider = file
            .providers
            .as_ref()
            .and_then(|providers| providers.get("local_vllm"))
            .unwrap();

        assert_eq!(provider.protocol, ProviderProtocol::OpenaiCompat);
        assert_eq!(
            provider.base_url.as_deref(),
            Some("http://127.0.0.1:8000/v1")
        );
        assert_eq!(provider.api_key_required, Some(false));
        assert_eq!(provider.max_tokens_field, Some(MaxTokensField::MaxTokens));
        assert_eq!(provider.fidelity_mode, Some(FidelityMode::Strict));
        assert_eq!(
            provider.token_counting,
            Some(TokenCountingConfig {
                mode: TokenCountingMode::Anthropic,
                context_tokens: Some(131072),
                recommended_reasoning_input_tokens: Some(94208),
                model_recommended_input_tokens: HashMap::from([
                    ("qwen-fast".to_owned(), 24576),
                    ("qwen-deep".to_owned(), 94208),
                ]),
                max_output_tokens: Some(32768),
                model_max_output_tokens: HashMap::from([
                    ("qwen-fast".to_owned(), 4096),
                    ("qwen-deep".to_owned(), 32768),
                ]),
            })
        );
        assert_eq!(
            provider.reasoning,
            Some(ReasoningConfig {
                mode: ReasoningMode::LlamaCpp,
                default_enabled: Some(false),
                model_enabled: HashMap::from([
                    ("qwen-fast".to_owned(), false),
                    ("qwen-deep".to_owned(), true),
                ]),
                default_effort: None,
                model_effort: HashMap::new(),
                default_budget_tokens: Some(4096),
                model_budget_tokens: HashMap::from([
                    ("qwen-fast".to_owned(), 512),
                    ("qwen-deep".to_owned(), 16384),
                ]),
            })
        );
        assert_eq!(
            provider.sampling,
            Some(SamplingConfig {
                mode: SamplingMode::LlamaCpp,
                profiles: HashMap::from([(
                    "qwen-fast".to_owned(),
                    SamplingProfile {
                        temperature: Some(0.7),
                        top_p: Some(0.8),
                        top_k: Some(20),
                        min_p: Some(0.0),
                        presence_penalty: Some(1.5),
                        repeat_penalty: Some(1.0),
                    },
                )]),
            })
        );
        assert_eq!(
            provider.pricing,
            Some(ModelPricing {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_write_per_million: 0.0,
                cache_read_per_million: 0.0,
            })
        );
        assert_eq!(
            provider.tool_use,
            Some(ToolUseConfig {
                supported: true,
                tool_choice: true,
                parallel_tool_calls: false,
                streaming_arguments: ToolArgumentMode::BestEffort,
                response_validation: ToolResponseValidation::BestEffort,
                repair_invalid_arguments: false,
            })
        );
    }

    #[test]
    fn provider_url_policy_blocks_metadata_ip_for_remote_provider() {
        let err = validate_provider_base_url_for_request(
            "deepseek",
            "https://169.254.169.254/latest/meta-data",
            false,
        )
        .unwrap_err();

        assert!(err.to_string().contains("non-public"));
    }

    #[test]
    fn provider_url_policy_blocks_ipv4_mapped_loopback_for_remote_provider() {
        let err = validate_provider_base_url_for_request(
            "deepseek",
            "https://[::ffff:127.0.0.1]:8443/v1",
            false,
        )
        .unwrap_err();

        assert!(err.to_string().contains("non-public"));
    }

    #[test]
    fn provider_dns_and_literal_url_policies_reject_special_use_addresses() {
        let special_use_addresses = [
            "0.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "100.100.100.200",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.0.0.9",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
            "64:ff9b:1::a00:1",
            "100::1",
            "2001::1",
            "2001:db8::1",
            "2002:a00:1::",
            "3fff::1",
            "5f00::1",
            "fc00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
        ];
        for address in special_use_addresses {
            let ip = address.parse::<IpAddr>().unwrap();
            assert!(
                matches!(
                    validate_provider_resolved_ips([ip]),
                    Err(AppError::Forbidden(_))
                ),
                "DNS policy accepted special-use address {address}"
            );

            let url = match ip {
                IpAddr::V4(_) => format!("https://{address}/v1"),
                IpAddr::V6(_) => format!("https://[{address}]/v1"),
            };
            assert!(
                validate_provider_base_url_for_request("deepseek", &url, false).is_err(),
                "literal URL policy accepted special-use address {address}"
            );
        }

        validate_provider_resolved_ips([
            "8.8.8.8".parse().unwrap(),
            "192.31.196.1".parse().unwrap(),
            "2606:4700:4700::1111".parse().unwrap(),
        ])
        .unwrap();
    }

    #[test]
    fn provider_url_policy_allows_ipv4_mapped_public_address() {
        validate_provider_base_url_for_request(
            "deepseek",
            "https://[::ffff:8.8.8.8]:8443/v1",
            false,
        )
        .unwrap();
    }

    #[test]
    fn provider_url_policy_rejects_plain_http_for_remote_provider() {
        let err = validate_provider_base_url_for_request(
            "deepseek",
            "http://api.deepseek.com/anthropic",
            false,
        )
        .unwrap_err();

        assert!(err.to_string().contains("must use https"));
    }

    #[test]
    fn provider_url_policy_allows_loopback_for_local_provider() {
        validate_provider_base_url_for_request("local_vllm", "http://127.0.0.1:8000/v1", false)
            .unwrap();
    }

    #[test]
    fn provider_url_policy_allows_only_trusted_internal_http_for_cpa() {
        for base_url in [
            "http://127.0.0.1:8317/v1",
            "http://cpa:8317/v1",
            "http://host.docker.internal:8317/v1",
        ] {
            validate_provider_base_url_for_request("cpa_codex", base_url, false).unwrap();
        }

        let err = validate_provider_base_url_for_request(
            "cpa_codex",
            "http://cpa.example.com:8317/v1",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("must use https"));
    }

    #[test]
    fn provider_url_policy_blocks_userinfo() {
        let err = validate_provider_base_url_for_request(
            "deepseek",
            "https://token:secret@api.deepseek.com/anthropic",
            false,
        )
        .unwrap_err();

        assert!(err.to_string().contains("userinfo"));
    }

    #[test]
    fn provider_url_policy_blocks_query_credentials() {
        let err = validate_provider_base_url_for_request(
            "deepseek",
            "https://api.deepseek.com/anthropic?api_key=secret",
            false,
        )
        .unwrap_err();

        assert!(err.to_string().contains("query parameters"));
    }

    #[test]
    fn runtime_guardrail_env_validation_rejects_bad_values() {
        let mut issues = Vec::new();

        validate_numeric_env_value(
            "MODELPORT_MAX_MESSAGES",
            "0",
            NumericEnvRequirement::NonZeroUsize,
            &mut issues,
        );
        validate_numeric_env_value(
            "MODELPORT_RATE_LIMIT_API_KEY_PER_MINUTE",
            "-1",
            NumericEnvRequirement::U32,
            &mut issues,
        );
        validate_numeric_env_value(
            "MODELPORT_RATE_LIMIT_WINDOW_SECONDS",
            "abc",
            NumericEnvRequirement::NonZeroU64,
            &mut issues,
        );

        assert_eq!(issues.len(), 3);
        assert!(
            issues
                .iter()
                .all(|issue| issue.severity == ConfigIssueSeverity::Error)
        );
    }

    #[test]
    fn parses_env_file_lines_for_runtime_reload() {
        assert_eq!(
            parse_env_line("export MIMO_MODEL=\"mimo-v2.5-pro\""),
            Some(("MIMO_MODEL".to_owned(), "mimo-v2.5-pro".to_owned()))
        );
        assert_eq!(
            parse_env_line("DEEPSEEK_API_KEY='sk-test'"),
            Some(("DEEPSEEK_API_KEY".to_owned(), "sk-test".to_owned()))
        );
        assert_eq!(parse_env_line("# comment"), None);
    }

    #[test]
    fn process_environment_overrides_provider_env_file_value() {
        assert_eq!(
            select_env_value(
                Some("process-provider-key".to_owned()),
                Some("env-file-provider-key".to_owned()),
            )
            .as_deref(),
            Some("process-provider-key")
        );
        assert_eq!(
            select_env_value(None, Some("env-file-provider-key".to_owned())).as_deref(),
            Some("env-file-provider-key")
        );
    }

    #[test]
    fn alias_can_target_specific_provider_model() {
        let resolved = test_config().resolve("sonnet-via-router").unwrap();

        assert_eq!(resolved.provider.display_name, "OpenRouter");
        assert_eq!(resolved.model, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn model_prefix_routes_to_provider() {
        let resolved = test_config().resolve("anthropic/claude-sonnet-4").unwrap();

        assert_eq!(resolved.provider.display_name, "OpenRouter");
        assert_eq!(resolved.model, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn known_provider_model_is_preserved() {
        let resolved = test_config().resolve("mimo-v2.5-pro").unwrap();

        assert_eq!(resolved.model, "mimo-v2.5-pro");
    }

    #[test]
    fn alias_to_missing_provider_is_rejected() {
        let mut config = test_config();
        config.aliases.insert(
            "missing".to_owned(),
            "missing-provider:any-model".to_owned(),
        );

        let err = config.resolve("missing").unwrap_err();

        assert!(
            matches!(err, AppError::ProviderNotFound(provider) if provider == "missing-provider")
        );
    }

    #[test]
    fn validation_accepts_test_config_without_errors() {
        let mut config = test_config();
        config.auth_token = Some("long-local-client-token".to_owned());

        let issues = config.validation_issues();

        assert!(
            issues
                .iter()
                .all(|issue| issue.severity != ConfigIssueSeverity::Error),
            "{issues:?}"
        );
    }

    #[test]
    fn validation_accepts_only_governed_exact_model_rate_cards() {
        let mut config = test_config();
        config.auth_token = Some("long-local-client-token".to_owned());
        let provider = config.providers.get_mut("mimo").unwrap();
        provider.model_pricing.insert(
            "mimo-v2.5-pro".to_owned(),
            ModelPricingCard {
                rates: ModelPricing {
                    input_per_million: 0.14,
                    output_per_million: 0.28,
                    cache_write_per_million: 0.14,
                    cache_read_per_million: 0.0028,
                },
                version: "contract-v1".to_owned(),
                effective_at: "2026-08-01T00:00:00Z".to_owned(),
                currency: "USD".to_owned(),
                source: PricingSource::ProviderContract,
                service_tier: PricingServiceTier::Standard,
                region: None,
                evidence: "contract://mimo/v1".to_owned(),
            },
        );
        let valid_card = provider.model_pricing["mimo-v2.5-pro"].clone();

        assert!(
            config
                .validation_issues()
                .iter()
                .all(|issue| { issue.severity != ConfigIssueSeverity::Error })
        );

        let mut invalid = valid_card.clone();
        config
            .providers
            .get_mut("mimo")
            .unwrap()
            .model_pricing
            .insert("not-an-exact-model".to_owned(), invalid.clone());

        let issues = config.validation_issues();
        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue.message.contains("requires an exact model")
        }));

        let provider = config.providers.get_mut("mimo").unwrap();
        provider.model_pricing.remove("not-an-exact-model");
        invalid.source = PricingSource::LegacyEstimate;
        provider
            .model_pricing
            .insert("mimo-v2.5-pro".to_owned(), invalid);
        let issues = config.validation_issues();
        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue.message.contains("cannot be legacy_estimate")
        }));

        let mut invalid = valid_card;
        invalid.service_tier = PricingServiceTier::Priority;
        config
            .providers
            .get_mut("mimo")
            .unwrap()
            .model_pricing
            .insert("mimo-v2.5-pro".to_owned(), invalid);
        let issues = config.validation_issues();
        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue.message.contains("must be standard")
        }));
    }

    #[test]
    fn built_in_deepseek_cards_match_published_flat_rates() {
        let cards = default_model_pricing("deepseek");

        assert_eq!(cards.len(), 2);
        assert_eq!(
            cards["deepseek-v4-flash"].rates,
            ModelPricing {
                input_per_million: 0.14,
                output_per_million: 0.28,
                cache_write_per_million: 0.14,
                cache_read_per_million: 0.0028,
            }
        );
        assert_eq!(
            cards["deepseek-v4-pro"].rates,
            ModelPricing {
                input_per_million: 0.435,
                output_per_million: 0.87,
                cache_write_per_million: 0.435,
                cache_read_per_million: 0.003625,
            }
        );
    }

    #[test]
    fn built_in_cpa_specs_are_isolated_and_closed_by_default() {
        let codex = OPTIONAL_PROVIDER_SPECS
            .iter()
            .find(|spec| spec.id == "cpa_codex")
            .unwrap();
        let claude = OPTIONAL_PROVIDER_SPECS
            .iter()
            .find(|spec| spec.id == "cpa_claude")
            .unwrap();

        assert_eq!(codex.protocol, ProviderProtocol::OpenaiCompat);
        assert_eq!(claude.protocol, ProviderProtocol::Anthropic);
        assert_eq!(codex.api_key_env, Some("CPA_CODEX_API_KEY"));
        assert_eq!(claude.api_key_env, Some("CPA_CLAUDE_API_KEY"));
        assert!(codex.model_prefixes.is_empty());
        assert!(claude.model_prefixes.is_empty());
        assert!(!codex.passthrough_unknown_models);
        assert!(!claude.passthrough_unknown_models);
    }

    #[test]
    fn cpa_providers_validate_and_resolve_by_qualified_model() {
        let mut config = test_config();
        config.auth_token = Some("long-local-client-token".to_owned());
        for provider_id in ["cpa_codex", "cpa_claude"] {
            config.provider_order.push(provider_id.to_owned());
            config
                .providers
                .insert(provider_id.to_owned(), test_cpa_provider(provider_id));
        }

        let issues = config.validation_issues();
        assert!(
            issues
                .iter()
                .all(|issue| issue.severity != ConfigIssueSeverity::Error),
            "{issues:?}"
        );
        let resolved = config.resolve("cpa_claude:claude-sonnet-4-6").unwrap();
        assert_eq!(resolved.provider_id, "cpa_claude");
        assert_eq!(resolved.model, "claude-sonnet-4-6");
    }

    #[test]
    fn cpa_validation_rejects_unsafe_or_ambiguous_configuration() {
        let mut config = test_config();
        config.auth_token = Some("long-local-client-token".to_owned());
        let mut provider = test_cpa_provider("cpa_codex");
        provider.protocol = ProviderProtocol::Anthropic;
        provider.base_url = "http://127.0.0.1:8317/v0/management".to_owned();
        provider.api_key_required = false;
        provider.models.clear();
        provider.model_prefixes.push("gpt-".to_owned());
        provider.passthrough_unknown_models = true;
        config.provider_order.push("cpa_codex".to_owned());
        config.providers.insert("cpa_codex".to_owned(), provider);

        let issues = config.validation_issues();
        for expected in [
            "must use protocol=openai-compat",
            "must require a dedicated CPA client API key",
            "requires an explicit model allowlist",
            "cannot enable passthrough_unknown_models",
            "cannot claim model prefixes",
            "not its management API",
            "must end with `/v1`",
        ] {
            assert!(
                issues.iter().any(|issue| issue.message.contains(expected)),
                "missing `{expected}` in {issues:?}"
            );
        }
    }

    #[test]
    fn smart_routing_groups_are_validated_and_advertised_in_shadow_mode() {
        let mut config = test_config();
        config.auth_token = Some("long-local-client-token".to_owned());
        config.smart_routing = SmartRoutingConfig {
            mode: SmartRoutingMode::Shadow,
            default_profile: RoutingProfile::Balanced,
            policy_version: "test-v1".to_owned(),
            activation_percent: 0,
            groups: HashMap::from([(
                "general".to_owned(),
                RouteGroupConfig {
                    aliases: vec!["modelport-auto".to_owned()],
                    default_profile: Some(RoutingProfile::Economy),
                    candidates: vec![
                        RouteCandidateConfig {
                            provider: "mimo".to_owned(),
                            model: "mimo-v2.5-pro".to_owned(),
                            quality: 0.9,
                            latency_hint_ms: 900,
                            enabled: true,
                        },
                        RouteCandidateConfig {
                            provider: "openrouter".to_owned(),
                            model: "openrouter/auto".to_owned(),
                            quality: 0.8,
                            latency_hint_ms: 700,
                            enabled: true,
                        },
                    ],
                },
            )]),
        };

        let issues = config.validation_issues();
        assert!(
            issues
                .iter()
                .all(|issue| issue.severity != ConfigIssueSeverity::Error),
            "{issues:?}"
        );
        assert!(
            config
                .model_list()
                .iter()
                .any(|(model, _)| model == "modelport-auto")
        );
        let (group_id, group) = config.smart_route_group("modelport-auto").unwrap();
        assert_eq!(group_id, "general");
        assert_eq!(group.candidates.len(), 2);
    }

    #[test]
    fn active_routing_defaults_to_a_zero_percent_canary() {
        let mut config = test_config();
        config.smart_routing.mode = SmartRoutingMode::Active;

        assert_eq!(config.smart_routing.activation_percent, 0);
        assert!(config.validation_issues().iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Warning
                && issue.message.contains("activation_percent=0")
        }));
    }

    #[test]
    fn validation_rejects_out_of_range_sampling_profile() {
        let mut config = test_config();
        config.auth_token = Some("long-local-client-token".to_owned());
        config.providers.get_mut("mimo").unwrap().sampling = SamplingConfig {
            mode: SamplingMode::LlamaCpp,
            profiles: HashMap::from([(
                "bad-profile".to_owned(),
                SamplingProfile {
                    temperature: Some(3.0),
                    ..Default::default()
                },
            )]),
        };

        let issues = config.validation_issues();

        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue
                    .message
                    .contains("temperature must be between 0 and 2")
        }));
    }

    #[test]
    fn debug_output_redacts_loaded_secrets() {
        let mut config = test_config();
        config.auth_token = Some("router-secret-never-log".to_owned());
        config.providers.get_mut("mimo").unwrap().api_key =
            Some("provider-secret-never-log".to_owned());

        let output = format!("{config:?}");

        assert!(!output.contains("router-secret-never-log"));
        assert!(!output.contains("provider-secret-never-log"));
        assert!(output.contains("auth_enabled"));
        assert!(output.contains("has_api_key"));
    }

    #[test]
    fn validation_rejects_missing_default_provider() {
        let mut config = test_config();
        config.default_provider = "missing".to_owned();

        let issues = config.validation_issues();

        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue.message.contains("default provider `missing`")
        }));
    }

    #[test]
    fn validation_rejects_placeholder_provider_secret() {
        let mut config = test_config();
        config
            .providers
            .get_mut("mimo")
            .unwrap()
            .api_key
            .replace("replace-with-real-key".to_owned());

        let issues = config.validation_issues();

        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue.message.contains("provider `mimo` API key")
        }));
    }

    #[test]
    fn validation_warns_for_missing_non_default_provider_secret() {
        let mut config = test_config();
        config
            .providers
            .get_mut("openrouter")
            .unwrap()
            .api_key
            .take();

        let issues = config.validation_issues();

        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Warning
                && issue
                    .message
                    .contains("provider `openrouter` requires API key")
        }));
        assert!(!issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue
                    .message
                    .contains("provider `openrouter` requires API key")
        }));
    }

    #[test]
    fn validation_rejects_missing_default_provider_secret() {
        let mut config = test_config();
        config.providers.get_mut("mimo").unwrap().api_key.take();

        let issues = config.validation_issues();

        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue
                    .message
                    .contains("default provider `mimo` requires API key")
        }));
    }

    #[test]
    fn validation_rejects_alias_cycles() {
        let mut config = test_config();
        config.aliases.insert("a".to_owned(), "b".to_owned());
        config.aliases.insert("b".to_owned(), "a".to_owned());

        let issues = config.validation_issues();

        assert!(issues.iter().any(|issue| {
            issue.severity == ConfigIssueSeverity::Error
                && issue.message.contains("alias `a` cannot resolve")
        }));
    }

    #[test]
    fn static_provider_headers_reject_sensitive_and_transport_authority() {
        for name in [
            "Authorization",
            "X-Auth-Token",
            "X-Vendor-Signature",
            "Content-Type",
            "X-Forwarded-For",
            "Baggage",
            "X-ModelPort-Request-ID",
        ] {
            assert!(
                validate_provider_static_header(name, "unsafe").is_err(),
                "{name} must remain adapter-owned"
            );
        }
        validate_provider_static_header("HTTP-Referer", "https://modelport.example")
            .expect("non-sensitive attribution header should be accepted");
        validate_provider_static_header("X-Title", "ModelPort")
            .expect("non-sensitive attribution header should be accepted");
    }
}
