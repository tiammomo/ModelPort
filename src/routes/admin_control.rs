use axum::{
    Router,
    routing::{delete, get, post},
};

use super::{
    AppState, admin_aliases, admin_create_alias, admin_delete_alias, admin_reload_config,
    admin_router_status, admin_settings, admin_test_provider, admin_update_settings,
};

#[cfg(test)]
use super::route_contract::RouteContract;

#[cfg(test)]
pub(super) const ROUTES: &[RouteContract] = &[
    RouteContract::new("admin-control", "/admin/aliases", &["GET", "POST"]),
    RouteContract::new("admin-control", "/admin/aliases/{alias}", &["DELETE"]),
    RouteContract::new("admin-control", "/admin/settings", &["GET", "PUT"]),
    RouteContract::new("admin-control", "/admin/settings/reload-config", &["POST"]),
    RouteContract::new("admin-control", "/admin/settings/test-provider", &["POST"]),
    RouteContract::new("admin-control", "/admin/router/status", &["GET"]),
];

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/aliases",
            get(admin_aliases).post(admin_create_alias),
        )
        .route("/admin/aliases/{alias}", delete(admin_delete_alias))
        .route(
            "/admin/settings",
            get(admin_settings).put(admin_update_settings),
        )
        .route("/admin/settings/reload-config", post(admin_reload_config))
        .route("/admin/settings/test-provider", post(admin_test_provider))
        .route("/admin/router/status", get(admin_router_status))
}
