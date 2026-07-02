use std::{collections::BTreeMap, pin::Pin};

use async_stream::try_stream;
use axum::{
    Json,
    http::HeaderMap,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::{Stream, StreamExt};
use serde_json::Value;

use crate::{
    config::ResolvedProvider,
    error::AppError,
    http::{Header, SseFrame, SseFrameStream},
    pricing::{self, USAGE_HEADER},
    providers::openai_compat::text_delta,
    routes::AppState,
    types::{AnthropicRequest, anthropic_error_event, anthropic_request_value},
};

pub async fn messages(
    state: AppState,
    resolved: ResolvedProvider,
    request: AnthropicRequest,
    client_headers: &HeaderMap,
) -> Result<Response, AppError> {
    let body = anthropic_request_value(&request, &resolved.model)?;
    let headers = headers(&resolved.provider, client_headers)?;
    let url = resolved.provider.endpoint("/v1/messages");

    if request.stream.unwrap_or(false) {
        let frames = state.transport.post_json_sse(url, headers, body);
        let events: Pin<Box<dyn Stream<Item = Result<Event, AppError>> + Send>> = Box::pin(
            normalize_anthropic_stream(frames, resolved.provider.deduplicate_stream_text),
        );
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

fn headers(
    provider: &crate::config::ProviderConfig,
    client_headers: &HeaderMap,
) -> Result<Vec<Header>, AppError> {
    let mut headers = Vec::new();

    if let Some(api_key) = provider.api_key()? {
        headers.push(("x-api-key".to_owned(), api_key.to_owned()));
    }

    headers.push((
        "anthropic-version".to_owned(),
        client_header(client_headers, "anthropic-version")
            .unwrap_or_else(|| "2023-06-01".to_owned()),
    ));

    for name in ["anthropic-beta", "x-request-id"] {
        if let Some(value) = client_header(client_headers, name) {
            headers.push((name.to_owned(), value));
        }
    }

    Ok(headers)
}

fn client_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn normalize_anthropic_stream(
    mut frames: SseFrameStream,
    deduplicate_stream_text: bool,
) -> impl Stream<Item = Result<Event, AppError>> + Send {
    try_stream! {
        let mut block_types = BTreeMap::<usize, String>::new();
        let mut text_seen = BTreeMap::<usize, String>::new();
        let mut processed_prefix = Vec::<String>::new();

        while let Some(result) = frames.next().await {
            let frame = match result {
                Ok(frame) => frame,
                Err(err) => {
                    yield anthropic_error_event(&err)?;
                    continue;
                }
            };

            let lines = frame
                .data
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            let skip_count = if lines.len() >= processed_prefix.len()
                && lines
                    .iter()
                    .take(processed_prefix.len())
                    .eq(processed_prefix.iter())
            {
                processed_prefix.len()
            } else {
                0
            };
            processed_prefix = lines.clone();

            for line in lines.into_iter().skip(skip_count) {
                if line == "[DONE]" {
                    yield event_with_metadata(
                        &frame,
                        frame.event.as_deref().unwrap_or("message"),
                        "[DONE]".to_owned(),
                    );
                    continue;
                }

                let Ok(mut data) = serde_json::from_str::<Value>(&line) else {
                    yield event_with_metadata(
                        &frame,
                        frame.event.as_deref().unwrap_or("message"),
                        line,
                    );
                    continue;
                };
            let event_type = data
                .get("type")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let index = data
                .get("index")
                .and_then(Value::as_u64)
                .map(|value| value as usize);

            if event_type.as_deref() == Some("content_block_start")
                && let Some(index) = index
                && let Some(block_type) = data
                    .get("content_block")
                    .and_then(|block| block.get("type"))
                    .and_then(Value::as_str)
            {
                block_types.insert(index, block_type.to_owned());
            }

            if deduplicate_stream_text
                && event_type.as_deref() == Some("content_block_delta")
                && let Some(index) = index
                && block_types.get(&index).is_some_and(|block_type| block_type == "text")
                && let Some(delta) = data.get_mut("delta").and_then(Value::as_object_mut)
                && delta.get("type").and_then(Value::as_str) == Some("text_delta")
                && let Some(text) = delta.get("text").and_then(Value::as_str)
            {
                let seen = text_seen.entry(index).or_default();
                let text = text_delta(seen, text, true);
                if text.is_empty() {
                    continue;
                }
                delta.insert("text".to_owned(), Value::String(text));
            }

            if event_type.as_deref() == Some("content_block_stop")
                && let Some(index) = index
            {
                block_types.remove(&index);
                text_seen.remove(&index);
            }

            let event_name = frame
                .event
                .as_deref()
                .or(event_type.as_deref())
                .unwrap_or("message")
                .to_owned();
            yield event_with_metadata(&frame, &event_name, serde_json::to_string(&data)?);
            }
        }
    }
}

fn event_with_metadata(frame: &SseFrame, event_name: &str, data: String) -> Event {
    let mut event = Event::default().event(event_name).data(data);
    if let Some(id) = &frame.id {
        event = event.id(id.clone());
    }
    if let Some(retry) = frame.retry {
        event = event.retry(retry);
    }
    for comment in &frame.comments {
        event = event.comment(comment.clone());
    }
    event
}
