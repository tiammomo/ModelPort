use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde_json::{Value, json};

use crate::auth::{CreateUserInput, UpdateUserInput};

use super::*;

pub(super) async fn admin_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let actor = require_console_user(&state, &headers)?;
    let requests = state
        .metrics
        .snapshot()
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    let mut users = state.auth.list_users(requests);
    if actor.role == "user" {
        users.retain(|user| user.id == actor.id);
    }
    for user in &mut users {
        user.api_key_count = state.control.active_api_key_count(&user.id);
    }
    Ok(Json(json!(users)))
}

pub(super) async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateUserInput>,
) -> Result<Json<Value>, AppError> {
    let actor = require_admin_write_user(&state, &headers)?;
    let auth = state.auth.clone();
    let user = tokio::task::spawn_blocking(move || auth.create_user(body))
        .await
        .map_err(|error| AppError::Config(format!("password worker failed: {error}")))??;
    record_admin_activity(
        &state,
        &actor,
        "config_change",
        format!("user:{}", user.id),
        format!("创建用户 {}", user.username),
        "info",
    );
    Ok(Json(json!(user)))
}

pub(super) async fn admin_update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(body): Json<UpdateUserInput>,
) -> Result<Json<Value>, AppError> {
    let current_user = require_admin_write_user(&state, &headers)?;
    let was_inactive = state
        .auth
        .user_by_id(&user_id)
        .is_some_and(|user| user.status != "active");
    let reactivating = was_inactive && body.status.as_deref() == Some("active");
    if reactivating {
        // Re-enabling an account must not resurrect keys that were revoked
        // when it was disabled. Commit the fail-closed control mutation first.
        state.control.delete_user_resources(&user_id)?;
    }
    let auth = state.auth.clone();
    let update_user_id = user_id.clone();
    let current_user_id = current_user.id.clone();
    let user = tokio::task::spawn_blocking(move || {
        auth.update_user(&update_user_id, &current_user_id, body)
    })
    .await
    .map_err(|error| AppError::Config(format!("password worker failed: {error}")))??;
    if user.status != "active" {
        state.control.delete_user_resources(&user.id)?;
    }
    record_admin_activity(
        &state,
        &current_user,
        "config_change",
        format!("user:{user_id}"),
        format!("更新用户 {} ({})", user.username, user.role),
        "info",
    );
    Ok(Json(json!(user)))
}

pub(super) async fn admin_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let current_user = require_admin_write_user(&state, &headers)?;
    state.auth.delete_user(&user_id, &current_user.id)?;
    state.control.delete_user_resources(&user_id)?;
    record_admin_activity(
        &state,
        &current_user,
        "config_change",
        format!("user:{user_id}"),
        format!("删除用户 {user_id} 并回收相关资源"),
        "warning",
    );
    Ok(Json(json!({ "ok": true })))
}
