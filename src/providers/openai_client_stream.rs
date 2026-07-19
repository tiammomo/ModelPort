use std::collections::BTreeMap;

use async_stream::try_stream;
use axum::response::sse::Event;
use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};

use crate::{
    error::AppError,
    exchange::{openai_completion_id, openai_usage_value, unix_timestamp_seconds},
    http::{SseFrame, SseFrameStream},
    pricing,
    stream_lifecycle::StreamLifecycle,
};

pub(crate) fn openai_stream_passthrough(
    mut frames: SseFrameStream,
    requested_model: String,
    stream_lifecycle: StreamLifecycle,
) -> impl Stream<Item = Result<Event, AppError>> + Send {
    try_stream! {
        let mut saw_chunk = false;
        let mut saw_finish_reason = false;

        while let Some(frame) = frames.next().await {
            let frame = match frame {
                Ok(frame) => frame,
                Err(_) if saw_finish_reason => {
                    stream_lifecycle.mark_completed();
                    yield openai_event("[DONE]");
                    return;
                }
                Err(error) => {
                    stream_lifecycle.mark_failed(error.to_string());
                    yield openai_error_event(&error);
                    return;
                }
            };

            for data in frame.data.lines().map(str::trim).filter(|data| !data.is_empty()) {
                if data == "[DONE]" {
                    if !saw_chunk {
                        let error = AppError::UpstreamProtocol(
                            "OpenAI-compatible stream ended before sending any chunks".to_owned(),
                        );
                        stream_lifecycle.mark_failed(error.to_string());
                        yield openai_error_event(&error);
                        return;
                    }
                    stream_lifecycle.mark_completed();
                    yield event_with_metadata(&frame, data.to_owned());
                    return;
                }

                let mut chunk: Value = match serde_json::from_str(data) {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        let error = AppError::UpstreamProtocol(format!(
                            "invalid OpenAI-compatible SSE chunk: {error}; data: {data}"
                        ));
                        stream_lifecycle.mark_failed(error.to_string());
                        yield openai_error_event(&error);
                        return;
                    }
                };
                if let Some(object) = chunk.as_object_mut()
                    && object.contains_key("model")
                {
                    object.insert("model".to_owned(), Value::String(requested_model.clone()));
                }
                saw_chunk = true;
                if chunk.get("error").is_some() {
                    let error = AppError::UpstreamProtocol(format!(
                        "OpenAI-compatible stream returned an error event: {}",
                        compact_json(&chunk)
                    ));
                    stream_lifecycle.mark_failed(error.to_string());
                    yield event_with_metadata(&frame, serde_json::to_string(&chunk)?);
                    return;
                }
                if let Some(usage) = pricing::openai_usage_if_present(&chunk) {
                    stream_lifecycle.merge_usage(usage);
                }
                observe_openai_stream_chunk(&stream_lifecycle, &chunk);
                if chunk
                    .get("choices")
                    .and_then(Value::as_array)
                    .is_some_and(|choices| {
                        choices.iter().any(|choice| {
                            choice
                                .get("finish_reason")
                                .is_some_and(|reason| !reason.is_null())
                        })
                    })
                {
                    saw_finish_reason = true;
                }
                yield event_with_metadata(&frame, serde_json::to_string(&chunk)?);
            }
        }

        if saw_chunk && saw_finish_reason {
            stream_lifecycle.mark_completed();
            yield openai_event("[DONE]");
            return;
        }
        let error = AppError::UpstreamProtocol(
            "OpenAI-compatible stream ended without [DONE] or finish_reason".to_owned(),
        );
        stream_lifecycle.mark_failed(error.to_string());
        yield openai_error_event(&error);
    }
}

