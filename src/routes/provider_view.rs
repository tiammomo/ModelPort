use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::{
    config::{AppConfig, ProviderConfig},
    control::{ProviderControlSnapshot, ProviderModelOverrideRecord},
    control_view::provider_credential_rows,
    error::AppError,
};

use super::{
    AppState, fidelity_mode_value, management_config, max_tokens_field_value,
    provider_protocol_value,
};

pub(super) fn provider_rows(state: &AppState) -> Vec<Value> {
    ProviderRowAssembler::from_state(state).into_rows()
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

        Some(json!({
            "id": id,
            "displayName": provider.display_name,
            "source": if self.controls.provider_overrides.contains_key(id) { "control" } else { "config" },
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
            "toolUse": provider.tool_use,
            "pricing": provider.pricing,
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
}

pub(super) fn provider_model_row(record: &ProviderModelOverrideRecord) -> Value {
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
