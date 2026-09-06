use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::{
    config::{AppConfig, ProviderConfig, ResolvedProvider, SmartRoutingMode},
    control::{
        ApiKeyPolicy, ProviderControlSnapshot, ProviderModelOverrideRecord, UserCatalogGrant,
    },
    control_view::provider_credential_rows,
    error::AppError,
    governance::{ProjectPolicy, ProviderBoundary, provider_governance_metadata},
    model_catalog::MODEL_ADAPTATION_CATALOG_VERSION,
};

use super::{
    AppState, effective_config, fidelity_mode_value, management_config, max_tokens_field_value,
    provider_protocol_value,
};

pub(super) fn provider_rows(state: &AppState) -> Vec<Value> {
    ProviderRowAssembler::from_state(state).into_rows()
}

/// Return the deliberately small Provider surface used by ordinary console
/// users. The full management representation contains deployment topology,
/// secret environment-variable names, credential health and raw operator
/// diagnostics; hiding those fields in React is not an authorization boundary.
pub(super) fn catalog_provider_rows(
    state: &AppState,
    user_id: &str,
    api_key_id: Option<&str>,
) -> Vec<Value> {
    EffectiveCatalog::for_user(state, user_id, api_key_id).provider_rows()
}

pub(super) fn catalog_alias_rows(
    state: &AppState,
    user_id: &str,
    api_key_id: Option<&str>,
) -> Vec<Value> {
    EffectiveCatalog::for_user(state, user_id, api_key_id).alias_rows
}

struct EffectiveCatalog {
    config: AppConfig,
    models_by_provider: BTreeMap<String, BTreeSet<String>>,
    aliases_by_provider: BTreeMap<String, BTreeSet<String>>,
    alias_rows: Vec<Value>,
}

impl EffectiveCatalog {
    fn for_user(state: &AppState, user_id: &str, api_key_id: Option<&str>) -> Self {
        let config = effective_config(state);
        let grants = state.control.user_catalog_grants(user_id, api_key_id);
        let mut models_by_provider = BTreeMap::<String, BTreeSet<String>>::new();
        let mut aliases_by_provider = BTreeMap::<String, BTreeSet<String>>::new();
        let mut alias_rows = Vec::new();

        // HashMap iteration is intentionally normalized so the API is stable
        // across processes and does not disclose insertion order.
        let mut aliases = config.aliases.keys().cloned().collect::<Vec<_>>();
        aliases.sort();
        for alias in aliases {
            let Ok(resolved) = config.resolve(&alias) else {
                continue;
            };
            if !grants
                .iter()
                .any(|grant| grant_allows(state, grant, &alias, &resolved))
            {
                continue;
            }
            models_by_provider
                .entry(resolved.provider_id.clone())
                .or_default()
                .insert(resolved.model.clone());
            aliases_by_provider
                .entry(resolved.provider_id.clone())
                .or_default()
                .insert(alias.clone());
            alias_rows.push(json!({
                "alias": alias,
                // Return the effective destination, never an internal alias
                // chain or other control-plane representation.
                "target": format!("{}:{}", resolved.provider_id, resolved.model),
                "resolvedProvider": resolved.provider_id,
                "resolvedModel": resolved.model,
            }));
        }

        if config.smart_routing.mode != SmartRoutingMode::Off {
            let mut smart_aliases = config
                .smart_routing
                .groups
                .values()
                .flat_map(|group| group.aliases.iter().cloned())
                .collect::<Vec<_>>();
            smart_aliases.sort();
            smart_aliases.dedup();
            for alias in smart_aliases {
                let mut candidates = BTreeMap::<(String, String), ResolvedProvider>::new();
                for grant in &grants {
                    let project_policy = state.governance.effective_policy(&grant.tenant);
                    for resolved in catalog_smart_alias_candidates(
                        state,
                        &config,
                        &alias,
                        Some(&grant.policy),
                        &project_policy,
                    ) {
                        candidates.insert(
                            (resolved.provider_id.clone(), resolved.model.clone()),
                            resolved,
                        );
                    }
                }
                if candidates.is_empty() {
                    continue;
                }
                for resolved in candidates.values() {
                    models_by_provider
                        .entry(resolved.provider_id.clone())
                        .or_default()
                        .insert(resolved.model.clone());
                    aliases_by_provider
                        .entry(resolved.provider_id.clone())
                        .or_default()
                        .insert(alias.clone());
                }
                alias_rows.push(json!({
                    "alias": alias,
                    "target": format!("modelport-router:{alias}"),
                    "resolvedProvider": "modelport-router",
                    "resolvedModel": alias,
                    "candidateCount": candidates.len(),
                }));
            }
        }

        for provider_id in &config.provider_order {
            let Some(provider) = config.providers.get(provider_id) else {
                continue;
            };
            for model in &provider.models {
                let requested_model = format!("{provider_id}:{model}");
                let Ok(resolved) = config.resolve(&requested_model) else {
                    continue;
                };
                if grants
                    .iter()
                    .any(|grant| grant_allows(state, grant, &requested_model, &resolved))
                {
                    models_by_provider
                        .entry(provider_id.clone())
                        .or_default()
                        .insert(model.clone());
                }
            }
        }

        Self {
            config,
            models_by_provider,
            aliases_by_provider,
            alias_rows,
        }
    }