pub(crate) fn anthropic_stream_to_openai(
    mut frames: SseFrameStream,
    requested_model: String,
    include_usage: bool,
    stream_lifecycle: StreamLifecycle,
) -> impl Stream<Item = Result<Event, AppError>> + Send {
    try_stream! {
        let mut completion_id = openai_completion_id(None);
        let created = unix_timestamp_seconds();
        let mut message_started = false;
        let mut finish_emitted = false;
        let mut tool_indexes = BTreeMap::<usize, usize>::new();
        let mut next_tool_index = 0usize;
        let mut text_present = false;

        while let Some(frame) = frames.next().await {
            let frame = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    stream_lifecycle.mark_failed(error.to_string());
                    yield openai_error_event(&error);
                    return;
                }
            };
            for data in frame.data.lines().map(str::trim).filter(|data| !data.is_empty()) {
                if data == "[DONE]" {
                    continue;
                }
                let event: Value = match serde_json::from_str(data) {
                    Ok(event) => event,
                    Err(error) => {
                        let error = AppError::UpstreamProtocol(format!(
                            "invalid Anthropic SSE event: {error}; data: {data}"
                        ));
                        stream_lifecycle.mark_failed(error.to_string());
                        yield openai_error_event(&error);
                        return;
                    }
                };
                let event_type = event
                    .get("type")
                    .and_then(Value::as_str)
                    .or(frame.event.as_deref())
                    .unwrap_or("message");

                if let Some(usage) = event
                    .get("message")
                    .and_then(pricing::anthropic_usage_if_present)
                    .or_else(|| pricing::anthropic_usage_if_present(&event))
                {
                    stream_lifecycle.merge_usage(usage);
                }

                match event_type {
                    "message_start" => {
                        completion_id = openai_completion_id(
                            event
                                .get("message")
                                .and_then(|message| message.get("id"))
                                .and_then(Value::as_str),
                        );
                        message_started = true;
                        yield openai_chunk_event(
                            &completion_id,
                            created,
                            &requested_model,
                            json!({ "role": "assistant", "content": "" }),
                            Value::Null,
                        )?;
                    }
                    "content_block_start" => {
                        if !message_started {
                            message_started = true;
                            yield openai_chunk_event(
                                &completion_id,
                                created,
                                &requested_model,
                                json!({ "role": "assistant", "content": "" }),
                                Value::Null,
                            )?;
                        }
                        let block = event.get("content_block").unwrap_or(&Value::Null);
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            let block_index = event
                                .get("index")
                                .and_then(Value::as_u64)
                                .and_then(|value| usize::try_from(value).ok())
                                .unwrap_or(next_tool_index);
                            let tool_index = next_tool_index;
                            next_tool_index = next_tool_index.saturating_add(1);
                            tool_indexes.insert(block_index, tool_index);
                            stream_lifecycle.observe_response_fragment(
                                next_tool_index,
                                text_present,
                                None,
                            );
                            yield openai_chunk_event(
                                &completion_id,
                                created,
                                &requested_model,
                                json!({
                                    "tool_calls": [{
                                        "index": tool_index,
                                        "id": block.get("id").and_then(Value::as_str).unwrap_or("call_modelport"),
                                        "type": "function",
                                        "function": {
                                            "name": block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                                            "arguments": ""
                                        }
                                    }]
                                }),
                                Value::Null,
                            )?;
                        }
                    }
                    "content_block_delta" => {
                        let delta = event.get("delta").unwrap_or(&Value::Null);
                        match delta.get("type").and_then(Value::as_str) {
                            Some("text_delta") => {
                                if delta
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .is_some_and(|text| !text.is_empty())
                                {
                                    text_present = true;
                                    stream_lifecycle.observe_response_fragment(
                                        next_tool_index,
                                        true,
                                        None,
                                    );
                                }
                                yield openai_chunk_event(
                                    &completion_id,
                                    created,
                                    &requested_model,
                                    json!({
                                        "content": delta.get("text").and_then(Value::as_str).unwrap_or("")
                                    }),
                                    Value::Null,
                                )?;
                            }
                            Some("input_json_delta") => {
                                let block_index = event
                                    .get("index")
                                    .and_then(Value::as_u64)
                                    .and_then(|value| usize::try_from(value).ok())
                                    .unwrap_or(0);
                                let tool_index = tool_indexes.get(&block_index).copied().unwrap_or(0);
                                yield openai_chunk_event(
                                    &completion_id,
                                    created,
                                    &requested_model,
                                    json!({
                                        "tool_calls": [{
                                            "index": tool_index,
                                            "function": {
                                                "arguments": delta
                                                    .get("partial_json")
                                                    .and_then(Value::as_str)
                                                    .unwrap_or("")
                                            }
                                        }]
                                    }),
                                    Value::Null,
                                )?;
                            }
                            _ => {}
                        }
                    }
                    "message_delta" => {
                        let stop_reason = event
                            .get("delta")
                            .and_then(|delta| delta.get("stop_reason"))
                            .and_then(Value::as_str)
                            .unwrap_or("end_turn");
                        finish_emitted = true;
                        stream_lifecycle.observe_response_fragment(
                            next_tool_index,
                            text_present,
                            Some(stop_reason),
                        );
                        yield openai_chunk_event(
                            &completion_id,
                            created,
                            &requested_model,
                            json!({}),
                            Value::String(map_anthropic_stop_reason(stop_reason).to_owned()),
                        )?;
                    }
                    "message_stop" => {
                        if !finish_emitted {
                            yield openai_chunk_event(
                                &completion_id,
                                created,
                                &requested_model,
                                json!({}),
                                Value::String("stop".to_owned()),
                            )?;
                        }
                        stream_lifecycle.mark_completed();
                        if include_usage
                            && let Some(usage) = stream_lifecycle.usage()
                        {
                            yield openai_usage_chunk_event(
                                &completion_id,
                                created,
                                &requested_model,
                                usage,
                            )?;
                        }
                        yield openai_event("[DONE]");
                        return;
                    }
                    "error" => {
                        let message = event
                            .get("error")
                            .and_then(|error| error.get("message"))
                            .and_then(Value::as_str)
                            .unwrap_or("Anthropic stream returned an error event");
                        stream_lifecycle.mark_failed(message.to_owned());
                        yield openai_event(&serde_json::to_string(&json!({
                            "error": {
                                "message": message,
                                "type": "api_error",
                                "code": "upstream_stream_error"
                            }
                        }))?);
                        return;
                    }
                    _ => {}
                }
            }
        }

        let error = AppError::UpstreamProtocol(
            "Anthropic stream ended without a message_stop event".to_owned(),
        );
        stream_lifecycle.mark_failed(error.to_string());
        yield openai_error_event(&error);
    }
}

