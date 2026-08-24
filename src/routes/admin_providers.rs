use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::*;

#[cfg(test)]
use super::route_contract::RouteContract;

#[cfg(test)]
pub(super) const ROUTES: &[RouteContract] = &[
    RouteContract::new("admin-providers", "/admin/providers", &["GET", "POST"]),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}",
        &["PUT", "DELETE"],
    ),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}/disable",
        &["POST"],
    ),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}/models",
        &["POST", "PUT", "DELETE"],
    ),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}/balance",
        &["POST"],
    ),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}/credentials",
        &["POST"],
    ),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}/credential-pool",
        &["PUT"],
    ),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}/credentials/{credential_id}",
        &["PUT", "DELETE"],
    ),
    RouteContract::new(
        "admin-providers",
        "/admin/providers/{provider_id}/credentials/{credential_id}/select",
        &["POST"],
    ),
];

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/providers",
            get(admin_providers).post(admin_create_provider),
        )
        .route(
            "/admin/providers/{provider_id}",
            put(admin_update_provider).delete(admin_delete_provider),
        )
        .route(
            "/admin/providers/{provider_id}/disable",
            post(admin_set_provider_disabled),
        )
        .route(
            "/admin/providers/{provider_id}/models",
            post(admin_provider_models)
                .put(admin_upsert_provider_model)
                .delete(admin_delete_provider_model),
        )
        .route(
            "/admin/providers/{provider_id}/balance",
            post(admin_provider_balance),
        )
        .route(
            "/admin/providers/{provider_id}/credentials",
            post(admin_create_provider_credential),
        )
        .route(
            "/admin/providers/{provider_id}/credential-pool",
            put(admin_set_provider_credential_pool_mode),
        )
        .route(
            "/admin/providers/{provider_id}/credentials/{credential_id}",
            put(admin_update_provider_credential).delete(admin_delete_provider_credential),
        )
        .route(
            "/admin/providers/{provider_id}/credentials/{credential_id}/select",
            post(admin_select_provider_credential),
        )
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeleteProviderQuery {
    force: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProviderWriteBody {
    id: Option<String>,
    display_name: Option<String>,
    protocol: Option<String>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    clear_api_key_env: Option<bool>,
    api_key_required: Option<bool>,
    default_model: Option<String>,
    models: Option<Vec<String>>,
    model_prefixes: Option<Vec<String>>,
    passthrough_unknown_models: Option<bool>,
    max_tokens_field: Option<String>,
    deduplicate_stream_text: Option<bool>,
    buffer_stream_text: Option<bool>,
    fidelity_mode: Option<String>,
    tool_use: Option<ToolUseConfig>,
    model_profile_defaults: Option<crate::model_catalog::ModelProfileOverride>,
    model_profiles:
        Option<std::collections::HashMap<String, crate::model_catalog::ModelProfileOverride>>,
    reasoning: Option<crate::config::ReasoningConfig>,
    sampling: Option<crate::config::SamplingConfig>,
    token_counting: Option<crate::config::TokenCountingConfig>,
    static_headers: Option<std::collections::BTreeMap<String, String>>,
    request_timeout_ms: Option<u64>,
    stream_idle_timeout_ms: Option<u64>,
    retry: Option<crate::config::ProviderRetryConfig>,
    pricing: Option<crate::pricing::ModelPricing>,
    model_pricing: Option<std::collections::HashMap<String, crate::pricing::ModelPricingCard>>,
    trust_upstream_cost: Option<bool>,
    disabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProviderDisableBody {
    disabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProviderModelWriteBody {
    model: String,
    status: Option<String>,
    display_name: Option<String>,
    family: Option<String>,
    context_window: Option<u64>,
    profile: Option<crate::model_catalog::ModelProfileOverride>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProviderCredentialWriteBody {
    id: Option<String>,
    name: String,
    api_key_env: String,
    base_url: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProviderCredentialPoolBody {
    mode: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProviderCatalogQuery {
    pub(super) api_key_id: Option<String>,
}

pub(super) async fn admin_providers(
    State(state): State<AppState>,
    Query(query): Query<ProviderCatalogQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    let rows = if actor.role == "admin" {
        provider_rows(&state)
    } else {
        catalog_provider_rows(&state, &actor.id, query.api_key_id.as_deref())
    };
    Ok(Json(Value::Array(rows)))
}

pub(super) async fn admin_create_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ProviderWriteBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let provider_id = body
        .id
        .clone()
        .ok_or_else(|| AppError::InvalidRequest("provider id is required".to_owned()))?;
    let disabled = body.disabled.unwrap_or(false);
    let approval_payload = serde_json::to_value(&body)?;
    let approval_id = require_high_risk_change(
        &state,
        &headers,
        "provider.allowlist_change",
        &format!("provider:{provider_id}"),
        &approval_payload,
    )?;
    let record = provider_body_to_record(&provider_id, body, None)?;
    ensure_organization_approved_provider(&state, &record)?;
    let record = state.control.upsert_provider_override(record)?;
    state.control.set_provider_disabled(&record.id, disabled)?;
    mark_high_risk_change_applied(&state, approval_id.as_deref())?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{}", record.id),
        format!("新增供应商 {}", record.id),
        "info",
    )
    .await;
    Ok(Json(provider_row_by_id(&state, &record.id)?))
}

pub(super) async fn admin_update_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderWriteBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let disabled = body.disabled;
    let approval_payload = serde_json::to_value(&body)?;
    let approval_id = require_high_risk_change(
        &state,
        &headers,
        "provider.allowlist_change",
        &format!("provider:{provider_id}"),
        &approval_payload,
    )?;
    let current = management_config(&state)
        .providers
        .get(&provider_id)
        .cloned()
        .ok_or_else(|| AppError::ProviderNotFound(provider_id.clone()))?;
    let record = provider_body_to_record(&provider_id, body, Some(current))?;
    ensure_organization_approved_provider(&state, &record)?;
    let record = state.control.upsert_provider_override(record)?;
    if let Some(disabled) = disabled {
        state.control.set_provider_disabled(&record.id, disabled)?;
    }
    mark_high_risk_change_applied(&state, approval_id.as_deref())?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{}", record.id),
        format!("更新供应商 {}", record.id),
        "info",
    )
    .await;
    Ok(Json(provider_row_by_id(&state, &record.id)?))
}

pub(super) async fn admin_set_provider_disabled(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderDisableBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    if !management_config(&state)
        .providers
        .contains_key(&provider_id)
    {
        return Err(AppError::ProviderNotFound(provider_id));
    }
    let disabled = body.disabled.unwrap_or(true);
    let approval_payload = serde_json::to_value(&body)?;
    let approval_id = require_high_risk_change(
        &state,
        &headers,
        "provider.allowlist_change",
        &format!("provider:{provider_id}"),
        &approval_payload,
    )?;
    state
        .control
        .set_provider_disabled(&provider_id, disabled)?;
    mark_high_risk_change_applied(&state, approval_id.as_deref())?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}"),
        if disabled {
            format!("禁用供应商 {provider_id}")
        } else {
            format!("启用供应商 {provider_id}")
        },
        if disabled { "warning" } else { "info" },
    )
    .await;
    Ok(Json(provider_row_by_id(&state, &provider_id)?))
}

