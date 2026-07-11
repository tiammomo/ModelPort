use axum::{
    Json,
    http::HeaderMap,
    response::{
        IntoResponse, Response,
        sse::{KeepAlive, Sse},
    },
};
use serde_json::{Value, json};

use crate::{
    config::{FidelityMode, ResolvedProvider, ToolArgumentMode},
    error::AppError,
    http::Header,
    pricing::{self, USAGE_HEADER},
    providers::openai_stream::{openai_complete_to_anthropic_stream, openai_stream_to_anthropic},
    routes::AppState,
    types::{
        AnthropicRequest, anthropic_to_openai_request, openai_response_to_anthropic,
        validate_anthropic_to_openai_fidelity,
    },
};

pub async fn messages(
    state: AppState,
    resolved: ResolvedProvider,
    request: AnthropicRequest,
    client_headers: &HeaderMap,
) -> Result<Response, AppError> {
    let headers = headers(&resolved.provider, client_headers)?;
    let url = resolved.provider.endpoint("/chat/completions");

    if resolved.provider.fidelity_mode == FidelityMode::Strict {
        if resolved.provider.buffer_stream_text || resolved.provider.deduplicate_stream_text {
            return Err(AppError::Config(
                "fidelity_mode=strict cannot be combined with stream text rewriting".to_owned(),
            ));
        }
        validate_anthropic_to_openai_fidelity(&request)?;
    }

    if request.stream.unwrap_or(false) {
        if resolved.provider.buffer_stream_text {
            let mut body = anthropic_to_openai_request(
                &request,
                &resolved.model,
                false,
                resolved.provider.max_tokens_field,
            )?;
            apply_buffered_generation_defaults(&mut body);
            let upstream = state.transport.post_json(&url, &headers, &body).await?;
            let usage = pricing::openai_usage_if_present(&upstream);
            let message = openai_response_to_anthropic(&upstream, &request.model)?;
            let events = openai_complete_to_anthropic_stream(message, request.model.clone());
            let mut response = Sse::new(events)
                .keep_alive(KeepAlive::default())
                .into_response();
            if let Some(usage) = usage {
                response.headers_mut().insert(
                    USAGE_HEADER,
                    pricing::usage_header_value(&resolved.model, usage)?,
                );
            }
            return Ok(response);
        }

        let deduplicate_stream_text = resolved.provider.deduplicate_stream_text;
        let deduplicate_tool_arguments = matches!(
            resolved.provider.tool_use.streaming_arguments,
            ToolArgumentMode::Cumulative | ToolArgumentMode::BestEffort
        );
        let body = anthropic_to_openai_request(
            &request,
            &resolved.model,
            true,
            resolved.provider.max_tokens_field,
        )?;
        let frames = state.transport.post_json_sse(url, headers, body).await?;
        let events = openai_stream_to_anthropic(
            frames,
            request.model.clone(),
            deduplicate_stream_text,
            deduplicate_tool_arguments,
        );
        Ok(Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let body = anthropic_to_openai_request(
            &request,
            &resolved.model,
            false,
            resolved.provider.max_tokens_field,
        )?;
        let response = state.transport.post_json(&url, &headers, &body).await?;
        let usage = pricing::openai_usage_if_present(&response);
        let mut response =
            Json(openai_response_to_anthropic(&response, &request.model)?).into_response();
        if let Some(usage) = usage {
            response.headers_mut().insert(
                USAGE_HEADER,
                pricing::usage_header_value(&resolved.model, usage)?,
            );
        }
        Ok(response)
    }
}

fn apply_buffered_generation_defaults(body: &mut Value) {
    let Some(body) = body.as_object_mut() else {
        return;
    };

    body.entry("temperature".to_owned()).or_insert(json!(0.2));
}

fn headers(
    provider: &crate::config::ProviderConfig,
    client_headers: &HeaderMap,
) -> Result<Vec<Header>, AppError> {
    let mut headers = Vec::new();
    if let Some(api_key) = provider.api_key()? {
        headers.push(("Authorization".to_owned(), format!("Bearer {api_key}")));
    }
    if let Some(request_id) = client_headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
    {
        headers.push(("x-request-id".to_owned(), request_id.to_owned()));
    }
    Ok(headers)
}