fn observe_openai_stream_chunk(stream_lifecycle: &StreamLifecycle, chunk: &Value) {
    let mut tool_call_count = 0usize;
    let mut text_present = false;
    let mut stop_reason = None;
    for choice in chunk
        .get("choices")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        if let Some(delta) = choice.get("delta") {
            text_present |= delta
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty());
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                tool_call_count = tool_call_count.max(
                    tool_calls
                        .iter()
                        .filter_map(|call| call.get("index").and_then(Value::as_u64))
                        .max()
                        .map_or(tool_calls.len(), |index| index as usize + 1),
                );
            }
            if delta.get("function_call").is_some_and(Value::is_object) {
                tool_call_count = tool_call_count.max(1);
            }
        }
        stop_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .or(stop_reason);
    }
    stream_lifecycle.observe_response_fragment(tool_call_count, text_present, stop_reason);
}

pub(crate) fn openai_complete_to_stream(
    response: Value,
    requested_model: String,
    include_usage: bool,
) -> impl Stream<Item = Result<Event, AppError>> + Send {
    try_stream! {
        let completion_id = openai_completion_id(response.get("id").and_then(Value::as_str));
        let created = response
            .get("created")
            .and_then(Value::as_u64)
            .unwrap_or_else(unix_timestamp_seconds);
        let choice = response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .ok_or_else(|| AppError::UpstreamProtocol(
                "OpenAI-compatible response has no choices".to_owned()
            ))?;
        let message = choice.get("message").ok_or_else(|| AppError::UpstreamProtocol(
            "OpenAI-compatible response has no message".to_owned()
        ))?;
        yield openai_chunk_event(
            &completion_id,
            created,
            &requested_model,
            json!({ "role": "assistant", "content": "" }),
            Value::Null,
        )?;
        if let Some(content) = message.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            yield openai_chunk_event(
                &completion_id,
                created,
                &requested_model,
                json!({ "content": content }),
                Value::Null,
            )?;
        }
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            let calls = tool_calls
                .iter()
                .enumerate()
                .map(|(index, call)| json!({
                    "index": index,
                    "id": call.get("id").cloned().unwrap_or_else(|| Value::String("call_modelport".to_owned())),
                    "type": "function",
                    "function": call.get("function").cloned().unwrap_or_else(|| json!({
                        "name": "tool",
                        "arguments": "{}"
                    }))
                }))
                .collect::<Vec<_>>();
            yield openai_chunk_event(
                &completion_id,
                created,
                &requested_model,
                json!({ "tool_calls": calls }),
                Value::Null,
            )?;
        }
        let finish_reason = choice
            .get("finish_reason")
            .cloned()
            .unwrap_or_else(|| Value::String("stop".to_owned()));
        yield openai_chunk_event(
            &completion_id,
            created,
            &requested_model,
            json!({}),
            finish_reason,
        )?;
        if include_usage
            && let Some(usage) = pricing::openai_usage_if_present(&response)
        {
            yield openai_usage_chunk_event(
                &completion_id,
                created,
                &requested_model,
                usage,
            )?;
        }
        yield openai_event("[DONE]");
    }
}