    fn provider_rows(self) -> Vec<Value> {
        self.config
            .provider_order
            .iter()
            .filter_map(|provider_id| {
                let provider = self.config.providers.get(provider_id)?;
                let allowed = self.models_by_provider.get(provider_id)?;
                if allowed.is_empty() {
                    return None;
                }
                let models = provider
                    .models
                    .iter()
                    .filter(|model| allowed.contains(*model))
                    .cloned()
                    .collect::<Vec<_>>();
                if models.is_empty() {
                    return None;
                }
                let default_model = if allowed.contains(&provider.default_model) {
                    provider.default_model.clone()
                } else {
                    models[0].clone()
                };
                let aliases = self
                    .aliases_by_provider
                    .get(provider_id)
                    .map(|aliases| aliases.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                let model_inventory = models
                    .iter()
                    .map(|model| {
                        model_profile_row(
                            provider_id,
                            provider,
                            model,
                            "active",
                            model == &default_model,
                        )
                    })
                    .collect::<Vec<_>>();

                // This is a capability catalog, not a management or health
                // surface. Keep the shape useful to existing clients while
                // omitting topology, secret, pricing and diagnostic fields.
                Some(json!({
                    "id": provider_id,
                    "displayName": provider.display_name,
                    "protocol": provider_protocol_value(provider.protocol),
                    "apiKeyRequired": provider.api_key_required,
                    "defaultModel": default_model,
                    "models": models,
                    "aliases": aliases,
                    "modelPrefixes": [],
                    "passthroughUnknownModels": false,
                    "maxTokensField": max_tokens_field_value(provider.max_tokens_field),
                    "deduplicateStreamText": provider.deduplicate_stream_text,
                    "bufferStreamText": provider.buffer_stream_text,
                    "fidelityMode": fidelity_mode_value(provider.fidelity_mode),
                    "toolUse": provider.tool_use,
                    "status": "active",
                    // Inclusion already proves that either a usable credential
                    // pool member or the static credential is available.
                    "hasApiKey": true,
                    "modelInventory": model_inventory,
                }))
            })
            .collect()
    }
}

fn grant_allows(
    state: &AppState,
    grant: &UserCatalogGrant,
    requested_model: &str,
    resolved: &ResolvedProvider,
) -> bool {
    let project_policy = state.governance.effective_policy(&grant.tenant);
    catalog_candidate_is_allowed(
        state,
        Some(&grant.policy),
        &project_policy,
        requested_model,
        resolved,
    )
}

pub(super) fn catalog_candidate_is_allowed(
    state: &AppState,
    api_key_policy: Option<&ApiKeyPolicy>,
    project_policy: &ProjectPolicy,
    requested_model: &str,
    resolved: &ResolvedProvider,
) -> bool {
    resolved.provider.models.contains(&resolved.model)
        && match state
            .control
            .provider_credential_route_available(&resolved.provider_id)
        {
            Some(available) => available,
            None => {
                !resolved.provider.api_key_required
                    || resolved.provider.api_key().ok().flatten().is_some()
            }
        }
        && api_key_policy.is_none_or(|policy| {
            policy
                .enforce_route(requested_model, &resolved.model, &resolved.provider_id)
                .is_ok()
        })
        && project_policy.enforce_attempt(resolved).is_ok()
}

pub(super) fn catalog_smart_alias_candidates(
    state: &AppState,
    config: &AppConfig,
    alias: &str,
    api_key_policy: Option<&ApiKeyPolicy>,
    project_policy: &ProjectPolicy,
) -> Vec<ResolvedProvider> {
    if config.smart_routing.mode == SmartRoutingMode::Off {
        return Vec::new();
    }
    let Some((_, group)) = config.smart_route_group(alias) else {
        return Vec::new();
    };
    group
        .candidates
        .iter()
        .filter(|candidate| candidate.enabled)
        .filter_map(|candidate| {
            let provider = config.providers.get(&candidate.provider)?.clone();
            let resolved = ResolvedProvider {
                provider_id: candidate.provider.clone(),
                provider,
                model: candidate.model.clone(),
            };
            catalog_candidate_is_allowed(state, api_key_policy, project_policy, alias, &resolved)
                .then_some(resolved)
        })
        .collect()
}

pub(super) fn provider_row_by_id(state: &AppState, provider_id: &str) -> Result<Value, AppError> {
    provider_rows(state)
        .into_iter()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(provider_id))
        .ok_or_else(|| AppError::ProviderNotFound(provider_id.to_owned()))
}

struct ProviderRowAssembler {
    config: AppConfig,
    controls: ProviderControlSnapshot,
    provider_tests: BTreeMap<String, Value>,
    provider_health: BTreeMap<String, Value>,
    credential_health: BTreeMap<String, BTreeMap<String, Value>>,
}