pub(super) async fn admin_delete_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Query(query): Query<DeleteProviderQuery>,
) -> Result<Response, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let approval_payload = json!({ "delete": true, "force": query.force.unwrap_or(false) });
    let approval_id = require_high_risk_change(
        &state,
        &headers,
        "provider.allowlist_change",
        &format!("provider:{provider_id}"),
        &approval_payload,
    )?;
    let management = management_config(&state);
    if !management.providers.contains_key(&provider_id) {
        return Err(AppError::ProviderNotFound(provider_id));
    }
    let dependencies = provider_delete_dependencies(&state, &management, &provider_id);
    if !dependencies.is_empty() && !query.force.unwrap_or(false) {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "blocked": true,
                "providerId": provider_id,
                "message": "provider is still referenced; pass force=true after reviewing dependencies",
                "dependencies": dependencies,
            })),
        )
            .into_response());
    }

    for dependency in dependencies
        .iter()
        .filter(|dependency| dependency.get("type").and_then(Value::as_str) == Some("alias"))
    {
        if let Some(alias) = dependency.get("id").and_then(Value::as_str) {
            let tombstone = state.config.snapshot().aliases.contains_key(alias);
            state.control.delete_alias(alias, tombstone)?;
        }
    }

    let tombstone = state.config.snapshot().providers.contains_key(&provider_id);
    state.control.delete_provider(&provider_id, tombstone)?;
    mark_high_risk_change_applied(&state, approval_id.as_deref())?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}"),
        format!("删除供应商 {provider_id}"),
        "warning",
    )
    .await;
    Ok(Json(json!({ "ok": true, "providerId": provider_id })).into_response())
}

