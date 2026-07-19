use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde_json::{Value, json};

use crate::{
    auth::AuthStore,
    control::{BindApiKeyScopeInput, CreateApiKeyInput, UpdateApiKeyInput},
};

use super::*;

pub(super) async fn admin_api_keys(
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

pub(super) async fn admin_user_api_keys(
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

pub(super) async fn admin_create_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<CreateApiKeyInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    populate_api_key_owner(state.auth.as_ref(), &mut body)?;
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

fn populate_api_key_owner(auth: &AuthStore, body: &mut CreateApiKeyInput) -> Result<(), AppError> {
    let user = auth
        .user_by_id(&body.user_id)
        .ok_or_else(|| AppError::InvalidRequest("API key user not found".to_owned()))?;
    if user.status != "active" {
        return Err(AppError::InvalidRequest(
            "API keys can only be created for active users".to_owned(),
        ));
    }
    body.user_id = user.id;
    body.username = Some(user.username);
    Ok(())
}

pub(super) async fn admin_revoke_api_key(
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

pub(super) async fn admin_update_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(body): Json<UpdateApiKeyInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_api_key_write_user(&state, &headers)?;
    ensure_api_key_access(&state, &actor, &key_id)?;
    if actor.role != "admin" {
        validate_self_service_api_key_update(&body)?;
    }
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

pub(super) async fn admin_bind_api_key_scope(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
    Json(body): Json<BindApiKeyScopeInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let updated = state.control.bind_api_key_scope(&key_id, body)?;
    record_admin_activity(
        &state,
        &actor,
        "security_change",
        format!("api_key:{key_id}"),
        format!(
            "绑定 API Key {} 到 {}/{}/{}",
            updated.name, updated.organization_id, updated.project_id, updated.environment_id
        ),
        "warning",
    );
    Ok(Json(json!(updated)))
}

fn validate_self_service_api_key_update(body: &UpdateApiKeyInput) -> Result<(), AppError> {
    let changes_admin_policy = body.team_id.is_some()
        || body.allowed_models.is_some()
        || body.allowed_providers.is_some()
        || body.expires_at.is_some()
        || body.status.is_some()
        || body.ip_restricted.is_some()
        || body.allowed_ips.is_some()
        || body.spend_limit_usd.is_some()
        || body.rate_limited.is_some()
        || body.five_hour_limit_usd.is_some()
        || body.daily_limit_usd.is_some()
        || body.weekly_limit_usd.is_some()
        || body.monthly_limit_usd.is_some();
    if changes_admin_policy {
        return Err(AppError::Forbidden(
            "only an admin can change API key status, expiry, team, access policy, or spend limits"
                .to_owned(),
        ));
    }
    Ok(())
}

pub(super) async fn admin_delete_api_key(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::CreateUserInput;

    fn create_api_key_input(user_id: &str) -> CreateApiKeyInput {
        CreateApiKeyInput {
            user_id: user_id.to_owned(),
            username: Some("forged-name".to_owned()),
            name: "Claude Code".to_owned(),
            group: None,
            team_id: None,
            allowed_models: None,
            allowed_providers: None,
            expires_at: None,
        }
    }

    #[test]
    fn api_key_owner_must_exist_and_be_active() {
        let auth = AuthStore::for_tests();
        let active = auth
            .create_user(CreateUserInput {
                username: "active-user".to_owned(),
                email: "active@example.com".to_owned(),
                password: "strong-active-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let disabled = auth
            .create_user(CreateUserInput {
                username: "disabled-user".to_owned(),
                email: "disabled@example.com".to_owned(),
                password: "strong-disabled-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("disabled".to_owned()),
            })
            .unwrap();

        let mut body = create_api_key_input(&active.id);
        populate_api_key_owner(&auth, &mut body).unwrap();
        assert_eq!(body.user_id, active.id);
        assert_eq!(body.username.as_deref(), Some("active-user"));

        assert!(populate_api_key_owner(&auth, &mut create_api_key_input(&disabled.id)).is_err());
        assert!(populate_api_key_owner(&auth, &mut create_api_key_input("usr_missing")).is_err());
    }

    #[test]
    fn self_service_api_key_update_cannot_change_admin_policy() {
        let rename = UpdateApiKeyInput {
            name: Some("renamed".to_owned()),
            group: Some("dev".to_owned()),
            team_id: None,
            allowed_models: None,
            allowed_providers: None,
            expires_at: None,
            status: None,
            ip_restricted: None,
            allowed_ips: None,
            spend_limit_usd: None,
            rate_limited: None,
            five_hour_limit_usd: None,
            daily_limit_usd: None,
            weekly_limit_usd: None,
            monthly_limit_usd: None,
        };
        assert!(validate_self_service_api_key_update(&rename).is_ok());

        let reactivate = UpdateApiKeyInput {
            status: Some("active".to_owned()),
            ..rename
        };
        assert!(matches!(
            validate_self_service_api_key_update(&reactivate),
            Err(AppError::Forbidden(_))
        ));
    }
}