impl ProviderRowAssembler {
    fn from_state(state: &AppState) -> Self {
        Self {
            config: management_config(state),
            controls: state.control.provider_control_snapshot(),
            provider_tests: state.control.provider_test_rows(),
            provider_health: state.control.provider_health_rows(),
            credential_health: state.control.provider_credential_health_rows(),
        }
    }

    fn into_rows(self) -> Vec<Value> {
        self.config
            .provider_order
            .iter()
            .filter_map(|id| self.provider_row(id))
            .collect()
    }

    fn provider_row(&self, id: &str) -> Option<Value> {
        let provider = self.config.providers.get(id)?;
        let has_api_key = provider.api_key().ok().flatten().is_some();
        let health = self.provider_health.get(id).cloned();
        let active_credential_id = self
            .controls
            .active_provider_credentials
            .get(id)
            .map(String::as_str);
        let credential_pool_mode = self
            .controls
            .provider_credential_pool_modes
            .get(id)
            .map(String::as_str)
            .unwrap_or("failover");
        let credentials = provider_credential_rows(
            self.controls.provider_credentials.get(id),
            active_credential_id,
            self.credential_health.get(id),
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
        let status = if self.controls.disabled_providers.contains(id) {
            "disabled"
        } else {
            config_status
        };
        let resolved = ResolvedProvider {
            provider_id: id.to_owned(),
            provider: provider.clone(),
            model: provider.default_model.clone(),
        };
        let (region, api_version) = provider_governance_metadata(&resolved);

        Some(json!({
            "id": id,
            "displayName": provider.display_name,
            "source": if self.controls.provider_overrides.contains_key(id) { "control" } else { "config" },
            "protocol": provider_protocol_value(provider.protocol),
            "governance": {
                "boundary": if ProviderBoundary::for_resolved(&resolved) == ProviderBoundary::Local { "local" } else { "cloud" },
                "region": region,
                "apiVersion": api_version,
            },
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
            "toolUse": provider.tool_use,
            "modelProfileDefaults": provider.model_profile_defaults,
            "modelProfiles": provider.model_profiles,
            "reasoning": provider.reasoning,
            "sampling": provider.sampling,
            "tokenCounting": provider.token_counting,
            "staticHeaders": provider.static_headers,
            "requestTimeoutMs": provider.request_timeout_ms,
            "streamIdleTimeoutMs": provider.stream_idle_timeout_ms,
            "retry": provider.retry,
            "adaptationCatalogVersion": MODEL_ADAPTATION_CATALOG_VERSION,
            "pricing": provider.pricing,
            "modelPricing": provider.model_pricing,
            "trustUpstreamCost": provider.trust_upstream_cost,
            "status": status,
            "runtimeStatus": runtime_status,
            "hasApiKey": has_api_key,
            "credentials": credentials,
            "activeCredentialId": active_credential_id,
            "credentialPoolMode": credential_pool_mode,
            "lastTest": self.provider_tests.get(id).cloned(),
            "health": health,
            "modelInventory": self.provider_inventory_rows(id, provider),
        }))
    }

    fn provider_inventory_rows(&self, provider_id: &str, provider: &ProviderConfig) -> Vec<Value> {
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        let overrides = self.controls.provider_model_overrides.get(provider_id);
        for model in &provider.models {
            seen.insert(model.clone());
            let override_record = overrides.and_then(|models| models.get(model));
            rows.push(model_profile_row(
                provider_id,
                provider,
                model,
                override_record
                    .map(|record| record.status.as_str())
                    .unwrap_or("active"),
                model == &provider.default_model,
            ));
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
}

pub(super) fn provider_model_row(record: &ProviderModelOverrideRecord) -> Value {
    json!({
        "providerId": record.provider_id,
        "model": record.model,
        "status": record.status,
        "displayName": record.display_name,
        "family": record.family,
        "contextWindow": record.context_window,
        "profile": record.profile,
        "createdAt": record.created_at_ms.to_string(),
        "updatedAt": record.updated_at_ms.to_string(),
    })
}

fn model_profile_row(
    provider_id: &str,
    provider: &ProviderConfig,
    model: &str,
    status: &str,
    is_default: bool,
) -> Value {
    let profile = provider.model_profile(provider_id, model);
    let mut value = serde_json::to_value(profile).unwrap_or_else(|_| json!({"model": model}));
    if let Some(value) = value.as_object_mut() {
        value.insert("status".to_owned(), Value::String(status.to_owned()));
        value.insert("default".to_owned(), Value::Bool(is_default));
        value.insert(
            "catalogVersion".to_owned(),
            Value::from(MODEL_ADAPTATION_CATALOG_VERSION),
        );
        if let Some(configured) = provider.model_profiles.get(model) {
            value.insert(
                "override".to_owned(),
                serde_json::to_value(configured).unwrap_or(Value::Null),
            );
        }
    }
    value
}