pub(super) async fn admin_provider_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let config = management_config(&state);
    let Some(provider) = config.providers.get(&provider_id).cloned() else {
        return Err(AppError::ProviderNotFound(provider_id));
    };

    let probe = run_provider_management_probe(&state, &provider_id, &provider).await?;
    let tested_credential_id = probe.tested_credential_id;
    let (success, message, models) = match probe.result {
        Ok(models) => {
            let message = if provider_supports_model_discovery(&provider_id, &provider) {
                format!("discovered {} model(s)", models.len())
            } else {
                "model discovery is not available for this protocol; returned configured models"
                    .to_owned()
            };
            (true, message, models)
        }
        Err(message) => (false, message, Vec::new()),
    };
    let tested_at = state.control.record_provider_test_for_credential(
        provider_id.clone(),
        success,
        message.clone(),
        models.clone(),
        tested_credential_id.clone(),
    )?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}"),
        format!("发现供应商 {provider_id} 模型: {message}"),
        if success { "info" } else { "warning" },
    )
    .await;

    Ok(Json(json!({
        "providerId": provider_id,
        "success": success,
        "message": message,
        "models": models,
        "modelCount": models.len(),
        "testedCredentialId": tested_credential_id,
        "discoveredAt": tested_at.to_string(),
    })))
}

pub(super) async fn admin_provider_balance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    if provider_id != "deepseek" {
        return Err(AppError::InvalidRequest(
            "online balance is currently supported only for the DeepSeek provider".to_owned(),
        ));
    }
    let config = management_config(&state);
    let provider = config
        .providers
        .get(&provider_id)
        .ok_or_else(|| AppError::ProviderNotFound(provider_id.clone()))?;
    let balance =
        crate::providers::deepseek_account::fetch_balance(&state.transport, provider).await?;
    record_admin_activity(
        &state,
        &actor,
        "provider_balance_check",
        format!("provider:{provider_id}"),
        format!("查询供应商 {provider_id} 线上余额"),
        if balance.is_available {
            "info"
        } else {
            "warning"
        },
    )
    .await;
    Ok(Json(json!({
        "providerId": provider_id,
        "isAvailable": balance.is_available,
        "balanceInfos": balance.balance_infos.into_iter().map(|info| json!({
            "currency": info.currency,
            "totalBalance": info.total_balance,
            "grantedBalance": info.granted_balance,
            "toppedUpBalance": info.topped_up_balance,
        })).collect::<Vec<_>>(),
        "checkedAt": now_millis_string(),
        "managementScope": "read-monitor-alert",
        "billingAuthority": "deepseek-console",
    })))
}

