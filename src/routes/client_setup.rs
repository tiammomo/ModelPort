use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    config::ResolvedProvider,
    control::UserCatalogGrant,
    governance::{HybridMode, ProjectPolicy, ProviderBoundary},
};

use super::{
    AppError, AppState, catalog_owner, effective_config, now_millis, require_console_user,
};

#[derive(Deserialize)]
pub(super) struct SetupQuery {
    model: String,
}

/// Read-only configuration preflight for a specific key. It deliberately does
/// not contact an upstream, reserve budget, or claim to validate the caller's
/// IP, payload capabilities, live health, or remaining quota.
pub(super) async fn check(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    Query(query): Query<SetupQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    let owner = catalog_owner(&state, &actor, Some(&key_id))?;
    if query.model.trim().is_empty() || query.model.len() > 256 {
        return Err(AppError::InvalidRequest(
            "model must contain 1–256 bytes".to_owned(),
        ));
    }
    let mut result = json!({
        "apiKeyId": key_id,
        "model": query.model,
        "status": "blocked",
        "code": "key_unavailable",
        "checkedAtMs": now_millis(),
        "upstreamVerified": false,
    });
    if state
        .auth
        .user_by_id(&owner)
        .is_none_or(|user| user.status != "active")
    {
        result["code"] = json!("owner_unavailable");
        return Ok(Json(result));
    }
    let grants = state.control.user_catalog_grants(&owner, Some(&key_id));
    let Some(grant) = grants.first() else {
        return Ok(Json(result));
    };
    if super::ensure_inference_purpose(grant.purpose.as_deref()).is_err() {
        result["code"] = json!("control_only_key");
        return Ok(Json(result));
    }
    let policy = state.governance.effective_policy(&grant.tenant);
    let mode = policy.effective_mode(None, policy.default_classification)?;
    result["project"] = json!({
        "organizationId": grant.tenant.organization_id.to_string(),
        "projectId": grant.tenant.project_id.to_string(),
        "environmentId": grant.tenant.environment_id.to_string(),
    });
    result["defaultClassification"] = json!(policy.default_classification);
    result["effectiveMode"] = json!(mode);
    result["ipRestricted"] = json!(grant.policy.ip_restricted);

    let config = effective_config(&state);
    let candidates = if config.smart_route_group(&query.model).is_some() {
        super::provider_view::catalog_smart_alias_candidates(
            &state,
            &config,
            &query.model,
            Some(&grant.policy),
            &policy,
        )
    } else {
        config.resolve(&query.model).into_iter().collect()
    };
    let mut code = "model_unavailable";
    for resolved in candidates {
        code = candidate_status(&state, grant, &policy, mode, &query.model, &resolved);
        if code == "ready" {
            break;
        }
    }
    result["code"] = json!(code);
    if code == "ready" {
        result["status"] = json!("ready");
    }
    Ok(Json(result))
}

fn candidate_status(
    state: &AppState,
    grant: &UserCatalogGrant,
    policy: &ProjectPolicy,
    mode: HybridMode,
    requested: &str,
    resolved: &ResolvedProvider,
) -> &'static str {
    if grant
        .policy
        .enforce_route(requested, &resolved.model, &resolved.provider_id)
        .is_err()
    {
        return "model_not_allowed";
    }
    if policy.enforce_attempt(resolved).is_err() {
        return "project_policy_denied";
    }
    if mode == HybridMode::LocalStrict
        && ProviderBoundary::for_resolved(resolved) == ProviderBoundary::Cloud
    {
        return "local_only";
    }
    if !resolved.provider.models.contains(&resolved.model) {
        return "model_unavailable";
    }
    let credential_ready = state
        .control
        .provider_credential_route_available(&resolved.provider_id)
        .unwrap_or_else(|| {
            !resolved.provider.api_key_required
                || resolved.provider.api_key().ok().flatten().is_some()
        });
    if !credential_ready {
        return "missing_credential";
    }
    "ready"
}
