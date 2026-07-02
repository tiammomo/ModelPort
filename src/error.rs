use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

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
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("upstream returned HTTP {status}: {body}")]
    Upstream { status: u16, body: String },
    #[error("upstream protocol error: {0}")]
    UpstreamProtocol(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl From<postgres::Error> for AppError {
    fn from(error: postgres::Error) -> Self {
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
        let status = status_code(&self);

        let kind = match &self {
            AppError::Auth => "authentication_error",
            AppError::Forbidden(_) => "forbidden_error",
            AppError::QuotaExceeded(_) => "quota_exceeded",
            AppError::RateLimited { .. } => "rate_limit_error",
            AppError::InvalidRequest(_) | AppError::ProviderNotFound(_) => "invalid_request_error",
            AppError::Transport(_) | AppError::Upstream { .. } | AppError::UpstreamProtocol(_) => {
                "upstream_error"
            }
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
        AppError::QuotaExceeded(_) | AppError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        AppError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        AppError::MissingSecret(_) => StatusCode::INTERNAL_SERVER_ERROR,
        AppError::ProviderNotFound(_) => StatusCode::BAD_REQUEST,
        AppError::Transport(_) | AppError::UpstreamProtocol(_) => StatusCode::BAD_GATEWAY,
        AppError::Upstream { status, .. } => {
            StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
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
        AppError::QuotaExceeded(_) => "quota_exceeded",
        AppError::RateLimited { .. } => "rate_limited",
        AppError::InvalidRequest(_) => "invalid_request",
        AppError::MissingSecret(_) => "missing_secret",
        AppError::ProviderNotFound(_) => "provider_not_found",
        AppError::Transport(_) => "transport_error",
        AppError::Upstream { .. } => "upstream_error",
        AppError::UpstreamProtocol(_) => "upstream_protocol_error",
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
        AppError::QuotaExceeded(_) => {
            "检查用户配额或 API Key 的额度限制，必要时提高限额或更换密钥。"
        }
        AppError::RateLimited { .. } => "请求速度超过本地限流护栏，请按 Retry-After 退避后重试。",
        AppError::InvalidRequest(_) => "检查表单字段、时间戳、IP/CIDR 或模型/provider 名称格式。",
        AppError::ProviderNotFound(_) => "确认该 provider 已在配置文件或环境变量中启用。",
        AppError::Transport(_) | AppError::Upstream { .. } | AppError::UpstreamProtocol(_) => {
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

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}