pub(super) async fn admin_upsert_provider_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderModelWriteBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let approval_payload = serde_json::to_value(&body)?;
    let approval_id = require_high_risk_change(
        &state,
        &headers,
        "model.production_promotion",
        &format!("provider:{provider_id}:model:{}", body.model),
        &approval_payload,
    )?;
    let config = management_config(&state);
    let Some(provider) = config.providers.get(&provider_id) else {
        return Err(AppError::ProviderNotFound(provider_id));
    };
    ensure_organization_approved_model(&state, &provider_id, &body.model)?;
    let status = body.status.unwrap_or_else(|| "active".to_owned());
    if status == "disabled"
        && provider.default_model == body.model
        && provider
            .models
            .iter()
            .filter(|model| *model != &body.model)
            .count()
            == 0
    {
        return Err(AppError::InvalidRequest(
            "cannot disable the last/default model of an enabled provider; disable the provider instead"
                .to_owned(),
        ));
    }
    let record = state
        .control
        .upsert_provider_model_override(ProviderModelOverrideRecord {
            provider_id: provider_id.clone(),
            model: body.model,
            status,
            display_name: body.display_name,
            family: body.family,
            context_window: body.context_window,
            profile: body.profile.unwrap_or_default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        })?;
    mark_high_risk_change_applied(&state, approval_id.as_deref())?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}:model:{}", record.model),
        format!(
            "更新供应商 {provider_id} 模型 {} 为 {}",
            record.model, record.status
        ),
        if record.status == "disabled" {
            "warning"
        } else {
            "info"
        },
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "providerId": provider_id,
        "model": provider_model_row(&record),
        "provider": provider_row_by_id(&state, &record.provider_id)?,
    })))
}

pub(super) async fn admin_delete_provider_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderModelWriteBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let approval_payload = serde_json::to_value(&body)?;
    let approval_id = require_high_risk_change(
        &state,
        &headers,
        "provider.allowlist_change",
        &format!("provider:{provider_id}:model:{}", body.model),
        &approval_payload,
    )?;
    let config = management_config(&state);
    let Some(provider) = config.providers.get(&provider_id) else {
        return Err(AppError::ProviderNotFound(provider_id));
    };
    if provider.default_model == body.model
        && provider
            .models
            .iter()
            .filter(|model| *model != &body.model)
            .count()
            == 0
    {
        return Err(AppError::InvalidRequest(
            "cannot delete the last/default model of an enabled provider; disable the provider instead"
                .to_owned(),
        ));
    }
    let record = state
        .control
        .delete_provider_model_override(&provider_id, &body.model)?;
    mark_high_risk_change_applied(&state, approval_id.as_deref())?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}:model:{}", record.model),
        format!("从供应商 {provider_id} 可路由列表移除模型 {}", record.model),
        "warning",
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "providerId": provider_id,
        "model": provider_model_row(&record),
        "provider": provider_row_by_id(&state, &provider_id)?,
    })))
}

fn ensure_organization_approved_provider(
    state: &AppState,
    record: &ProviderOverrideRecord,
) -> Result<(), AppError> {
    let catalog = state.config.snapshot();
    let approved = catalog.providers.get(&record.id).ok_or_else(|| {
        AppError::Forbidden(
            "provider is not present in the organization-reviewed catalog; arbitrary compatible URLs are prohibited"
                .to_owned(),
        )
    })?;
    let candidate = provider_record_to_config(record)?;
    if candidate.protocol != approved.protocol || candidate.base_url != approved.base_url {
        return Err(AppError::Forbidden(
            "provider protocol and base URL must exactly match the organization-reviewed catalog"
                .to_owned(),
        ));
    }
    if candidate
        .models
        .iter()
        .any(|model| !approved.models.contains(model))
        || candidate.passthrough_unknown_models != approved.passthrough_unknown_models
    {
        return Err(AppError::Forbidden(
            "provider models must be an explicit subset of the organization-reviewed catalog"
                .to_owned(),
        ));
    }
    Ok(())
}

fn ensure_organization_approved_model(
    state: &AppState,
    provider_id: &str,
    model: &str,
) -> Result<(), AppError> {
    let catalog = state.config.snapshot();
    let approved = catalog
        .providers
        .get(provider_id)
        .ok_or_else(|| AppError::Forbidden("provider is not organization-approved".to_owned()))?;
    if approved.models.iter().any(|approved| approved == model) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "model is not present in the organization-reviewed provider catalog".to_owned(),
        ))
    }
}

