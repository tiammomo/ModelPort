use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

use crate::pricing::TokenUsageBreakdown;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("client authentication failed")]
    Auth,
    #[error("configuration error: {0}")]
    Config(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("idempotency conflict: {0}")]
    IdempotencyConflict(String),
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after_secs: u64,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("missing secret environment variable: {0}")]
    MissingSecret(String),
    #[error("service not ready: {0}")]
    NotReady(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("upstream returned HTTP {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error("upstream protocol error: {0}")]
    UpstreamProtocol(String),
    #[error(
        "upstream tool arguments failed strict schema validation at {instance_path} (schema path {schema_path}; value [redacted])"
    )]
    ToolArgumentsInvalid {
        instance_path: String,
        schema_path: String,
        usage: Option<TokenUsageBreakdown>,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl AppError {
    pub(crate) fn http_status(&self) -> StatusCode {
        status_code(self)
    }

    pub(crate) fn with_tool_argument_usage(self, usage: Option<TokenUsageBreakdown>) -> Self {
        match self {
            Self::ToolArgumentsInvalid {
                instance_path,
                schema_path,
                ..
            } => Self::ToolArgumentsInvalid {
                instance_path,
                schema_path,
                usage,
            },
            other => other,
        }
    }

    pub(crate) fn tool_argument_usage(&self) -> Option<TokenUsageBreakdown> {
        match self {
            Self::ToolArgumentsInvalid { usage, .. } => *usage,
            _ => None,
        }
    }

    /// Returns a bounded-detail message suitable for persistent usage, ledger,
    /// and provider-health records. The HTTP response can retain actionable
    /// detail for the authenticated caller, but durable telemetry must not
    /// retain request values, validation paths, provider bodies, URLs, or
    /// storage diagnostics that may contain tenant data or credentials.
    pub(crate) fn audit_message(&self) -> String {
        match self {
            Self::Auth => "client authentication failed".to_owned(),
            Self::Config(_) => "configuration error [details redacted]".to_owned(),
            Self::Database(_) => "database error [details redacted]".to_owned(),
            Self::Forbidden(_) => "forbidden [details redacted]".to_owned(),
            Self::IdempotencyConflict(_) => "idempotency conflict [details redacted]".to_owned(),
            Self::QuotaExceeded(_) => "quota exceeded [details redacted]".to_owned(),
            Self::RateLimited { .. } => "rate limited [details redacted]".to_owned(),
            Self::InvalidRequest(_) => "invalid request [details redacted]".to_owned(),
            Self::MissingSecret(_) => {
                "missing secret environment variable [name redacted]".to_owned()
            }
            Self::NotReady(_) => "service not ready [details redacted]".to_owned(),
            Self::NotFound(_) => "not found [details redacted]".to_owned(),
            Self::ProviderNotFound(_) => "provider not found [details redacted]".to_owned(),
            Self::Transport(message) if message.to_ascii_lowercase().contains("timed out") => {
                "upstream transport timed out [details redacted]".to_owned()
            }
            Self::Transport(_) => "upstream transport error [details redacted]".to_owned(),
            Self::Upstream { status, body } => {
                format!(
                    "upstream returned HTTP {status}: {}",
                    upstream_audit_category(*status, body)
                )
            }
            Self::UpstreamProtocol(message) if contains_tool_protocol_marker(message) => {
                "upstream tool protocol error [details redacted]".to_owned()
            }
            Self::UpstreamProtocol(_) => "upstream protocol error [details redacted]".to_owned(),
            Self::ToolArgumentsInvalid { .. } => {
                "upstream tool arguments failed strict schema validation \
                 [value and validation paths redacted]"
                    .to_owned()
            }
            Self::Io(_) => "I/O error [details redacted]".to_owned(),
            Self::Json(_) => "JSON error [details redacted]".to_owned(),
        }
    }
}

fn upstream_audit_category(status: u16, body: &str) -> &'static str {
    let normalized = body.to_ascii_lowercase();
    if [
        "insufficient_balance",
        "insufficient balance",
        "insufficient account balance",
        "balance not enough",
        "余额不足",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        "insufficient balance [body redacted]"
    } else if status == 401 || status == 403 {
        "authentication or authorization failed [body redacted]"
    } else if status == 429 || normalized.contains("rate limit") {
        "rate limit [body redacted]"
    } else {
        "body [redacted]"
    }
}

fn contains_tool_protocol_marker(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    ["tool", "function", "input_json", "tool_use", "tool_result"]
        .iter()
        .any(|marker| normalized.contains(marker))
}

