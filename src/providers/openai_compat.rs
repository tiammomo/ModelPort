use std::collections::BTreeMap;

use async_stream::try_stream;
use axum::{
    Json,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    config::ResolvedProvider,
    error::AppError,
    http::{Header, HttpTransport, SseFrameStream},
    pricing::{self, USAGE_HEADER},
    routes::AppState,
    types::{
        AnthropicRequest, anthropic_error_event, anthropic_event, anthropic_to_openai_request,
        openai_response_to_anthropic,
    },
};

pub async fn messages(
    state: AppState,
    resolved: ResolvedProvider,
    request: AnthropicRequest,
) -> Result<Response, AppError> {
    let headers = headers(&resolved.provider)?;
    let url = resolved.provider.endpoint("/chat/completions");

    if request.stream.unwrap_or(false) {
        if resolved.provider.buffer_stream_text {
            let mut body = anthropic_to_openai_request(
                &request,
                &resolved.model,
                false,
                resolved.provider.max_tokens_field,
            )?;
            apply_buffered_generation_defaults(&mut body);
            let events = openai_complete_to_anthropic_stream(
                state.transport.clone(),
                url,
                headers,
                body,
                request.model.clone(),
            );
            return Ok(Sse::new(events)
                .keep_alive(KeepAlive::default())
                .into_response());
        }

        let deduplicate_stream_text = resolved.provider.deduplicate_stream_text;
        let body = anthropic_to_openai_request(
            &request,
            &resolved.model,
            true,
            resolved.provider.max_tokens_field,
        )?;
        let frames = state.transport.post_json_sse(url, headers, body);
        let events =
            openai_stream_to_anthropic(frames, request.model.clone(), deduplicate_stream_text);
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
        let usage = pricing::openai_usage(&response);
        let mut response =
            Json(openai_response_to_anthropic(&response, &request.model)?).into_response();
        response.headers_mut().insert(
            USAGE_HEADER,
            pricing::usage_header_value(&resolved.model, usage)?,
        );
        Ok(response)
    }
}

fn openai_complete_to_anthropic_stream(
    transport: HttpTransport,
    url: String,
    headers: Vec<Header>,
    body: Value,
    requested_model: String,
) -> impl Stream<Item = Result<Event, AppError>> + Send {
    try_stream! {
        let response = match transport.post_json(&url, &headers, &body).await {
            Ok(response) => response,
            Err(err) => {
                yield anthropic_error_event(&err)?;
                return;
            }
        };
        let message = match openai_response_to_anthropic(&response, &requested_model) {
            Ok(message) => message,
            Err(err) => {
                yield anthropic_error_event(&err)?;
                return;
            }
        };

        let message_id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("msg_modelport");
        yield message_start_event(message_id, &requested_model)?;

        if let Some(blocks) = message.get("content").and_then(Value::as_array) {
            for (index, block) in blocks.iter().enumerate() {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                        let text = suppress_repeated_output(text);
                        yield anthropic_event("content_block_start", json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {
                                "type": "text",
                                "text": ""
                            }
                        }))?;

                        for chunk in stable_text_chunks(&text) {
                            yield anthropic_event("content_block_delta", json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {
                                    "type": "text_delta",
                                    "text": chunk
                                }
                            }))?;
                        }

                        yield anthropic_event("content_block_stop", json!({
                            "type": "content_block_stop",
                            "index": index
                        }))?;
                    }
                    Some("tool_use") => {
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4().simple()));
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let input = block.get("input").cloned().unwrap_or_else(|| json!({}));

                        yield anthropic_event("content_block_start", json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": {}
                            }
                        }))?;

                        if input != json!({}) {
                            let partial_json = serde_json::to_string(&input)?;
                            yield anthropic_event("content_block_delta", json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {
                                    "type": "input_json_delta",
                                    "partial_json": partial_json
                                }
                            }))?;
                        }

                        yield anthropic_event("content_block_stop", json!({
                            "type": "content_block_stop",
                            "index": index
                        }))?;
                    }
                    _ => {}
                }
            }
        }

        let stop_reason = message
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn");
        let output_tokens = message
            .get("usage")
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);

        yield anthropic_event("message_delta", json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": null
            },
            "usage": {
                "output_tokens": output_tokens
            }
        }))?;

        yield anthropic_event("message_stop", json!({
            "type": "message_stop"
        }))?;
    }
}

fn apply_buffered_generation_defaults(body: &mut Value) {
    let Some(body) = body.as_object_mut() else {
        return;
    };

    body.entry("temperature".to_owned()).or_insert(json!(0.2));
}

