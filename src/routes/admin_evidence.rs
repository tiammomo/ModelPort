use axum::{
    Router,
    routing::{get, post},
};

use super::{
    AppState, admin_adjust_enterprise_budget, admin_audit, admin_backup, admin_dashboard,
    admin_enterprise_budget, admin_enterprise_overview, admin_enterprise_request_detail,
    admin_enterprise_requests, admin_latency, admin_log_by_id, admin_logs, admin_run_retention,
    admin_update_enterprise_budget,
};

#[cfg(test)]
use super::route_contract::RouteContract;

#[cfg(test)]
pub(super) const ROUTES: &[RouteContract] = &[
    RouteContract::new("admin-evidence", "/admin/dashboard", &["GET"]),
    RouteContract::new("admin-evidence", "/admin/audit", &["GET"]),
    RouteContract::new("admin-evidence", "/admin/backup", &["POST"]),
    RouteContract::new("admin-evidence", "/admin/retention/run", &["POST"]),
    RouteContract::new("admin-evidence", "/admin/logs", &["GET"]),
    RouteContract::new("admin-evidence", "/admin/logs/{log_id}", &["GET"]),
    RouteContract::new("admin-evidence", "/admin/latency", &["GET"]),
    RouteContract::new("admin-evidence", "/admin/enterprise/overview", &["GET"]),
    RouteContract::new(
        "admin-evidence",
        "/admin/enterprise/budget",
        &["GET", "PUT"],
    ),
    RouteContract::new(
        "admin-evidence",
        "/admin/enterprise/budget/adjustments",
        &["POST"],
    ),
    RouteContract::new("admin-evidence", "/admin/enterprise/requests", &["GET"]),
    RouteContract::new(
        "admin-evidence",
        "/admin/enterprise/requests/{ledger_id}",
        &["GET"],
    ),
];

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/dashboard", get(admin_dashboard))
        .route("/admin/audit", get(admin_audit))
        .route("/admin/backup", post(admin_backup))
        .route("/admin/retention/run", post(admin_run_retention))
        .route("/admin/logs", get(admin_logs))
        .route("/admin/logs/{log_id}", get(admin_log_by_id))
        .route("/admin/latency", get(admin_latency))
        .route("/admin/enterprise/overview", get(admin_enterprise_overview))
        .route(
            "/admin/enterprise/budget",
            get(admin_enterprise_budget).put(admin_update_enterprise_budget),
        )
        .route(
            "/admin/enterprise/budget/adjustments",
            post(admin_adjust_enterprise_budget),
        )
        .route("/admin/enterprise/requests", get(admin_enterprise_requests))
        .route(
            "/admin/enterprise/requests/{ledger_id}",
            get(admin_enterprise_request_detail),
        )
}