pub(crate) fn audit_safe_persisted_error(message: &str) -> String {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("timed out") || normalized.contains("timeout") {
        "request failed: timeout [details redacted]".to_owned()
    } else if [
        "insufficient_balance",
        "insufficient balance",
        "insufficient account balance",
        "balance not enough",
        "余额不足",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        "request failed: insufficient balance [details redacted]".to_owned()
    } else if normalized.contains("rate limit") || normalized.contains("rate_limited") {
        "request failed: rate limit [details redacted]".to_owned()
    } else if contains_tool_protocol_marker(message) || normalized.contains("schema path") {
        "request failed: tool protocol error [details redacted]".to_owned()
    } else if normalized.contains("authentication")
        || normalized.contains("authorization")
        || normalized.contains("api key")
    {
        "request failed: authentication or authorization [details redacted]".to_owned()
    } else if normalized.contains("configuration") || normalized.contains("missing secret") {
        "request failed: configuration [details redacted]".to_owned()
    } else {
        "request failed [details redacted]".to_owned()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.to_string())
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    kind: &'static str,
    code: &'static str,
    status: u16,
    message: String,
    hint: &'static str,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let retry_after_secs = match &self {
            AppError::RateLimited {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        };
        let message = self.to_string();
        let status = self.http_status();

        let kind = match &self {
            AppError::Auth => "authentication_error",
            AppError::Forbidden(_) => "forbidden_error",
            AppError::IdempotencyConflict(_) => "invalid_request_error",
            AppError::QuotaExceeded(_) => "quota_exceeded",
            AppError::RateLimited { .. } => "rate_limit_error",
            AppError::InvalidRequest(_) | AppError::ProviderNotFound(_) => "invalid_request_error",
            AppError::NotFound(_) => "not_found_error",
            AppError::NotReady(_) => "server_error",
            AppError::Transport(_)
            | AppError::Upstream { .. }
            | AppError::UpstreamProtocol(_)
            | AppError::ToolArgumentsInvalid { .. } => "upstream_error",
            AppError::Config(_)
            | AppError::Database(_)
            | AppError::MissingSecret(_)
            | AppError::Io(_)
            | AppError::Json(_) => "server_error",
        };

        let mut response = (
            status,
            Json(ErrorBody {
                error: ErrorDetail {
                    kind,
                    code: error_code(&self),
                    status: status.as_u16(),
                    message,
                    hint: error_hint(&self),
                },
            }),
        )
            .into_response();

        if let Some(retry_after_secs) = retry_after_secs
            && let Ok(value) = HeaderValue::from_str(&retry_after_secs.max(1).to_string())
        {
            response.headers_mut().insert(RETRY_AFTER, value);
        }

        response
    }
}

