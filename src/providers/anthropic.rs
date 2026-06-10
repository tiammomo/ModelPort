use axum::{
    Json,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::StreamExt;

use crate::{
    config::ResolvedProvider,
    error::AppError,
    http::{Header, SseFrame},
    pricing::{self, USAGE_HEADER},
    routes::AppState,
    types::{AnthropicRequest, anthropic_error_event, anthropic_request_value},
};

pub async fn messages(
    state: AppState,
    resolved: ResolvedProvider,
    request: AnthropicRequest,
) -> Result<Response, AppError> {
    let body = anthropic_request_value(&request, &resolved.model)?;
    let headers = headers(&resolved.provider)?;
    let url = resolved.provider.endpoint("/v1/messages");

    if request.stream.unwrap_or(false) {
        let frames = state.transport.post_json_sse(url, headers, body);
        let events = frames.map(|result| match result {
            Ok(frame) => Ok(frame_to_event(frame)),
            Err(err) => anthropic_error_event(&err),
        });
        Ok(Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let response = state.transport.post_json(&url, &headers, &body).await?;
        let usage = pricing::anthropic_usage(&response);
        let mut response = Json(response).into_response();
        response.headers_mut().insert(
            USAGE_HEADER,
            pricing::usage_header_value(&resolved.model, usage)?,
        );
        Ok(response)
    }
}

fn headers(provider: &crate::config::ProviderConfig) -> Result<Vec<Header>, AppError> {
    let Some(api_key) = provider.api_key()? else {
        return Ok(vec![(
            "anthropic-version".to_owned(),
            "2023-06-01".to_owned(),
        )]);
    };

    Ok(vec![
        ("x-api-key".to_owned(), api_key.to_owned()),
        ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
    ])
}

fn frame_to_event(frame: SseFrame) -> Event {
    let mut event = Event::default().data(frame.data);
    if let Some(name) = frame.event {
        event = event.event(name);
    }
    event
}