fn openai_chunk_event(
    id: &str,
    created: u64,
    model: &str,
    delta: Value,
    finish_reason: Value,
) -> Result<Event, AppError> {
    Ok(openai_event(&serde_json::to_string(&json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "logprobs": null,
            "finish_reason": finish_reason
        }]
    }))?))
}

fn openai_usage_chunk_event(
    id: &str,
    created: u64,
    model: &str,
    usage: pricing::TokenUsageBreakdown,
) -> Result<Event, AppError> {
    Ok(openai_event(&serde_json::to_string(&json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [],
        "usage": openai_usage_value(usage)
    }))?))
}

fn openai_error_event(error: &AppError) -> Event {
    openai_event(
        &serde_json::to_string(&json!({
            "error": {
                "message": error.to_string(),
                "type": "api_error",
                "code": "upstream_stream_error"
            }
        }))
        .unwrap_or_else(|_| {
            "{\"error\":{\"message\":\"stream error\",\"type\":\"api_error\"}}".to_owned()
        }),
    )
}

fn openai_event(data: &str) -> Event {
    Event::default().data(data)
}

fn event_with_metadata(frame: &SseFrame, data: String) -> Event {
    let mut event = openai_event(&data);
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

fn map_anthropic_stop_reason(reason: &str) -> &'static str {
    match reason {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "invalid error event".to_owned())
}

#[cfg(test)]
mod tests {
    use futures_util::{StreamExt, stream};

    use crate::http::SseFrame;

    use super::*;

    fn frame(data: &str) -> SseFrame {
        SseFrame {
            event: None,
            id: None,
            retry: None,
            comments: Vec::new(),
            data: data.to_owned(),
        }
    }

    #[tokio::test]
    async fn passthrough_waits_for_usage_chunk_before_done() {
        let frames: SseFrameStream = Box::pin(stream::iter(vec![
            Ok(frame(
                r#"{"id":"chatcmpl-1","choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            )),
            Ok(frame(
                r#"{"id":"chatcmpl-1","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
            )),
            Ok(frame("[DONE]")),
        ]));
        let lifecycle = StreamLifecycle::new();
        let mut events = Box::pin(openai_stream_passthrough(
            frames,
            "virtual-model".to_owned(),
            lifecycle.clone(),
        ));
        let mut count = 0;
        while let Some(event) = events.next().await {
            assert!(event.is_ok());
            count += 1;
        }

        assert_eq!(count, 3);
        assert_eq!(
            lifecycle.state(),
            crate::stream_lifecycle::UpstreamStreamState::Completed
        );
        assert_eq!(lifecycle.usage().map(|usage| usage.output_tokens), Some(2));
    }
}