fn status_code(error: &AppError) -> StatusCode {
    match error {
        AppError::Auth => StatusCode::UNAUTHORIZED,
        AppError::Config(_) | AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        AppError::Forbidden(_) => StatusCode::FORBIDDEN,
        AppError::IdempotencyConflict(_) => StatusCode::CONFLICT,
        AppError::QuotaExceeded(_) | AppError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        AppError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        AppError::MissingSecret(_) => StatusCode::INTERNAL_SERVER_ERROR,
        AppError::NotReady(_) => StatusCode::SERVICE_UNAVAILABLE,
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::ProviderNotFound(_) => StatusCode::BAD_REQUEST,
        AppError::Transport(_)
        | AppError::UpstreamProtocol(_)
        | AppError::ToolArgumentsInvalid { .. } => StatusCode::BAD_GATEWAY,
        AppError::Upstream { status, .. } => {
            if (400..=599).contains(status) {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            } else {
                StatusCode::BAD_GATEWAY
            }
        }
        AppError::Io(_) | AppError::Json(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn error_code(error: &AppError) -> &'static str {
    match error {
        AppError::Auth => "auth_failed",
        AppError::Config(_) => "config_error",
        AppError::Database(_) => "database_error",
        AppError::Forbidden(_) => "forbidden",
        AppError::IdempotencyConflict(_) => "idempotency_conflict",
        AppError::QuotaExceeded(_) => "quota_exceeded",
        AppError::RateLimited { .. } => "rate_limited",
        AppError::InvalidRequest(_) => "invalid_request",
        AppError::MissingSecret(_) => "missing_secret",
        AppError::NotReady(_) => "not_ready",
        AppError::NotFound(_) => "not_found",
        AppError::ProviderNotFound(_) => "provider_not_found",
        AppError::Transport(_) => "transport_error",
        AppError::Upstream { .. } => "upstream_error",
        AppError::UpstreamProtocol(_) => "upstream_protocol_error",
        AppError::ToolArgumentsInvalid { .. } => "tool_arguments_invalid",
        AppError::Io(_) => "io_error",
        AppError::Json(_) => "json_error",
    }
}

fn error_hint(error: &AppError) -> &'static str {
    match error {
        AppError::Auth => "请重新登录控制台，或确认请求携带有效的 API Key。",
        AppError::Config(_) | AppError::MissingSecret(_) => {
            "检查环境变量、配置文件和供应商 API Key 后重启 ModelPort。"
        }
        AppError::Database(_) => {
            "检查 MODELPORT_DATABASE_URL、PostgreSQL 容器健康状态和数据库权限。"
        }
        AppError::Forbidden(_) => "当前账号权限不足，或 API Key 的归属/IP 策略拒绝了本次操作。",
        AppError::IdempotencyConflict(_) => {
            "该幂等键已被当前租户中的请求占用；请等待原请求完成，或使用新的幂等键。"
        }
        AppError::QuotaExceeded(_) => {
            "检查用户配额或 API Key 的额度限制，必要时提高限额或更换密钥。"
        }
        AppError::RateLimited { .. } => "请求速度超过本地限流护栏，请按 Retry-After 退避后重试。",
        AppError::InvalidRequest(_) => "检查表单字段、时间戳、IP/CIDR 或模型/provider 名称格式。",
        AppError::ProviderNotFound(_) => "确认该 provider 已在配置文件或环境变量中启用。",
        AppError::NotReady(_) => "检查持久化存储和运行配置，恢复后再发送流量。",
        AppError::NotFound(_) => "确认资源 ID 正确，且资源仍在当前保留窗口内。",
        AppError::Transport(_)
        | AppError::Upstream { .. }
        | AppError::UpstreamProtocol(_)
        | AppError::ToolArgumentsInvalid { .. } => {
            "上游 provider 连接失败，可先在系统设置中测试连接并查看请求日志。"
        }
        AppError::Io(_) | AppError::Json(_) => {
            "查看服务日志和控制面数据文件，确认磁盘和 JSON 数据状态正常。"
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        http::StatusCode,
        response::{IntoResponse, Response},
    };
    use serde_json::Value;

    use super::*;

    #[tokio::test]
    async fn rate_limited_response_sets_retry_after() {
        let response = AppError::RateLimited {
            message: "API key request rate limit exceeded".to_owned(),
            retry_after_secs: 7,
        }
        .into_response();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
            Some("7")
        );
        let body = response_json(response).await;
        assert_eq!(body["error"]["type"], "rate_limit_error");
        assert_eq!(body["error"]["code"], "rate_limited");
    }

    #[tokio::test]
    async fn idempotency_conflict_uses_stable_http_409_envelope() {
        let response = AppError::IdempotencyConflict(
            "the key was already used with a different request body".to_owned(),
        )
        .into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response_json(response).await;
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["code"], "idempotency_conflict");
    }

    #[tokio::test]
    async fn not_ready_response_uses_service_unavailable() {
        let response = AppError::NotReady("control storage unavailable".to_owned()).into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "not_ready");
    }

    #[tokio::test]
    async fn not_found_response_uses_standard_json_envelope() {
        let response = AppError::NotFound("request log missing".to_owned()).into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        assert_eq!(body["error"]["type"], "not_found_error");
        assert_eq!(body["error"]["code"], "not_found");
        assert_eq!(body["error"]["status"], 404);
    }

    #[tokio::test]
    async fn upstream_response_keeps_status_but_uses_upstream_type() {
        let response = AppError::Upstream {
            status: 402,
            body: "Insufficient Balance".to_owned(),
        }
        .into_response();

        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
        let body = response_json(response).await;
        assert_eq!(body["error"]["type"], "upstream_error");
        assert_eq!(body["error"]["code"], "upstream_error");
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Insufficient Balance"))
        );
    }

    #[tokio::test]
    async fn upstream_redirect_is_reported_as_bad_gateway() {
        let response = AppError::Upstream {
            status: 302,
            body: "redirects are disabled".to_owned(),
        }
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn audit_messages_redact_provider_bodies_and_tool_validation_paths() {
        let upstream = AppError::Upstream {
            status: 500,
            body: r#"{"echo":"tenant prompt","authorization":"Bearer secret"}"#.to_owned(),
        };
        let upstream_audit = upstream.audit_message();
        assert_eq!(
            upstream_audit,
            "upstream returned HTTP 500: body [redacted]"
        );
        assert!(!upstream_audit.contains("tenant prompt"));
        assert!(!upstream_audit.contains("Bearer secret"));

        let tool = AppError::ToolArgumentsInvalid {
            instance_path: "/private_customer_id".to_owned(),
            schema_path: "/properties/private_customer_id/type".to_owned(),
            usage: None,
        };
        let tool_audit = tool.audit_message();
        assert!(tool_audit.contains("tool arguments"));
        assert!(!tool_audit.contains("private_customer_id"));
        assert!(!tool_audit.contains("/properties"));
    }

    #[test]
    fn audit_messages_keep_only_safe_provider_failure_categories() {
        let balance = AppError::Upstream {
            status: 402,
            body: "Insufficient Balance for account customer@example.test".to_owned(),
        }
        .audit_message();
        assert_eq!(
            balance,
            "upstream returned HTTP 402: insufficient balance [body redacted]"
        );
        assert!(!balance.contains("customer@example.test"));
    }

    #[test]
    fn historical_audit_sanitizer_is_category_only() {
        let historical = audit_safe_persisted_error(
            "upstream tool arguments failed at /private_customer_id (schema path /properties/private_customer_id)",
        );
        assert_eq!(
            historical,
            "request failed: tool protocol error [details redacted]"
        );
        assert!(!historical.contains("private_customer_id"));
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}
