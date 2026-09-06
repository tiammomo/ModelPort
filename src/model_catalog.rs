use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};

use crate::config::{ProviderProtocol, ToolUseConfig};

pub(crate) const MODEL_ADAPTATION_CATALOG_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

impl CapabilitySupport {
    pub(crate) fn is_supported(self) -> bool {
        self == Self::Supported
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum InputModality {
    Text,
    Image,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl ReasoningEffort {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "off" | "none" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningDialect {
    #[default]
    None,
    NativeAnthropic,
    Openai,
    Deepseek,
    Openrouter,
    Qwen,
    Zai,
    StringThinking,
    LlamaCpp,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningReplay {
    #[default]
    None,
    SameProtocol,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Verified,
    #[default]
    Unverified,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelProfileSource {
    Catalog,
    Config,
    Control,
    Discovery,
    #[default]
    ProviderDefault,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelProfileOverride {
    pub display_name: Option<String>,
    pub family: Option<String>,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub input_modalities: Option<Vec<InputModality>>,
    pub tool_use: Option<CapabilitySupport>,
    pub tool_choice: Option<CapabilitySupport>,
    pub parallel_tool_calls: Option<CapabilitySupport>,
    pub strict_tool_schema: Option<CapabilitySupport>,
    pub reasoning: Option<CapabilitySupport>,
    pub reasoning_efforts: Option<Vec<ReasoningEffort>>,
    #[serde(default)]
    pub reasoning_effort_map: BTreeMap<ReasoningEffort, String>,
    pub default_reasoning_effort: Option<ReasoningEffort>,
    pub reasoning_dialect: Option<ReasoningDialect>,
    pub reasoning_replay: Option<ReasoningReplay>,
    #[serde(skip)]
    pub source: Option<ModelProfileSource>,
}

impl ModelProfileOverride {
    pub(crate) fn merge(&mut self, next: &Self) {
        macro_rules! replace_option {
            ($field:ident) => {
                if next.$field.is_some() {
                    self.$field.clone_from(&next.$field);
                }
            };
        }
        replace_option!(display_name);
        replace_option!(family);
        replace_option!(context_window);
        replace_option!(max_output_tokens);
        replace_option!(input_modalities);
        replace_option!(tool_use);
        replace_option!(tool_choice);
        replace_option!(parallel_tool_calls);
        replace_option!(strict_tool_schema);
        replace_option!(reasoning);
        replace_option!(reasoning_efforts);
        replace_option!(default_reasoning_effort);
        replace_option!(reasoning_dialect);
        replace_option!(reasoning_replay);
        if !next.reasoning_effort_map.is_empty() {
            self.reasoning_effort_map
                .extend(next.reasoning_effort_map.clone());
        }
        if next.source.is_some() {
            self.source = next.source;
        }
    }
}

pub(crate) fn validate_model_profile_override(value: &ModelProfileOverride) -> Result<(), String> {
    for (field, text, max_len) in [
        ("display_name", value.display_name.as_deref(), 240usize),
        ("family", value.family.as_deref(), 120usize),
    ] {
        if let Some(text) = text
            && (text.trim().is_empty()
                || text.len() > max_len
                || text.chars().any(char::is_control))
        {
            return Err(format!(
                "{field} must be non-empty, contain no control characters, and be at most {max_len} bytes"
            ));
        }
    }
    if value.context_window == Some(0) || value.max_output_tokens == Some(0) {
        return Err("token limits must be positive".to_owned());
    }
    if let (Some(context), Some(output)) = (value.context_window, value.max_output_tokens)
        && output > context
    {
        return Err("max_output_tokens cannot exceed context_window".to_owned());
    }
    if let Some(modalities) = &value.input_modalities {
        let unique = modalities.iter().copied().collect::<BTreeSet<_>>();
        if modalities.is_empty() || unique.len() != modalities.len() {
            return Err("input_modalities must be non-empty and contain no duplicates".to_owned());
        }
        if !modalities.contains(&InputModality::Text) {
            return Err(
                "input_modalities must include text in the current protocol slice".to_owned(),
            );
        }
    }
    if let Some(efforts) = &value.reasoning_efforts {
        let unique = efforts.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != efforts.len() {
            return Err("reasoning_efforts cannot contain duplicates".to_owned());
        }
        if let Some(default_effort) = value.default_reasoning_effort
            && !efforts.contains(&default_effort)
        {
            return Err("default_reasoning_effort must be listed in reasoning_efforts".to_owned());
        }
        if value
            .reasoning_effort_map
            .keys()
            .any(|effort| !efforts.contains(effort))
        {
            return Err("reasoning_effort_map keys must be listed in reasoning_efforts".to_owned());
        }
    }
    if value.reasoning_effort_map.values().any(|mapped| {
        mapped.trim().is_empty() || mapped.len() > 64 || mapped.chars().any(char::is_control)
    }) {
        return Err(
            "reasoning_effort_map values must be non-empty, bounded strings without control characters"
                .to_owned(),
        );
    }
    if value
        .tool_use
        .is_some_and(|support| !support.is_supported())
        && (value
            .tool_choice
            .is_some_and(CapabilitySupport::is_supported)
            || value
                .parallel_tool_calls
                .is_some_and(CapabilitySupport::is_supported)
            || value
                .strict_tool_schema
                .is_some_and(CapabilitySupport::is_supported))
    {
        return Err(
            "advanced tool features cannot be supported when tool_use is not supported".to_owned(),
        );
    }
    if value.reasoning == Some(CapabilitySupport::Supported)
        && value.reasoning_dialect == Some(ReasoningDialect::None)
    {
        return Err("reasoning_dialect cannot be none when reasoning is supported".to_owned());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    pub model: String,
    pub display_name: Option<String>,
    pub family: Option<String>,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub input_modalities: Vec<InputModality>,
    pub tool_use: CapabilitySupport,
    pub tool_choice: CapabilitySupport,
    pub parallel_tool_calls: CapabilitySupport,
    pub strict_tool_schema: CapabilitySupport,
    pub reasoning: CapabilitySupport,
    pub reasoning_efforts: Vec<ReasoningEffort>,
    pub reasoning_effort_map: BTreeMap<ReasoningEffort, String>,
    pub default_reasoning_effort: Option<ReasoningEffort>,
    pub reasoning_dialect: ReasoningDialect,
    pub reasoning_replay: ReasoningReplay,
    pub verification: VerificationStatus,
    pub source: ModelProfileSource,
}

impl ModelProfile {
    fn from_provider(model: &str, tool_use: &ToolUseConfig) -> Self {
        Self {
            model: model.to_owned(),
            display_name: None,
            family: None,
            context_window: None,
            max_output_tokens: None,
            input_modalities: vec![InputModality::Text],
            tool_use: support(tool_use.supported),
            tool_choice: support(tool_use.supported && tool_use.tool_choice),
            parallel_tool_calls: support(tool_use.supported && tool_use.parallel_tool_calls),
            strict_tool_schema: CapabilitySupport::Unknown,
            reasoning: CapabilitySupport::Unknown,
            reasoning_efforts: Vec::new(),
            reasoning_effort_map: BTreeMap::new(),
            default_reasoning_effort: None,
            reasoning_dialect: ReasoningDialect::None,
            reasoning_replay: ReasoningReplay::None,
            verification: VerificationStatus::Unverified,
            source: ModelProfileSource::ProviderDefault,
        }
    }

    fn apply(&mut self, value: &ModelProfileOverride, source: ModelProfileSource) {
        let mut applied = false;
        macro_rules! apply_optional_target {
            ($field:ident) => {
                if let Some(next) = &value.$field {
                    self.$field = Some(next.clone());
                    applied = true;
                }
            };
        }
        macro_rules! apply_required_target {
            ($field:ident) => {
                if let Some(next) = &value.$field {
                    self.$field = next.clone();
                    applied = true;
                }
            };
        }
        apply_optional_target!(display_name);
        apply_optional_target!(family);
        apply_optional_target!(context_window);
        apply_optional_target!(max_output_tokens);
        apply_required_target!(input_modalities);
        apply_required_target!(tool_use);
        apply_required_target!(tool_choice);
        apply_required_target!(parallel_tool_calls);
        apply_required_target!(strict_tool_schema);
        apply_required_target!(reasoning);
        apply_required_target!(reasoning_efforts);
        apply_optional_target!(default_reasoning_effort);
        apply_required_target!(reasoning_dialect);
        apply_required_target!(reasoning_replay);
        if !value.reasoning_effort_map.is_empty() {
            self.reasoning_effort_map
                .extend(value.reasoning_effort_map.clone());
            applied = true;
        }
        if applied {
            self.source = value.source.unwrap_or(source);
        }
    }
}

fn support(value: bool) -> CapabilitySupport {
    if value {
        CapabilitySupport::Supported
    } else {
        CapabilitySupport::Unsupported
    }
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    version: u32,
    providers: BTreeMap<String, CatalogProvider>,
}

#[derive(Debug, Default, Deserialize)]
struct CatalogProvider {
    #[serde(default)]
    defaults: ModelProfileOverride,
    #[serde(default)]
    models: BTreeMap<String, ModelProfileOverride>,
}

fn catalog() -> &'static CatalogFile {
    static CATALOG: OnceLock<CatalogFile> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let parsed: CatalogFile = serde_json::from_str(include_str!(
            "../resources/catalog/provider-adaptations-v1.json"
        ))
        .expect("embedded provider adaptation catalog must be valid JSON");
        assert_eq!(
            parsed.version, MODEL_ADAPTATION_CATALOG_VERSION,
            "embedded provider adaptation catalog version drift"
        );
        parsed
    })
}

pub(crate) fn resolve_model_profile(
    provider_id: &str,
    provider_protocol: ProviderProtocol,
    model: &str,
    provider_tool_use: &ToolUseConfig,
    provider_defaults: &ModelProfileOverride,
    configured: Option<&ModelProfileOverride>,
) -> ModelProfile {
    let mut profile = ModelProfile::from_provider(model, provider_tool_use);
    let generic_provider_id = match provider_protocol {
        ProviderProtocol::OpenaiCompat => "custom",
        ProviderProtocol::Anthropic => "generic_anthropic",
    };
    if let Some(provider) = catalog()
        .providers
        .get(provider_id)
        .or_else(|| catalog().providers.get(generic_provider_id))
    {
        profile.apply(&provider.defaults, ModelProfileSource::Catalog);
        if let Some(model_profile) = provider.models.get(model) {
            profile.apply(model_profile, ModelProfileSource::Catalog);
        }
    }
    profile.apply(provider_defaults, ModelProfileSource::Config);
    if let Some(configured) = configured {
        profile.apply(configured, ModelProfileSource::Config);
    }
    profile
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolUseConfig;

    #[test]
    fn catalog_version_and_deepseek_reasoning_profile_are_stable() {
        assert_eq!(catalog().version, MODEL_ADAPTATION_CATALOG_VERSION);
        let profile = resolve_model_profile(
            "deepseek_openai",
            ProviderProtocol::OpenaiCompat,
            "deepseek-v4-flash",
            &ToolUseConfig::default(),
            &ModelProfileOverride::default(),
            None,
        );
        assert_eq!(profile.context_window, Some(1_000_000));
        assert_eq!(profile.reasoning, CapabilitySupport::Supported);
        assert_eq!(profile.reasoning_dialect, ReasoningDialect::Deepseek);
        assert_eq!(profile.reasoning_replay, ReasoningReplay::SameProtocol);
        assert!(profile.reasoning_efforts.contains(&ReasoningEffort::Max));
    }

    #[test]
    fn explicit_model_override_wins_catalog_without_marking_verified() {
        let configured = ModelProfileOverride {
            context_window: Some(65_536),
            reasoning: Some(CapabilitySupport::Unsupported),
            source: Some(ModelProfileSource::Control),
            ..ModelProfileOverride::default()
        };
        let profile = resolve_model_profile(
            "deepseek_openai",
            ProviderProtocol::OpenaiCompat,
            "deepseek-v4-flash",
            &ToolUseConfig::default(),
            &ModelProfileOverride::default(),
            Some(&configured),
        );
        assert_eq!(profile.context_window, Some(65_536));
        assert_eq!(profile.reasoning, CapabilitySupport::Unsupported);
        assert_eq!(profile.source, ModelProfileSource::Control);
        assert_eq!(profile.verification, VerificationStatus::Unverified);
    }

    #[test]
    fn unknown_provider_uses_a_protocol_safe_generic_profile() {
        let openai = resolve_model_profile(
            "siliconflow",
            ProviderProtocol::OpenaiCompat,
            "vendor/model",
            &ToolUseConfig::default(),
            &ModelProfileOverride::default(),
            None,
        );
        assert_eq!(openai.reasoning, CapabilitySupport::Unknown);
        assert_eq!(openai.reasoning_dialect, ReasoningDialect::Openai);

        let anthropic = resolve_model_profile(
            "anthropic_proxy",
            ProviderProtocol::Anthropic,
            "vendor/model",
            &ToolUseConfig::default(),
            &ModelProfileOverride::default(),
            None,
        );
        assert_eq!(anthropic.reasoning, CapabilitySupport::Unknown);
        assert_eq!(
            anthropic.reasoning_dialect,
            ReasoningDialect::NativeAnthropic
        );
    }

    #[test]
    fn profile_validation_rejects_ambiguous_or_impossible_overrides() {
        let invalid_limits = ModelProfileOverride {
            context_window: Some(8_192),
            max_output_tokens: Some(16_384),
            ..ModelProfileOverride::default()
        };
        assert!(validate_model_profile_override(&invalid_limits).is_err());

        let invalid_modalities = ModelProfileOverride {
            input_modalities: Some(vec![InputModality::Image]),
            ..ModelProfileOverride::default()
        };
        assert!(validate_model_profile_override(&invalid_modalities).is_err());

        let invalid_effort = ModelProfileOverride {
            reasoning_efforts: Some(vec![ReasoningEffort::Low]),
            default_reasoning_effort: Some(ReasoningEffort::High),
            ..ModelProfileOverride::default()
        };
        assert!(validate_model_profile_override(&invalid_effort).is_err());
    }

    #[test]
    fn catalog_covers_every_built_in_provider_id() {
        for provider_id in [
            "cpa_codex",
            "cpa_claude",
            "deepseek",
            "deepseek_openai",
            "anthropic",
            "openai",
            "openrouter",
            "gemini",
            "xai",
            "groq",
            "dashscope",
            "kimi",
            "zhipu",
            "mistral",
            "ark",
            "mimo",
            "ollama",
            "local_sglang",
            "local_vllm",
            "local_llamacpp",
            "custom",
        ] {
            assert!(
                catalog().providers.contains_key(provider_id),
                "missing catalog entry for {provider_id}"
            );
        }
    }
}
