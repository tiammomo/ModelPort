use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use modelport_ops_protocol::{
    OpsAgentConfiguration, OpsAgentConfigurationUpdate, OpsAgentConfigurationView, OpsHeartbeat,
    OpsIncidentFeedbackInput, OpsIncidentList, OpsIncidentStatus, OpsIncidentStatusUpdate,
    OpsModelCandidate, OpsObservation, OpsSnapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::control::OpsAgentConfigRecord;

#[cfg(test)]
use super::route_contract::RouteContract;

#[cfg(test)]
pub(super) const INTERNAL_ROUTES: &[RouteContract] = &[
    RouteContract::new("internal-ops", "/internal/ops/v1/snapshot", &["GET"]),
    RouteContract::new("internal-ops", "/internal/ops/v1/observations", &["POST"]),
    RouteContract::new("internal-ops", "/internal/ops/v1/heartbeats", &["POST"]),
];

#[cfg(test)]
pub(super) const ADMIN_ROUTES: &[RouteContract] = &[
    RouteContract::new("admin-operations", "/admin/ops/incidents", &["GET"]),
    RouteContract::new(
        "admin-operations",
        "/admin/ops/configuration",
        &["GET", "PUT"],
    ),
    RouteContract::new(
        "admin-operations",
        "/admin/ops/incidents/{incident_id}",
        &["GET"],
    ),
    RouteContract::new(
        "admin-operations",
        "/admin/ops/incidents/{incident_id}/status",
        &["POST"],
    ),
    RouteContract::new(
        "admin-operations",
        "/admin/ops/incidents/{incident_id}/feedback",
        &["POST"],
    ),
];

pub(super) fn internal_router() -> Router<AppState> {
    Router::new()
        .route("/internal/ops/v1/snapshot", get(snapshot))
        .route("/internal/ops/v1/observations", post(submit_observation))
        .route("/internal/ops/v1/heartbeats", post(heartbeat))
}

pub(super) fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/admin/ops/incidents", get(admin_incidents))
        .route(
            "/admin/ops/configuration",
            get(admin_configuration).put(admin_update_configuration),
        )
        .route(
            "/admin/ops/incidents/{incident_id}",
            get(admin_incident_detail),
        )
        .route(
            "/admin/ops/incidents/{incident_id}/status",
            post(admin_update_incident_status),
        )
        .route(
            "/admin/ops/incidents/{incident_id}/feedback",
            post(admin_record_incident_feedback),
        )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IncidentListQuery {
    status: Option<OpsIncidentStatus>,
    limit: Option<usize>,
}

fn require_ops_agent(state: &AppState, headers: &HeaderMap) -> Result<ClientIdentity, AppError> {
    let identity = authenticate_client(state, headers)?;
    if !is_ops_agent_identity(&identity) {
        return Err(AppError::Forbidden(
            "a service-account key with purpose modelport_ops_agent is required".to_owned(),
        ));
    }
    Ok(identity)
}

fn is_ops_agent_identity(identity: &ClientIdentity) -> bool {
    identity.principal_type == "service_account"
        && identity.purpose.as_deref() == Some("modelport_ops_agent")
}

pub(super) async fn snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OpsSnapshot>, AppError> {
    require_ops_agent(&state, &headers)?;
    let database_ready = state.ledger.health_check().await.is_ok();
    let auth_ready = state.auth.health_check().is_ok();
    let control_ready = state.control.health_check().is_ok();
    let governance_ready = state.governance.is_ready();
    let draining = state.is_draining();
    let mut degraded_ledger_operations = state.metrics.degraded_ledger_operations();
    let (requests, ledger, recent_change_at_ms, snapshot_ready) =
        match state.ledger.ops_runtime_snapshot(300).await {
            Ok((requests, ledger, recent_change_at_ms)) => {
                (requests, ledger, recent_change_at_ms, true)
            }
            Err(_) => {
                degraded_ledger_operations.push("ops_runtime_snapshot".to_owned());
                (
                    modelport_ops_protocol::OpsRequestWindow {
                        window_seconds: 300,
                        ..Default::default()
                    },
                    Default::default(),
                    None,
                    false,
                )
            }
        };
    let gateway_ready = database_ready
        && auth_ready
        && control_ready
        && governance_ready
        && snapshot_ready
        && !draining
        && degraded_ledger_operations.is_empty();
    let mut provider_health = state.control.provider_health_rows();
    for (provider_id, provider) in &effective_config(&state).providers {
        let route_available = state
            .control
            .provider_credential_route_available(provider_id)
            .unwrap_or_else(|| {
                !provider.api_key_required || provider.api_key().ok().flatten().is_some()
            });
        if !route_available {
            provider_health
                .entry(provider_id.clone())
                .or_insert_with(|| {
                    json!({
                        "providerId": provider_id,
                        "status": "unavailable",
                        "failureKind": "missing_credential",
                        "rechargeRequired": false,
                        "consecutiveFailures": 0,
                    })
                });
        }
    }
    Ok(Json(OpsSnapshot {
        captured_at_ms: now_millis(),
        gateway_ready,
        database_ready,
        auth_ready,
        control_ready,
        governance_ready,
        draining,
        pending_finalizers: u64::try_from(state.finalizers.active()).unwrap_or(u64::MAX),
        degraded_ledger_operations,
        provider_health,
        requests,
        ledger,
        recent_change_at_ms,
        agent_configuration: runtime_configuration(&state),
    }))
}

pub(super) async fn admin_configuration(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OpsAgentConfigurationView>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(configuration_view(&state)))
}