pub(super) async fn admin_create_provider_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderCredentialWriteBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    ensure_provider_exists(&state, &provider_id)?;
    let credential_id = body.id.unwrap_or_else(|| slugify_credential_id(&body.name));
    let record = state
        .control
        .upsert_provider_credential(ProviderCredentialRecord {
            id: credential_id,
            provider_id: provider_id.clone(),
            name: body.name,
            api_key_env: body.api_key_env,
            base_url: body.base_url,
            status: body.status.unwrap_or_else(|| "active".to_owned()),
            created_at_ms: 0,
            updated_at_ms: 0,
        })?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}:credential:{}", record.id),
        format!("新增供应商 {provider_id} 账号 {}", record.name),
        "info",
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "credential": provider_credential_row(&record, false),
        "provider": provider_row_by_id(&state, &provider_id)?,
    })))
}

pub(super) async fn admin_update_provider_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((provider_id, credential_id)): Path<(String, String)>,
    Json(body): Json<ProviderCredentialWriteBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    ensure_provider_exists(&state, &provider_id)?;
    let active_id = state
        .control
        .provider_control_snapshot()
        .active_provider_credentials
        .get(&provider_id)
        .cloned();
    let record = state
        .control
        .upsert_provider_credential(ProviderCredentialRecord {
            id: credential_id,
            provider_id: provider_id.clone(),
            name: body.name,
            api_key_env: body.api_key_env,
            base_url: body.base_url,
            status: body.status.unwrap_or_else(|| "active".to_owned()),
            created_at_ms: 0,
            updated_at_ms: 0,
        })?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}:credential:{}", record.id),
        format!("更新供应商 {provider_id} 账号 {}", record.name),
        "info",
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "credential": provider_credential_row(&record, active_id.as_deref() == Some(record.id.as_str())),
        "provider": provider_row_by_id(&state, &provider_id)?,
    })))
}

pub(super) async fn admin_select_provider_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((provider_id, credential_id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    ensure_provider_exists(&state, &provider_id)?;
    let record = state
        .control
        .set_active_provider_credential(&provider_id, &credential_id)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}:credential:{}", record.id),
        format!("切换供应商 {provider_id} 当前账号为 {}", record.name),
        "info",
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "credential": provider_credential_row(&record, true),
        "provider": provider_row_by_id(&state, &provider_id)?,
    })))
}

pub(super) async fn admin_set_provider_credential_pool_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<ProviderCredentialPoolBody>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    ensure_provider_exists(&state, &provider_id)?;
    let mode = state
        .control
        .set_provider_credential_pool_mode(&provider_id, &body.mode)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}:credential-pool"),
        format!("更新供应商 {provider_id} 号池策略为 {mode}"),
        "info",
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "providerId": provider_id,
        "credentialPoolMode": mode,
        "provider": provider_row_by_id(&state, &provider_id)?,
    })))
}

pub(super) async fn admin_delete_provider_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((provider_id, credential_id)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    ensure_provider_exists(&state, &provider_id)?;
    let record = state
        .control
        .delete_provider_credential(&provider_id, &credential_id)?;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("provider:{provider_id}:credential:{}", record.id),
        format!("删除供应商 {provider_id} 账号 {}", record.name),
        "warning",
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "credential": provider_credential_row(&record, false),
        "provider": provider_row_by_id(&state, &provider_id)?,
    })))
}

fn ensure_provider_exists(state: &AppState, provider_id: &str) -> Result<(), AppError> {
    let config = management_config(state);
    if config.providers.contains_key(provider_id) {
        Ok(())
    } else {
        Err(AppError::ProviderNotFound(provider_id.to_owned()))
    }
}

fn slugify_credential_id(name: &str) -> String {
    let mut slug = name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        format!("credential-{}", uuid::Uuid::new_v4().simple())
    } else {
        slug
    }
}