fn headers(provider: &crate::config::ProviderConfig) -> Result<Vec<Header>, AppError> {
    let Some(api_key) = provider.api_key()? else {
        return Ok(Vec::new());
    };

    Ok(vec![(
        "Authorization".to_owned(),
        format!("Bearer {api_key}"),
    )])
}

fn openai_stream_to_anthropic(
    mut frames: SseFrameStream,
    model: String,
    deduplicate_stream_text: bool,
) -> impl Stream<Item = Result<Event, AppError>> + Send {
    try_stream! {
        let message_id = format!("msg_{}", Uuid::new_v4().simple());
        let mut message_started = false;
        let mut next_index = 0usize;
        let mut text_index: Option<usize> = None;
        let mut text_seen = String::new();
        let mut tools = BTreeMap::<usize, ToolState>::new();
        let mut finish_reason = "end_turn".to_owned();
        let mut stream_done = false;

        while let Some(frame) = frames.next().await {
            let frame = match frame {
                Ok(frame) => frame,
                Err(err) => {
                    yield anthropic_error_event(&err)?;
                    return;
                }
            };

            for data in frame.data.lines().map(str::trim).filter(|data| !data.is_empty()) {
                if data == "[DONE]" {
                    if !message_started {
                        yield message_start_event(&message_id, &model)?;
                        message_started = true;
                    }
                    stream_done = true;
                    break;
                }

                let chunk: OpenAiStreamChunk = match serde_json::from_str(data) {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        let app_error = AppError::UpstreamProtocol(format!(
                            "invalid OpenAI-compatible SSE chunk: {err}; data: {data}"
                        ));
                        yield anthropic_error_event(&app_error)?;
                        return;
                    }
                };

                if !message_started {
                    yield message_start_event(&message_id, &model)?;
                    message_started = true;
                }

                for choice in chunk.choices {
                    if let Some(reason) = choice.finish_reason {
                        finish_reason = map_finish_reason(&reason).to_owned();
                    }

                    if let Some(content) = choice.delta.content
                    {
                        let content =
                            text_delta(&mut text_seen, &content, deduplicate_stream_text);
                        if content.is_empty() {
                            continue;
                        }

                        let index = match text_index {
                            Some(index) => index,
                            None => {
                                let index = next_index;
                                next_index += 1;
                                text_index = Some(index);
                                yield anthropic_event("content_block_start", json!({
                                    "type": "content_block_start",
                                    "index": index,
                                    "content_block": {
                                        "type": "text",
                                        "text": ""
                                    }
                                }))?;
                                index
                            }
                        };

                        yield anthropic_event("content_block_delta", json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": {
                                "type": "text_delta",
                                "text": content
                            }
                        }))?;
                    }

                    if let Some(tool_calls) = choice.delta.tool_calls {
                        for tool_call in tool_calls {
                            let state = tools.entry(tool_call.index).or_insert_with(|| {
                                let index = next_index;
                                next_index += 1;
                                ToolState {
                                    index,
                                    upstream_id: None,
                                    name: None,
                                    started: false,
                                    arguments_seen: String::new(),
                                    raw_arguments: Vec::new(),
                                    pending_arguments: String::new(),
                                }
                            });

                            if let Some(id) = tool_call.id {
                                state.upstream_id = Some(id);
                            }
                            if let Some(function) = tool_call.function {
                                if let Some(name) = function.name {
                                    state.name = Some(name);
                                }
                                if let Some(arguments) = function.arguments {
                                    if deduplicate_stream_text {
                                        if !arguments.is_empty() {
                                            state.raw_arguments.push(arguments.clone());
                                            let arguments = text_delta(
                                                &mut state.arguments_seen,
                                                &arguments,
                                                true,
                                            );
                                            if !arguments.is_empty() {
                                                state.pending_arguments.push_str(&arguments);
                                            }
                                        }
                                    } else if !arguments.is_empty() {
                                        if state.started {
                                            yield anthropic_event("content_block_delta", json!({
                                                "type": "content_block_delta",
                                                "index": state.index,
                                                "delta": {
                                                    "type": "input_json_delta",
                                                    "partial_json": arguments
                                                }
                                            }))?;
                                        } else {
                                            state.pending_arguments.push_str(&arguments);
                                        }
                                    }
                                }
                            }

                            if !state.started && state.name.is_some() {
                                state.started = true;
                                let id = state
                                    .upstream_id
                                    .clone()
                                    .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4().simple()));
                                let name = state.name.clone().unwrap_or_else(|| "tool".to_owned());
                                yield anthropic_event("content_block_start", json!({
                                    "type": "content_block_start",
                                    "index": state.index,
                                    "content_block": {
                                        "type": "tool_use",
                                        "id": id,
                                        "name": name,
                                        "input": {}
                                    }
                                }))?;

                                if !state.pending_arguments.is_empty() {
                                    let pending = std::mem::take(&mut state.pending_arguments);
                                    yield anthropic_event("content_block_delta", json!({
                                        "type": "content_block_delta",
                                        "index": state.index,
                                        "delta": {
                                            "type": "input_json_delta",
                                            "partial_json": pending
                                        }
                                    }))?;
                                }
                            }
                        }
                    }
                }

                if stream_done {
                    break;
                }
            }

            if stream_done {
                break;
            }
        }

        if !message_started {
            let app_error = AppError::UpstreamProtocol(
                "upstream stream ended before sending any SSE chunks".to_owned(),
            );
            yield anthropic_error_event(&app_error)?;
            return;
        }

        if let Some(index) = text_index {
            yield anthropic_event("content_block_stop", json!({
                "type": "content_block_stop",
                "index": index
            }))?;
        }

        for state in tools.values() {
            if state.started {
                if deduplicate_stream_text
                    && let Some(arguments) = state.complete_arguments()
                {
                    yield anthropic_event("content_block_delta", json!({
                        "type": "content_block_delta",
                        "index": state.index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": arguments
                        }
                    }))?;
                }

                yield anthropic_event("content_block_stop", json!({
                    "type": "content_block_stop",
                    "index": state.index
                }))?;
            }
        }

        yield anthropic_event("message_delta", json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": finish_reason,
                "stop_sequence": null
            },
            "usage": {
                "output_tokens": 0
            }
        }))?;

        yield anthropic_event("message_stop", json!({
            "type": "message_stop"
        }))?;
    }
}