pub(super) async fn admin_update_configuration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<OpsAgentConfigurationUpdate>,
) -> Result<Json<OpsAgentConfigurationView>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let selected_model = input
        .selected_model
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let candidate_ids = ops_model_candidates(&state, input.prefer_local)
        .into_iter()
        .map(|candidate| candidate.id)
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(model) = selected_model.as_deref()
        && !candidate_ids.contains(model)
    {
        return Err(AppError::InvalidRequest(
            "selected operations model is not currently routable".to_owned(),
        ));
    }
    if input.analysis_enabled && selected_model.is_none() {
        return Err(AppError::InvalidRequest(
            "select a routable model before enabling model analysis".to_owned(),
        ));
    }
    state.control.set_ops_agent_config(OpsAgentConfigRecord {
        enabled: input.enabled,
        analysis_enabled: input.analysis_enabled,
        selected_model: selected_model.clone(),
        prefer_local: input.prefer_local,
    })?;
    record_admin_activity(
        &state,
        &actor,
        "ops_agent_configuration_changed",
        "ops-agent",
        format!(
            "operations agent {}; model analysis {}; selected model {}",
            if input.enabled { "enabled" } else { "disabled" },
            if input.analysis_enabled {
                "enabled"
            } else {
                "disabled"
            },
            selected_model.as_deref().unwrap_or("none")
        ),
        "info",
    )
    .await;
    Ok(Json(configuration_view(&state)))
}

fn configuration_view(state: &AppState) -> OpsAgentConfigurationView {
    let configuration = runtime_configuration(state);
    let candidates = ops_model_candidates(state, configuration.prefer_local);
    let recommended_model = candidates
        .iter()
        .find(|candidate| configuration.prefer_local && candidate.local)
        .or_else(|| candidates.first())
        .map(|candidate| candidate.id.clone());
    OpsAgentConfigurationView {
        configuration,
        recommended_model,
        candidates,
    }
}

fn runtime_configuration(state: &AppState) -> OpsAgentConfiguration {
    let stored = state.control.ops_agent_config();
    let candidates = ops_model_candidates(state, stored.prefer_local);
    let selected = stored
        .selected_model
        .as_deref()
        .and_then(|model| candidates.iter().find(|candidate| candidate.id == model));
    OpsAgentConfiguration {
        enabled: stored.enabled,
        analysis_enabled: stored.analysis_enabled,
        selected_model: stored.selected_model,
        prefer_local: stored.prefer_local,
        model_ready: selected.is_some(),
        selected_model_local: selected.is_some_and(|candidate| candidate.local),
    }
}

