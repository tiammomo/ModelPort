use axum::{
    Json,
    http::StatusCode,
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
        let message = self.to_string();
        let status = status_code(&self);

        let kind = match status {
            StatusCode::UNAUTHORIZED => "authentication_error",
            StatusCode::FORBIDDEN => "forbidden_error",
            StatusCode::TOO_MANY_REQUESTS => "quota_exceeded",
            StatusCode::BAD_REQUEST => "invalid_request_error",
            StatusCode::BAD_GATEWAY => "upstream_error",
            _ => "server_error",
        };

        (
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
            .into_response()
    }
}

fn status_code(error: &AppError) -> StatusCode {
    match error {
        AppError::Auth => StatusCode::UNAUTHORIZED,
        AppError::Config(_) | AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        AppError::Forbidden(_) => StatusCode::FORBIDDEN,
        AppError::QuotaExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
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
