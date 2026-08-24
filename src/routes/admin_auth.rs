use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};

use super::{
    AppState, add_no_store_header, admin_auth_methods, admin_login, admin_logout, admin_me,
    admin_oidc_callback, admin_oidc_start,
};

#[cfg(test)]
use super::route_contract::RouteContract;

#[cfg(test)]
pub(super) const ROUTES: &[RouteContract] = &[
    RouteContract::new("admin-auth", "/admin/auth/login", &["POST"]),
    RouteContract::new("admin-auth", "/admin/auth/methods", &["GET"]),
    RouteContract::new("admin-auth", "/admin/auth/oidc/start", &["GET"]),
    RouteContract::new("admin-auth", "/admin/auth/oidc/callback", &["GET"]),
    RouteContract::new("admin-auth", "/admin/auth/logout", &["POST"]),
    RouteContract::new("admin-auth", "/admin/auth/me", &["GET"]),
];

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/auth/login",
            post(admin_login)
                .layer(DefaultBodyLimit::max(16 * 1024))
                .layer(middleware::from_fn(add_no_store_header)),
        )
        .route(
            "/admin/auth/methods",
            get(admin_auth_methods).layer(middleware::from_fn(add_no_store_header)),
        )
        .route(
            "/admin/auth/oidc/start",
            get(admin_oidc_start).layer(middleware::from_fn(add_no_store_header)),
        )
        .route(
            "/admin/auth/oidc/callback",
            get(admin_oidc_callback).layer(middleware::from_fn(add_no_store_header)),
        )
        .route("/admin/auth/logout", post(admin_logout))
        .route("/admin/auth/me", get(admin_me))
}