fn message_start_event(message_id: &str, model: &str) -> Result<Event, AppError> {
    anthropic_event(
        "message_start",
        json!({
            "type": "message_start",
            "message": {
                "id": message_id,
                "type": "message",
                "role": "assistant",
                "model": model,
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0
                }
            }
        }),
    )
}

fn suppress_repeated_output(text: &str) -> String {
    let mut current = collapse_repeated_lines(text);

    for _ in 0..4 {
        let next = collapse_repeated_char_sequences(&current);
        if next == current {
            break;
        }
        current = next;
    }

    current
}

fn collapse_repeated_lines(text: &str) -> String {
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    if lines.is_empty() {
        return text.to_owned();
    }

    let mut output = String::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let mut count = 1;
        while index + count < lines.len() && lines[index + count] == line {
            count += 1;
        }

        let kept = if count >= 3 { 1 } else { count };
        for _ in 0..kept {
            output.push_str(line);
        }
        index += count;
    }

    output
}

fn collapse_repeated_char_sequences(text: &str) -> String {
    const MAX_UNIT_CHARS: usize = 48;

    let chars = text.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        let max_width = MAX_UNIT_CHARS.min((chars.len() - index) / 3);
        let mut repeated = None;

        for width in 2..=max_width {
            let unit = &chars[index..index + width];
            if !is_repetition_unit(unit) {
                continue;
            }

            let mut count = 1usize;
            while index + (count + 1) * width <= chars.len()
                && unit == &chars[index + count * width..index + (count + 1) * width]
            {
                count += 1;
            }

            let min_count = if width <= 2 { 4 } else { 3 };
            if count >= min_count {
                repeated = Some((width, count));
                break;
            }
        }

        if let Some((width, count)) = repeated {
            for ch in &chars[index..index + width] {
                output.push(*ch);
            }
            index += width * count;
        } else {
            output.push(chars[index]);
            index += 1;
        }
    }

    output
}

fn is_repetition_unit(chars: &[char]) -> bool {
    chars
        .iter()
        .filter(|ch| ch.is_alphanumeric())
        .take(2)
        .count()
        >= 2
}

fn stable_text_chunks(text: &str) -> Vec<String> {
    const MAX_CHARS: usize = 240;

    let mut chunks = Vec::new();
    let mut line_start = 0;
    for (index, ch) in text.char_indices() {
        if ch == '\n' {
            push_wrapped_text_chunk(&text[line_start..index + ch.len_utf8()], &mut chunks);
            line_start = index + ch.len_utf8();
        }
    }

    if line_start < text.len() {
        push_wrapped_text_chunk(&text[line_start..], &mut chunks);
    }

    if chunks.is_empty() && !text.is_empty() {
        chunks.push(text.to_owned());
    }

    chunks
        .into_iter()
        .flat_map(|chunk| wrap_long_chunk(&chunk, MAX_CHARS))
        .collect()
}

