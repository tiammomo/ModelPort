use axum::{
    Router,
    routing::{get, post, put},
};

use super::{
    AppState, admin_api_keys, admin_create_quota, admin_delete_quota, admin_delete_team,
    admin_quotas, admin_teams, admin_update_quota, admin_update_team, admin_upsert_team,
    admin_users,
};

#[cfg(test)]
use super::route_contract::RouteContract;

#[cfg(test)]
pub(super) const ROUTES: &[RouteContract] = &[
    RouteContract::new("admin-identity", "/admin/teams", &["GET", "POST"]),
    RouteContract::new(
        "admin-identity",
        "/admin/teams/{team_id}",
        &["PUT", "DELETE"],
    ),
    RouteContract::new("admin-identity", "/admin/users", &["GET", "POST"]),
    RouteContract::new(
        "admin-identity",
        "/admin/users/{user_id}",
        &["PUT", "DELETE"],
    ),
    RouteContract::new("admin-identity", "/admin/api-keys", &["GET", "POST"]),
    RouteContract::new(
        "admin-identity",
        "/admin/api-keys/{key_id}/disable",
        &["POST"],
    ),
    RouteContract::new(
        "admin-identity",
        "/admin/api-keys/{key_id}/rotate",
        &["POST"],
    ),
    RouteContract::new(
        "admin-identity",
        "/admin/api-keys/{key_id}/rotate/{replacement_id}",
        &["POST", "DELETE"],
    ),
    RouteContract::new(
        "admin-identity",
        "/admin/users/{user_id}/api-keys",
        &["GET", "POST"],
    ),
    RouteContract::new(
        "admin-identity",
        "/admin/api-keys/{key_id}",
        &["PUT", "DELETE"],
    ),
    RouteContract::new("admin-identity", "/admin/api-keys/{key_id}/scope", &["PUT"]),
    RouteContract::new("admin-identity", "/admin/quotas", &["GET", "POST"]),
    RouteContract::new(
        "admin-identity",
        "/admin/quotas/{quota_id}",
        &["PUT", "DELETE"],
    ),
];

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/teams", get(admin_teams).post(admin_upsert_team))
        .route(
            "/admin/teams/{team_id}",
            put(admin_update_team).delete(admin_delete_team),
        )
        .route(
            "/admin/users",
            get(admin_users::admin_users).post(admin_users::admin_create_user),
        )
        .route(
            "/admin/users/{user_id}",
            put(admin_users::admin_update_user).delete(admin_users::admin_delete_user),
        )
        .route(
            "/admin/api-keys",
            get(admin_api_keys::admin_api_keys).post(admin_api_keys::admin_create_api_key),
        )
        .route(
            "/admin/api-keys/{key_id}/disable",
            post(admin_api_keys::admin_revoke_api_key),
        )
        .route(
            "/admin/api-keys/{key_id}/rotate",
            post(admin_api_keys::admin_rotate_api_key),
        )
        .route(
            "/admin/api-keys/{key_id}/rotate/{replacement_id}",
            post(admin_api_keys::admin_confirm_api_key_rotation)
                .delete(admin_api_keys::admin_cancel_api_key_rotation),
        )
        .route(
            "/admin/users/{user_id}/api-keys",
            get(admin_api_keys::admin_user_api_keys).post(admin_api_keys::admin_create_api_key),
        )
        .route(
            "/admin/api-keys/{key_id}",
            put(admin_api_keys::admin_update_api_key).delete(admin_api_keys::admin_delete_api_key),
        )
        .route(
            "/admin/api-keys/{key_id}/scope",
            put(admin_api_keys::admin_bind_api_key_scope),
        )
        .route("/admin/quotas", get(admin_quotas).post(admin_create_quota))
        .route(
            "/admin/quotas/{quota_id}",
            put(admin_update_quota).delete(admin_delete_quota),
        )
}
