use std::{
    collections::{BTreeSet, HashMap},
    env, fs,
    net::SocketAddr,
    path::PathBuf,
};

use axum::http::HeaderMap;
use serde::Deserialize;

use crate::error::AppError;

const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 64;

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub provider_id: String,
    pub provider: ProviderConfig,
    pub model: String,
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
                .and_then(|name| env::var(name).ok())
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
                .and_then(|name| env::var(name).ok())
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
            let api_key = section
                .api_key_env
                .as_deref()
                .and_then(|name| env::var(name).ok());
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
                    deduplicate_stream_text: section.deduplicate_stream_text.unwrap_or(false),
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
        let bind_addr = resolve_bind(env::var("MODELPORT_BIND").ok())?;
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
        insert_spec(&mut providers, &mut provider_order, &MIMO_SPEC);

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
            env::var("MODELPORT_DEFAULT_PROVIDER").unwrap_or_else(|_| "mimo".to_owned());

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
    display_name: "DeepSeek Official Anthropic",
    protocol: ProviderProtocol::Anthropic,
    base_url_env: "DEEPSEEK_ANTHROPIC_BASE_URL",
    base_url_env_fallbacks: &[],
    default_base_url: "https://api.deepseek.com/anthropic",
    api_key_env: Some("DEEPSEEK_ANTHROPIC_AUTH_TOKEN"),
    api_key_env_fallbacks: &["DEEPSEEK_API_KEY"],
    api_key_required: true,
    default_model_env: "DEEPSEEK_MODEL",
    default_model: "deepseek-v4-pro",
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
    deduplicate_stream_text: false,
};

const MIMO_SPEC: ProviderSpec = ProviderSpec {
    id: "mimo",
    display_name: "Xiaomi Mimo OpenAI-Compatible",
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
    models: &["mimo-v2.5-pro", "gpt-5.5"],
    model_prefixes: &["mimo-"],
    passthrough_unknown_models: false,
    max_tokens_field: MaxTokensField::MaxCompletionTokens,
    deduplicate_stream_text: true,
};

const OPTIONAL_PROVIDER_SPECS: &[ProviderSpec] = &[
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
        default_model: "claude-sonnet-4-20250514",
        models_env: "ANTHROPIC_UPSTREAM_MODELS",
        models: &[
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
        base_url_env: "OPENAI_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://api.openai.com/v1",
        api_key_env: Some("OPENAI_API_KEY"),
        api_key_env_fallbacks: &[],
        api_key_required: true,
        default_model_env: "OPENAI_MODEL",
        default_model: "gpt-4o",
        models_env: "OPENAI_MODELS",
        models: &["gpt-4o", "gpt-4o-mini", "o3"],
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
        display_name: "Google Gemini OpenAI-Compatible",
        protocol: ProviderProtocol::OpenaiCompat,
        base_url_env: "GEMINI_OPENAI_BASE_URL",
        base_url_env_fallbacks: &[],
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        api_key_env: Some("GEMINI_API_KEY"),
        api_key_env_fallbacks: &["GOOGLE_API_KEY"],
        api_key_required: true,
        default_model_env: "GEMINI_MODEL",
        default_model: "gemini-2.5-flash",
        models_env: "GEMINI_MODELS",
        models: &["gemini-2.5-pro", "gemini-2.5-flash"],
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
        display_name: "Alibaba DashScope Qwen",
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
        models: &["qwen-plus", "qwen-max", "qwen-turbo"],
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
        models: &["kimi-k2.6", "moonshot-v1-128k"],
        model_prefixes: &["kimi-", "moonshot-"],
        passthrough_unknown_models: false,
        max_tokens_field: MaxTokensField::MaxCompletionTokens,
        deduplicate_stream_text: false,
    },
    ProviderSpec {
        id: "zhipu",
        display_name: "Zhipu GLM",
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
        models: &["glm-4.7", "glm-4-flash"],
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
        display_name: "Volcengine Ark Doubao",
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
        display_name: "Ollama Local OpenAI-Compatible",
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
];

const CUSTOM_OPENAI_SPEC: ProviderSpec = ProviderSpec {
    id: "custom",
    display_name: "Custom OpenAI-Compatible",
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
    let default_model = env::var(spec.default_model_env).unwrap_or_else(|_| {
        if spec.id == "mimo" {
            env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| spec.default_model.to_owned())
        } else {
            spec.default_model.to_owned()
        }
    });
    let mut models = env_list(spec.models_env, spec.models);
    if !models.contains(&default_model) {
        models.insert(0, default_model.clone());
    }

    if spec.id == "mimo" {
        extend_mimo_models_from_claude_env(&mut models);
    }

    let api_key = spec
        .api_key_env
        .and_then(|name| env::var(name).ok())
        .or_else(|| first_env(spec.api_key_env_fallbacks));

    insert_provider(
        providers,
        provider_order,
        spec.id.to_owned(),
        ProviderConfig {
            display_name: spec.display_name.to_owned(),
            protocol: spec.protocol,
            base_url: env::var(spec.base_url_env)
                .ok()
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
        },
    );
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

    if env::var(spec.base_url_env).is_ok() || env::var(spec.default_model_env).is_ok() {
        return true;
    }

    if first_env(spec.base_url_env_fallbacks).is_some() {
        return true;
    }

    spec.api_key_env
        .and_then(|name| env::var(name).ok())
        .is_some()
        || first_env(spec.api_key_env_fallbacks).is_some()
}

fn should_enable_custom_openai_provider() -> bool {
    env::var(CUSTOM_OPENAI_SPEC.base_url_env).is_ok()
        || env::var(CUSTOM_OPENAI_SPEC.default_model_env).is_ok()
        || env::var("CUSTOM_OPENAI_API_KEY").is_ok()
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
        if let Ok(value) = env::var(name)
            && !value.starts_with("deepseek-")
            && !models.contains(&value)
        {
            models.push(value);
        }
    }
}

fn env_list(name: &str, defaults: &[&str]) -> Vec<String> {
    env::var(name)
        .ok()
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

fn first_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| env::var(name).ok())
}

fn first_env_owned(names: &[String]) -> Option<String> {
    names.iter().find_map(|name| env::var(name).ok())
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            matches!(
                value.as_str(),
                "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(false)
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
    if let Some(path) = env::var_os("MODELPORT_CONFIG") {
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
        .or_else(|| {
            env::var(env_name)
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
        })
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn default_auth_token() -> Option<String> {
    env::var("MODELPORT_AUTH_TOKEN")
        .ok()
        .or_else(|| env::var("ANTHROPIC_AUTH_TOKEN").ok())
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
        if let Ok(model) = env::var(name) {
            let provider = if model.starts_with("deepseek-") {
                "deepseek"
            } else {
                "mimo"
            };
            aliases.insert(model, provider.to_owned());
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
}