fn push_wrapped_text_chunk(chunk: &str, chunks: &mut Vec<String>) {
    if !chunk.is_empty() {
        chunks.push(chunk.to_owned());
    }
}

fn wrap_long_chunk(chunk: &str, max_chars: usize) -> Vec<String> {
    if chunk.chars().count() <= max_chars {
        return vec![chunk.to_owned()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    let mut count = 0usize;
    let mut last_boundary = None;

    for (index, ch) in chunk.char_indices() {
        count += 1;
        if ch.is_whitespace() || matches!(ch, '。' | '，' | ',' | '.' | ';' | '；' | '、') {
            last_boundary = Some(index + ch.len_utf8());
        }

        if count >= max_chars {
            let end = last_boundary
                .filter(|boundary| *boundary > start)
                .unwrap_or(index + ch.len_utf8());
            chunks.push(chunk[start..end].to_owned());
            start = end;
            count = 0;
            last_boundary = None;
        }
    }

    if start < chunk.len() {
        chunks.push(chunk[start..].to_owned());
    }

    chunks
}

fn text_delta(seen: &mut String, content: &str, deduplicate: bool) -> String {
    if content.is_empty() {
        return String::new();
    }

    if !deduplicate {
        seen.push_str(content);
        return content.to_owned();
    }

    if content.starts_with(seen.as_str()) {
        let delta = content[seen.len()..].to_owned();
        seen.clear();
        seen.push_str(content);
        return delta;
    }

    if seen.starts_with(content) {
        return String::new();
    }

    if seen.ends_with(content) {
        return String::new();
    }

    if content.chars().count() >= 3 && seen.contains(content) {
        return String::new();
    }

    let overlap = suffix_prefix_overlap(seen, content);
    let delta = content[overlap..].to_owned();
    seen.push_str(&delta);
    delta
}

fn suffix_prefix_overlap(seen: &str, content: &str) -> usize {
    let mut best = 0;
    let mut boundaries = content
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    boundaries.push(content.len());

    for end in boundaries.into_iter().skip(1) {
        let prefix = &content[..end];
        if prefix.chars().count() >= 2 && seen.ends_with(prefix) {
            best = end;
        }
    }

    best
}

#[derive(Debug)]
struct ToolState {
    index: usize,
    upstream_id: Option<String>,
    name: Option<String>,
    started: bool,
    arguments_seen: String,
    raw_arguments: Vec<String>,
    pending_arguments: String,
}

impl ToolState {
    fn complete_arguments(&self) -> Option<String> {
        let joined_raw_arguments = self.raw_arguments.concat();
        best_complete_json_object(
            std::iter::once(self.arguments_seen.as_str())
                .chain(std::iter::once(self.pending_arguments.as_str()))
                .chain(std::iter::once(joined_raw_arguments.as_str()))
                .chain(self.raw_arguments.iter().map(String::as_str)),
        )
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    #[serde(default)]
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    #[serde(default)]
    delta: OpenAiStreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenAiStreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OpenAiFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct OpenAiFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

fn map_finish_reason(reason: &str) -> &'static str {
    match reason {
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        "stop" => "end_turn",
        _ => "end_turn",
    }
}

fn best_complete_json_object<'a>(sources: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut best = None::<String>;

    for source in sources {
        collect_complete_json_objects(source, &mut best);
    }

    best
}

fn collect_complete_json_objects(source: &str, best: &mut Option<String>) {
    for (start, ch) in source.char_indices() {
        if ch != '{' {
            continue;
        }

        let slice = &source[start..];
        let mut values = serde_json::Deserializer::from_str(slice).into_iter::<Value>();
        let Some(Ok(value)) = values.next() else {
            continue;
        };
        if !value.is_object() {
            continue;
        }

        let end = values.byte_offset();
        if end == 0 {
            continue;
        }

        let candidate = &slice[..end];
        if best
            .as_ref()
            .is_none_or(|current| candidate.len() > current.len())
        {
            *best = Some(candidate.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        best_complete_json_object, stable_text_chunks, suppress_repeated_output, text_delta,
    };

    #[test]
    fn cumulative_stream_text_is_reduced_to_suffix() {
        let mut seen = String::new();

        assert_eq!(text_delta(&mut seen, "hel", true), "hel");
        assert_eq!(text_delta(&mut seen, "hello", true), "lo");
        assert_eq!(text_delta(&mut seen, "hel", true), "");
        assert_eq!(text_delta(&mut seen, "hello", true), "");
    }

    #[test]
    fn standard_delta_stream_text_is_preserved() {
        let mut seen = String::new();

        assert_eq!(text_delta(&mut seen, "hel", false), "hel");
        assert_eq!(text_delta(&mut seen, "lo", false), "lo");
        assert_eq!(seen, "hello");
    }

    #[test]
    fn overlapping_stream_text_is_reduced_to_unseen_suffix() {
        let mut seen = String::new();

        assert_eq!(text_delta(&mut seen, "我是 Mi", true), "我是 Mi");
        assert_eq!(text_delta(&mut seen, "Mo-v2.", true), "Mo-v2.");
        assert_eq!(text_delta(&mut seen, "Mo-v2.", true), "");
        assert_eq!(text_delta(&mut seen, "Mo-v2.5-pro", true), "5-pro");
        assert_eq!(seen, "我是 MiMo-v2.5-pro");
    }

    #[test]
    fn replayed_prior_stream_text_is_ignored_in_deduplicate_mode() {
        let mut seen = String::new();

        assert_eq!(text_delta(&mut seen, "我是由", true), "我是由");
        assert_eq!(text_delta(&mut seen, "小米Mi", true), "小米Mi");
        assert_eq!(text_delta(&mut seen, "Mo团队开发的", true), "Mo团队开发的");
        assert_eq!(text_delta(&mut seen, "小米Mi", true), "");
        assert_eq!(text_delta(&mut seen, "Mo团队开发的", true), "");
        assert_eq!(text_delta(&mut seen, "MiMo-v2", true), "MiMo-v2");
    }

    #[test]
    fn cumulative_tool_arguments_are_reduced_to_suffixes() {
        let mut seen = String::new();
        let chunks = [
            "",
            "{\"description\": ",
            "",
            "{\"description\": ",
            "\"",
            "",
            "{\"description\": ",
            "\"",
            "scan",
            "",
            "{\"description\": ",
            "\"",
            "scan",
            "\"",
            "{\"description\": \"scan\", \"prompt\": ",
            "\"",
            "{\"description\": \"scan\", \"prompt\": \"list project files",
            "\"",
            "{\"description\": \"scan\", \"prompt\": \"list project files\"}",
            "",
            "{\"description\": \"scan\", \"prompt\": \"list project files\"}",
        ];

        let reduced = chunks
            .into_iter()
            .map(|chunk| text_delta(&mut seen, chunk, true))
            .collect::<String>();

        assert_eq!(
            reduced,
            "{\"description\": \"scan\", \"prompt\": \"list project files\"}"
        );
    }

    #[test]
    fn best_complete_json_object_ignores_trailing_replayed_tool_fragments() {
        let sources = [
            "{\"description\": \"scan\", \"prompt\": \"list project files\"}\"}\"}",
            "{\"description\": \"scan\"}",
            "scan",
        ];

        assert_eq!(
            best_complete_json_object(sources),
            Some("{\"description\": \"scan\", \"prompt\": \"list project files\"}".to_owned())
        );
    }

    #[test]
    fn stable_text_chunks_preserve_markdown_lines() {
        assert_eq!(
            stable_text_chunks("| A | B |\n| --- | --- |\n| x | y |"),
            vec![
                "| A | B |\n".to_owned(),
                "| --- | --- |\n".to_owned(),
                "| x | y |".to_owned()
            ]
        );
    }

    #[test]
    fn suppresses_short_phrase_stutter() {
        assert_eq!(
            suppress_repeated_output(
                "上次构建被中断了，让我重新验证一下：一下：一下：一下：一下：一下："
            ),
            "上次构建被中断了，让我重新验证一下："
        );
    }

    #[test]
    fn suppresses_replayed_status_phrases() {
        assert_eq!(
            suppress_repeated_output(
                "构建通过，生成之前删除，生成之前删除，生成之前删除，所有页面正常"
            ),
            "构建通过，生成之前删除，所有页面正常"
        );
    }

    #[test]
    fn preserves_two_repeated_lines() {
        assert_eq!(
            suppress_repeated_output("same\nsame\nnext\n"),
            "same\nsame\nnext\n"
        );
    }
}
