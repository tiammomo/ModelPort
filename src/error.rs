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

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    kind: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::Auth => StatusCode::UNAUTHORIZED,
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            AppError::MissingSecret(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::ProviderNotFound(_) => StatusCode::BAD_REQUEST,
            AppError::Transport(_) | AppError::UpstreamProtocol(_) => StatusCode::BAD_GATEWAY,
            AppError::Upstream { status, .. } => {
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            AppError::Io(_) | AppError::Json(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let kind = match status {
            StatusCode::UNAUTHORIZED => "authentication_error",
            StatusCode::BAD_REQUEST => "invalid_request_error",
            StatusCode::BAD_GATEWAY => "upstream_error",
            _ => "server_error",
        };

        (
            status,
            Json(ErrorBody {
                error: ErrorDetail {
                    kind,
                    message: self.to_string(),
                },
            }),
        )
            .into_response()
    }
}