fn provider_body_to_record(
    provider_id: &str,
    body: ProviderWriteBody,
    current: Option<ProviderConfig>,
) -> Result<ProviderOverrideRecord, AppError> {
    let current_provider = current.as_ref();
    let id = provider_id.trim().to_ascii_lowercase();
    let display_name = body
        .display_name
        .or_else(|| current_provider.map(|provider| provider.display_name.clone()))
        .unwrap_or_else(|| id.clone());
    let protocol = body
        .protocol
        .or_else(|| {
            current_provider.map(|provider| provider_protocol_value(provider.protocol).to_owned())
        })
        .unwrap_or_else(|| "openai-compat".to_owned());
    let protocol_kind = parse_provider_protocol(&protocol)?;
    let base_url = body
        .base_url
        .or_else(|| current_provider.map(|provider| provider.base_url.clone()))
        .ok_or_else(|| AppError::InvalidRequest("baseUrl is required".to_owned()))?;
    let default_model = body
        .default_model
        .or_else(|| current_provider.map(|provider| provider.default_model.clone()))
        .ok_or_else(|| AppError::InvalidRequest("defaultModel is required".to_owned()))?;
    let mut models = body
        .models
        .or_else(|| current_provider.map(|provider| provider.models.clone()))
        .unwrap_or_default();
    if !models.contains(&default_model) {
        models.insert(0, default_model.clone());
    }
    let model_prefixes = body
        .model_prefixes
        .or_else(|| current_provider.map(|provider| provider.model_prefixes.clone()))
        .unwrap_or_default();
    let api_key_env = resolve_provider_api_key_env(
        body.api_key_env,
        body.clear_api_key_env.unwrap_or(false),
        current_provider.and_then(|provider| provider.api_key_env.as_deref()),
    )?;
    let max_tokens_field = body
        .max_tokens_field
        .or_else(|| {
            current_provider
                .map(|provider| max_tokens_field_value(provider.max_tokens_field).to_owned())
        })
        .unwrap_or_else(|| "max_completion_tokens".to_owned());
    parse_max_tokens_field(&max_tokens_field)?;
    let fidelity_mode = body
        .fidelity_mode
        .or_else(|| {
            current_provider.map(|provider| fidelity_mode_value(provider.fidelity_mode).to_owned())
        })
        .unwrap_or_else(|| "best_effort".to_owned());
    parse_fidelity_mode(&fidelity_mode)?;
    let deduplicate_stream_text = body
        .deduplicate_stream_text
        .or_else(|| current_provider.map(|provider| provider.deduplicate_stream_text))
        .unwrap_or(false);
    let buffer_stream_text = body
        .buffer_stream_text
        .or_else(|| current_provider.map(|provider| provider.buffer_stream_text))
        .unwrap_or(false);
    let tool_use = body
        .tool_use
        .or_else(|| current_provider.map(|provider| provider.tool_use))
        .unwrap_or_else(|| {
            ToolUseConfig::default_for_provider(&id, protocol_kind, deduplicate_stream_text)
        });
    validate_provider_tool_use(&id, protocol_kind, &tool_use)?;

    let record = ProviderOverrideRecord {
        id,
        display_name,
        protocol,
        base_url,
        api_key_env,
        api_key_required: body
            .api_key_required
            .or_else(|| current_provider.map(|provider| provider.api_key_required))
            .unwrap_or(true),
        default_model,
        models,
        model_prefixes,
        passthrough_unknown_models: body
            .passthrough_unknown_models
            .or_else(|| current_provider.map(|provider| provider.passthrough_unknown_models))
            .unwrap_or(false),
        max_tokens_field,
        deduplicate_stream_text,
        buffer_stream_text,
        fidelity_mode,
        tool_use,
        model_profile_defaults: body
            .model_profile_defaults
            .or_else(|| current_provider.map(|provider| provider.model_profile_defaults.clone()))
            .unwrap_or_default(),
        model_profiles: body
            .model_profiles
            .or_else(|| current_provider.map(|provider| provider.model_profiles.clone()))
            .unwrap_or_default(),
        reasoning: body
            .reasoning
            .or_else(|| current_provider.map(|provider| provider.reasoning.clone()))
            .unwrap_or_default(),
        sampling: body
            .sampling
            .or_else(|| current_provider.map(|provider| provider.sampling.clone()))
            .unwrap_or_default(),
        token_counting: body
            .token_counting
            .or_else(|| current_provider.map(|provider| provider.token_counting.clone()))
            .unwrap_or_default(),
        static_headers: body
            .static_headers
            .or_else(|| current_provider.map(|provider| provider.static_headers.clone()))
            .unwrap_or_default(),
        request_timeout_ms: body
            .request_timeout_ms
            .or_else(|| current_provider.and_then(|provider| provider.request_timeout_ms)),
        stream_idle_timeout_ms: body
            .stream_idle_timeout_ms
            .or_else(|| current_provider.and_then(|provider| provider.stream_idle_timeout_ms)),
        retry: body
            .retry
            .or_else(|| current_provider.map(|provider| provider.retry.clone()))
            .unwrap_or_default(),
        pricing: body
            .pricing
            .or_else(|| current_provider.and_then(|provider| provider.pricing)),
        model_pricing: body
            .model_pricing
            .or_else(|| current_provider.map(|provider| provider.model_pricing.clone()))
            .unwrap_or_default(),
        trust_upstream_cost: body
            .trust_upstream_cost
            .or_else(|| current_provider.map(|provider| provider.trust_upstream_cost))
            .unwrap_or(false),
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    for (model, card) in &record.model_pricing {
        crate::pricing::validate_model_pricing_card(model, &record.models, card)
            .map_err(AppError::InvalidRequest)?;
    }
    Ok(record)
}

fn resolve_provider_api_key_env(
    requested: Option<String>,
    clear: bool,
    current: Option<&str>,
) -> Result<Option<String>, AppError> {
    if clear {
        if requested
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(AppError::InvalidRequest(
                "apiKeyEnv and clearApiKeyEnv=true cannot be used together".to_owned(),
            ));
        }
        return Ok(None);
    }
    Ok(requested.or_else(|| current.map(str::to_owned)))
}

