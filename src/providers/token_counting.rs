use axum::{Json, http::HeaderMap};
use serde_json::{Value, json};

use crate::{
    config::{ProviderProtocol, ResolvedProvider},
    error::AppError,
    routes::AppState,
    types::{AnthropicCountTokensRequest, anthropic_count_tokens_request_value},
};

pub async fn count_tokens(
    state: AppState,
    resolved: ResolvedProvider,
    request: AnthropicCountTokensRequest,
    client_headers: &HeaderMap,
) -> Result<Json<Value>, AppError> {
    let input_tokens = input_tokens(state, resolved, request, client_headers).await?;
    Ok(Json(json!({"input_tokens": input_tokens})))
}

pub async fn input_tokens(
    state: AppState,
    resolved: ResolvedProvider,
    request: AnthropicCountTokensRequest,
    client_headers: &HeaderMap,
) -> Result<u64, AppError> {
    let mut body = anthropic_count_tokens_request_value(&request, &resolved.model)?;
    let (url, headers) = match resolved.provider.protocol {
        ProviderProtocol::Anthropic => (
            resolved.provider.endpoint("/v1/messages/count_tokens"),
            super::anthropic::headers(&resolved.provider, client_headers)?,
        ),
        ProviderProtocol::OpenaiCompat => {
            super::openai_compat::apply_reasoning_config(
                &request.as_message_request(),
                &resolved.provider.reasoning,
                &mut body,
            )?;
            (
                resolved.provider.endpoint("/messages/count_tokens"),
                super::openai_compat::headers(&resolved.provider, client_headers)?,
            )
        }
    };
    let upstream = state.transport.post_json(&url, &headers, &body).await?;
    let input_tokens = upstream
        .get("input_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            AppError::UpstreamProtocol(
                "token-counting response is missing integer input_tokens".to_owned(),
            )
        })?;

    Ok(input_tokens)
}