fn ops_model_candidates(state: &AppState, prefer_local: bool) -> Vec<OpsModelCandidate> {
    let config = effective_config(state);
    let mut candidates = Vec::new();
    for provider_id in &config.provider_order {
        let Some(provider) = config.providers.get(provider_id) else {
            continue;
        };
        let route_available = state
            .control
            .provider_credential_route_available(provider_id)
            .unwrap_or_else(|| provider.api_key().ok().flatten().is_some());
        if !route_available {
            continue;
        }
        let local = provider_is_local(provider_id);
        for model in &provider.models {
            candidates.push(OpsModelCandidate {
                id: format!("{provider_id}:{model}"),
                provider_id: provider_id.clone(),
                model: model.clone(),
                display_name: format!("{} · {model}", provider.display_name),
                local,
            });
        }
    }
    if prefer_local {
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.local));
    }
    candidates
}

fn provider_is_local(provider_id: &str) -> bool {
    provider_id.starts_with("local_") || provider_id == "ollama"
}

pub(super) async fn submit_observation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(observation): Json<OpsObservation>,
) -> Result<Json<Value>, AppError> {
    let identity = require_ops_agent(&state, &headers)?;
    let incident = state
        .ledger
        .upsert_ops_observation(&observation, &identity.user_id, &identity.username)
        .await?;
    Ok(Json(json!({ "incident": incident })))
}

pub(super) async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut heartbeat): Json<OpsHeartbeat>,
) -> Result<Json<Value>, AppError> {
    let identity = require_ops_agent(&state, &headers)?;
    heartbeat.instance_id = identity.api_key_id.ok_or(AppError::Auth)?;
    state.ledger.record_ops_heartbeat(&heartbeat).await?;
    Ok(Json(json!({ "accepted": true })))
}

pub(super) async fn admin_incidents(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<IncidentListQuery>,
) -> Result<Json<OpsIncidentList>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(
        state
            .ledger
            .list_ops_incidents(query.status, query.limit.unwrap_or(100))
            .await?,
    ))
}

pub(super) async fn admin_incident_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(incident_id): Path<String>,
) -> Result<Json<modelport_ops_protocol::OpsIncidentDetail>, AppError> {
    require_admin_user(&state, &headers)?;
    Ok(Json(state.ledger.ops_incident_detail(&incident_id).await?))
}

pub(super) async fn admin_update_incident_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(incident_id): Path<String>,
    Json(update): Json<OpsIncidentStatusUpdate>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let incident = state
        .ledger
        .update_ops_incident_status(&incident_id, &update, &actor.id, &actor.username)
        .await?;
    record_admin_activity(
        &state,
        &actor,
        "ops_incident_status_changed",
        format!("ops-incident:{incident_id}"),
        format!(
            "operations incident status changed to {}",
            update.status.as_str()
        ),
        "info",
    )
    .await;
    Ok(Json(json!(incident)))
}

pub(super) async fn admin_record_incident_feedback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(incident_id): Path<String>,
    Json(feedback): Json<OpsIncidentFeedbackInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    state
        .ledger
        .record_ops_incident_feedback(&incident_id, &feedback, &actor.id, &actor.username)
        .await?;
    record_admin_activity(
        &state,
        &actor,
        "ops_incident_feedback_recorded",
        format!("ops-incident:{incident_id}"),
        "operations incident feedback recorded",
        "info",
    )
    .await;
    Ok(Json(json!({ "accepted": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_agent_boundary_requires_exact_service_account_purpose() {
        let mut identity = ControlStore::legacy_identity();
        assert!(!is_ops_agent_identity(&identity));
        identity.principal_type = "service_account".to_owned();
        identity.purpose = Some("modelport_ops_agent".to_owned());
        assert!(is_ops_agent_identity(&identity));
        identity.purpose = Some("other_agent".to_owned());
        assert!(!is_ops_agent_identity(&identity));
        identity.principal_type = "user".to_owned();
        identity.purpose = Some("modelport_ops_agent".to_owned());
        assert!(!is_ops_agent_identity(&identity));
        assert!(matches!(
            ensure_inference_identity(&identity),
            Err(AppError::Forbidden(_))
        ));
        identity.purpose = None;
        assert!(ensure_inference_identity(&identity).is_ok());
    }

    #[test]
    fn local_operations_models_are_identified_without_classifying_cloud_routes() {
        assert!(provider_is_local("ollama"));
        assert!(provider_is_local("local_vllm"));
        assert!(provider_is_local("local_team_runtime"));
        assert!(!provider_is_local("deepseek"));
        assert!(!provider_is_local("custom"));
    }
}