fn validate_provider_tool_use(
    provider_id: &str,
    protocol: ProviderProtocol,
    tool_use: &ToolUseConfig,
) -> Result<(), AppError> {
    if !tool_use.supported && (tool_use.tool_choice || tool_use.parallel_tool_calls) {
        return Err(AppError::InvalidRequest(format!(
            "provider `{provider_id}` cannot enable toolChoice or parallelToolCalls when toolUse.supported=false"
        )));
    }
    if tool_use.repair_invalid_arguments
        && (protocol != ProviderProtocol::OpenaiCompat
            || tool_use.response_validation != ToolResponseValidation::Strict)
    {
        return Err(AppError::InvalidRequest(format!(
            "provider `{provider_id}` can enable repairInvalidArguments only for an OpenAI-compatible provider with strict response validation"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_inconsistent_tool_use_capability_matrix() {
        let tool_use = ToolUseConfig {
            supported: false,
            tool_choice: true,
            parallel_tool_calls: false,
            ..ToolUseConfig::default()
        };

        let error = validate_provider_tool_use("local", ProviderProtocol::OpenaiCompat, &tool_use)
            .unwrap_err();

        assert!(error.to_string().contains("toolUse.supported=false"));
    }

    #[test]
    fn rejects_argument_repair_without_strict_openai_contract() {
        let tool_use = ToolUseConfig {
            repair_invalid_arguments: true,
            response_validation: ToolResponseValidation::BestEffort,
            ..ToolUseConfig::default()
        };
        assert!(
            validate_provider_tool_use("local", ProviderProtocol::OpenaiCompat, &tool_use)
                .unwrap_err()
                .to_string()
                .contains("strict response validation")
        );

        let tool_use = ToolUseConfig {
            response_validation: ToolResponseValidation::Strict,
            ..tool_use
        };
        assert!(
            validate_provider_tool_use("native", ProviderProtocol::Anthropic, &tool_use).is_err()
        );
    }

    #[test]
    fn provider_api_key_env_has_explicit_clear_semantics() {
        assert_eq!(
            resolve_provider_api_key_env(None, false, Some("OLD_API_KEY")).unwrap(),
            Some("OLD_API_KEY".to_owned())
        );
        assert_eq!(
            resolve_provider_api_key_env(None, true, Some("OLD_API_KEY")).unwrap(),
            None
        );
        assert!(resolve_provider_api_key_env(Some("NEW_API_KEY".to_owned()), true, None).is_err());
    }
}
