use std::{
    collections::{BTreeSet, HashMap},
    env, fmt, fs,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, RwLock},
};

use axum::http::HeaderMap;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::{error::AppError, pricing::ModelPricing};

const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 64;

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
    pub reasoning: ReasoningConfig,
    pub sampling: SamplingConfig,
    pub token_counting: TokenCountingConfig,
    pub pricing: Option<ModelPricing>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderProtocol {
    Anthropic,
    OpenaiCompat,
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
    pub default_budget_tokens: Option<u64>,
    #[serde(default)]
    pub model_budget_tokens: HashMap<String, u64>,
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
            .field("providers", &self.providers)
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
            .field("reasoning", &self.reasoning)
            .field("sampling", &self.sampling)
            .field("token_counting", &self.token_counting)
            .field("pricing", &self.pricing)
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

#[derive(Debug, Deserialize)]
struct FileConfig {
    server: Option<ServerSection>,
    auth: Option<AuthSection>,
    default_provider: Option<String>,
    provider_order: Option<Vec<String>>,
    providers: Option<HashMap<String, ProviderSection>>,
    aliases: Option<HashMap<String, String>>,
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
    reasoning: Option<ReasoningConfig>,
    sampling: Option<SamplingConfig>,
    token_counting: Option<TokenCountingConfig>,
    pricing: Option<ModelPricing>,
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

        issues
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
                    reasoning: section.reasoning.unwrap_or_default(),
                    sampling: section.sampling.unwrap_or_default(),
                    token_counting: section.token_counting.unwrap_or_default(),
                    pricing: section.pricing,
                },
            );
        }

        let default_provider = file
            .default_provider
            .or_else(|| provider_order.first().cloned())
            .ok_or_else(|| AppError::Config("at least one provider is required".to_owned()))?;

        Ok(Self {
            bind_addr,
            max_request_body_bytes,
            max_concurrent_requests,
            auth_token,
            default_provider,
            provider_order,
            providers,
            aliases: file.aliases.unwrap_or_default(),
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

        Ok(Self {
            bind_addr,
            max_request_body_bytes,
            max_concurrent_requests,
            auth_token: require_auth_token(default_auth_token())?,
            default_provider,
            provider_order,
            providers,
            aliases,
        })
    }
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
    models: &[
        "deepseek-v4-pro",
        "deepseek-v4-flash",
        "deepseek-chat",
        "deepseek-reasoner",
    ],
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
        models: &[
            "deepseek-v4-pro",
            "deepseek-v4-flash",
            "deepseek-chat",
            "deepseek-reasoner",
        ],
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
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            pricing: None,
        },
    );
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
            "MODELPORT_USAGE_LOG_LIMIT",
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
            "provider `{id}` reasoning budgets require a non-none reasoning.mode"
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
                .is_some())
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
        if provider_allows_loopback_base_url(provider_id) {
            return Ok(());
        }
        return Err("localhost base URLs are only allowed for local/custom providers".to_owned());
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip.is_loopback() && provider_allows_loopback_base_url(provider_id) {
            return Ok(());
        }
        if private_or_metadata_ip(ip) {
            return Err(format!(
                "private, link-local, metadata, or unspecified IP `{ip}` requires MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS=1"
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

fn private_or_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => private_or_metadata_ipv4(ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
            || {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ipv6_is_unique_local(ip)
                    || ipv6_is_unicast_link_local(ip)
            },
            private_or_metadata_ipv4,
        ),
    }
}

fn private_or_metadata_ipv4(ip: std::net::Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.octets() == [169, 254, 169, 254]
}

fn ipv6_is_unique_local(ip: std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

fn ipv6_is_unicast_link_local(ip: std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
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
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            pricing: None,
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
            reasoning: ReasoningConfig::default(),
            sampling: SamplingConfig::default(),
            token_counting: TokenCountingConfig::default(),
            pricing: None,
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
            })
        );
        assert_eq!(
            provider.reasoning,
            Some(ReasoningConfig {
                mode: ReasoningMode::LlamaCpp,
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

        assert!(err.to_string().contains("private"));
    }

    #[test]
    fn provider_url_policy_blocks_ipv4_mapped_loopback_for_remote_provider() {
        let err = validate_provider_base_url_for_request(
            "deepseek",
            "https://[::ffff:127.0.0.1]:8443/v1",
            false,
        )
        .unwrap_err();

        assert!(err.to_string().contains("private"));
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
}
