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
    exchange::{ExchangeRequest, anthropic_response_to_openai},
    http::{Header, SseFrame, SseFrameStream},
    pricing::{self, USAGE_HEADER},
    providers::{openai_client_stream::anthropic_stream_to_openai, openai_stream::text_delta},
    routes::AppState,
    stream_lifecycle::StreamLifecycle,
    types::{AnthropicRequest, anthropic_error_event, anthropic_request_value},
};

pub async fn chat_completions(
    state: AppState,
    resolved: ResolvedProvider,
    request: ExchangeRequest,
    client_headers: &HeaderMap,
    stream_lifecycle: StreamLifecycle,
) -> Result<Response, AppError> {
    let mut body = request.to_anthropic_request(&resolved.model, request.stream)?;
    apply_provider_request_compatibility(&resolved.provider_id, &mut body);
    let headers = headers(&resolved.provider, client_headers)?;
    let url = resolved.provider.endpoint("/v1/messages");

    if request.stream {
        let include_usage = request.include_stream_usage();
        let frames = state.transport.post_json_sse(url, headers, body).await?;
        let events = anthropic_stream_to_openai(
            frames,
            request.requested_model.clone(),
            include_usage,
            stream_lifecycle,
        );
        Ok(Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let upstream = state.transport.post_json(&url, &headers, &body).await?;
        let usage = pricing::anthropic_usage_if_present(&upstream);
        let mut response = Json(anthropic_response_to_openai(
            &upstream,
            &request.requested_model,
        )?)
        .into_response();
        if let Some(usage) = usage {
            response.headers_mut().insert(
                USAGE_HEADER,
                pricing::usage_header_value(&resolved.model, usage, resolved.provider.pricing)?,
            );
        }
        Ok(response)
    }
}

fn apply_provider_request_compatibility(provider_id: &str, body: &mut Value) {
    if provider_id != "deepseek" || body.get("thinking").is_some() {
        return;
    }
    if let Some(body) = body.as_object_mut() {
        // This function is used only by the OpenAI Chat Completions -> Anthropic
        // conversion path. DeepSeek's Anthropic endpoint enables thinking by
        // default, but OpenAI messages cannot round-trip Anthropic thinking
        // blocks. That breaks multi-turn tool conversations and forced tool
        // choice. Disable thinking for this lossy protocol bridge; native
        // /v1/messages requests bypass this compatibility rule entirely.
        body.insert(
            "thinking".to_owned(),
            serde_json::json!({ "type": "disabled" }),
        );
    }
}

pub async fn messages(
    state: AppState,
    resolved: ResolvedProvider,
    request: AnthropicRequest,
    client_headers: &HeaderMap,
    stream_lifecycle: StreamLifecycle,
) -> Result<Response, AppError> {
    let body = anthropic_request_value(&request, &resolved.model)?;
    let headers = headers(&resolved.provider, client_headers)?;
    let url = resolved.provider.endpoint("/v1/messages");

    if request.stream.unwrap_or(false) {
        let frames = state.transport.post_json_sse(url, headers, body).await?;
        let events: Pin<Box<dyn Stream<Item = Result<Event, AppError>> + Send>> =
            Box::pin(normalize_anthropic_stream(
                frames,
                resolved.provider.deduplicate_stream_text,
                stream_lifecycle,
            ));
        Ok(Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let response = state.transport.post_json(&url, &headers, &body).await?;
        let usage = pricing::anthropic_usage_if_present(&response);
        let mut response = Json(response).into_response();
        if let Some(usage) = usage {
            response.headers_mut().insert(
                USAGE_HEADER,
                pricing::usage_header_value(&resolved.model, usage, resolved.provider.pricing)?,
            );
        }
        Ok(response)
    }
}

pub(crate) fn headers(
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
    stream_lifecycle: StreamLifecycle,
) -> impl Stream<Item = Result<Event, AppError>> + Send {
    try_stream! {
        let mut block_types = BTreeMap::<usize, String>::new();
        let mut text_seen = BTreeMap::<usize, String>::new();
        let mut saw_event = false;
        let mut saw_message_stop = false;

        while let Some(result) = frames.next().await {
            let frame = match result {
                Ok(frame) => frame,
                Err(err) => {
                    if saw_message_stop {
                        return;
                    }
                    stream_lifecycle.mark_failed(err.to_string());
                    yield anthropic_error_event(&err)?;
                    return;
                }
            };

            let lines = frame
                .data
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();

            for line in lines {
                if line == "[DONE]" {
                    yield event_with_metadata(
                        &frame,
                        frame.event.as_deref().unwrap_or("message"),
                        "[DONE]".to_owned(),
                    );
                    continue;
                }

                let Ok(mut data) = serde_json::from_str::<Value>(&line) else {
                    saw_event = true;
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
            if let Some(usage) = data
                .get("message")
                .and_then(pricing::anthropic_usage_if_present)
                .or_else(|| pricing::anthropic_usage_if_present(&data))
            {
                stream_lifecycle.merge_usage(usage);
            }
            saw_event = true;
            if event_type.as_deref() == Some("message_stop")
                || frame.event.as_deref() == Some("message_stop")
            {
                saw_message_stop = true;
                stream_lifecycle.mark_completed();
            }
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
            if saw_message_stop {
                return;
            }
            }
        }

        if !saw_event || !saw_message_stop {
            let app_error = AppError::UpstreamProtocol(
                "Anthropic stream ended without a message_stop event".to_owned(),
            );
            stream_lifecycle.mark_failed(app_error.to_string());
            yield anthropic_error_event(&app_error)?;
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

#[cfg(test)]
mod tests {
    use futures_util::{StreamExt, stream};

    use super::*;

    fn frame(event: &str, data: &str) -> SseFrame {
        SseFrame {
            event: Some(event.to_owned()),
            id: None,
            retry: None,
            comments: Vec::new(),
            data: data.to_owned(),
        }
    }

    #[test]
    fn deepseek_openai_bridge_disables_default_thinking() {
        for mut body in [
            serde_json::json!({}),
            serde_json::json!({ "tool_choice": { "type": "auto" } }),
            serde_json::json!({ "tool_choice": { "type": "any" } }),
            serde_json::json!({ "tool_choice": { "type": "tool" } }),
        ] {
            apply_provider_request_compatibility("deepseek", &mut body);
            assert_eq!(body["thinking"]["type"], "disabled");
        }

        let mut anthropic = serde_json::json!({ "tool_choice": { "type": "tool" } });
        apply_provider_request_compatibility("anthropic", &mut anthropic);
        assert!(anthropic.get("thinking").is_none());
    }

    #[tokio::test]
    async fn native_stream_stops_immediately_after_message_stop() {
        let frames: SseFrameStream = Box::pin(stream::iter(vec![
            Ok(frame("message_stop", r#"{"type":"message_stop"}"#)),
            Err(AppError::Transport(
                "must not be observed after terminal event".to_owned(),
            )),
        ]));
        let lifecycle = StreamLifecycle::new();
        let mut events = Box::pin(normalize_anthropic_stream(frames, false, lifecycle.clone()));

        assert!(events.next().await.is_some_and(|event| event.is_ok()));
        assert!(events.next().await.is_none());
        assert_eq!(
            lifecycle.state(),
            crate::stream_lifecycle::UpstreamStreamState::Completed
        );
    }
}
