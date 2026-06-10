use axum::response::sse::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{config::MaxTokensField, error::AppError};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(default)]
    pub system: Option<Value>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub fn anthropic_request_value(request: &AnthropicRequest, model: &str) -> Result<Value, AppError> {
    let mut value = serde_json::to_value(request)?;
    value["model"] = Value::String(model.to_owned());
    Ok(value)
}

pub fn anthropic_to_openai_request(
    request: &AnthropicRequest,
    model: &str,
    stream: bool,
    max_tokens_field: MaxTokensField,
) -> Result<Value, AppError> {
    let mut body = Map::new();
    body.insert("model".to_owned(), Value::String(model.to_owned()));
    body.insert("stream".to_owned(), Value::Bool(stream));

    if let Some(max_tokens) = request.max_tokens {
        match max_tokens_field {
            MaxTokensField::MaxCompletionTokens => {
                body.insert("max_completion_tokens".to_owned(), json!(max_tokens));
            }
            MaxTokensField::MaxTokens => {
                body.insert("max_tokens".to_owned(), json!(max_tokens));
            }
            MaxTokensField::Both => {
                body.insert("max_completion_tokens".to_owned(), json!(max_tokens));
                body.insert("max_tokens".to_owned(), json!(max_tokens));
            }
        }
    }

    let mut messages = Vec::new();
    if let Some(system) = &request.system {
        let content = content_to_text(system);
        if !content.is_empty() {
            messages.push(json!({
                "role": "system",
                "content": content
            }));
        }
    }

    for message in &request.messages {
        messages.extend(convert_message(message)?);
    }
    body.insert("messages".to_owned(), Value::Array(messages));

    copy_optional(&request.extra, &mut body, "temperature");
    copy_optional(&request.extra, &mut body, "top_p");
    copy_optional(&request.extra, &mut body, "presence_penalty");
    copy_optional(&request.extra, &mut body, "frequency_penalty");
    copy_optional(&request.extra, &mut body, "seed");

    if let Some(stop_sequences) = request.extra.get("stop_sequences") {
        body.insert("stop".to_owned(), stop_sequences.clone());
    }

    if let Some(tools) = request.extra.get("tools") {
        body.insert("tools".to_owned(), convert_tools(tools)?);
    }

    if let Some(tool_choice) = request.extra.get("tool_choice") {
        body.insert("tool_choice".to_owned(), convert_tool_choice(tool_choice));
    }

    Ok(Value::Object(body))
}

pub fn openai_response_to_anthropic(
    response: &Value,
    requested_model: &str,
) -> Result<Value, AppError> {
    let choice = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| {
            AppError::UpstreamProtocol("OpenAI-compatible response has no choices".to_owned())
        })?;
    let message = choice.get("message").ok_or_else(|| {
        AppError::UpstreamProtocol("OpenAI-compatible response has no message".to_owned())
    })?;

    let mut content = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str)
        && !text.is_empty()
    {
        content.push(json!({
            "type": "text",
            "text": text
        }));
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4().simple()));
            let function = call.get("function").unwrap_or(&Value::Null);
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let arguments = function
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input = serde_json::from_str::<Value>(arguments)
                .unwrap_or_else(|_| json!({ "_raw_arguments": arguments }));

            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }

    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(map_finish_reason)
        .unwrap_or("end_turn");

    let usage = response.get("usage").unwrap_or(&Value::Null);
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    Ok(json!({
        "id": format!("msg_{}", response.get("id").and_then(Value::as_str).unwrap_or("modelport")),
        "type": "message",
        "role": "assistant",
        "model": requested_model,
        "content": content,
        "stop_reason": finish_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    }))
}

pub fn anthropic_event(event: &str, data: Value) -> Result<Event, AppError> {
    Ok(Event::default()
        .event(event)
        .data(serde_json::to_string(&data)?))
}

pub fn anthropic_error_event(error: &AppError) -> Result<Event, AppError> {
    let kind = match error {
        AppError::InvalidRequest(_) | AppError::ProviderNotFound(_) => "invalid_request_error",
        AppError::Auth => "authentication_error",
        AppError::Forbidden(_) => "permission_error",
        AppError::QuotaExceeded(_) => "rate_limit_error",
        AppError::MissingSecret(_) | AppError::Config(_) => "server_error",
        AppError::Transport(_) | AppError::Upstream { .. } | AppError::UpstreamProtocol(_) => {
            "api_error"
        }
        AppError::Io(_) | AppError::Json(_) => "server_error",
    };

    anthropic_event(
        "error",
        json!({
            "type": "error",
            "error": {
                "type": kind,
                "message": error.to_string()
            }
        }),
    )
}

fn convert_message(message: &Value) -> Result<Vec<Value>, AppError> {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::InvalidRequest("message.role is required".to_owned()))?;
    let content = message.get("content").unwrap_or(&Value::Null);

    if let Some(text) = content.as_str() {
        return Ok(vec![json!({
            "role": role,
            "content": text
        })]);
    }

    let Some(blocks) = content.as_array() else {
        return Ok(vec![json!({
            "role": role,
            "content": content_to_text(content)
        })]);
    };

    match role {
        "assistant" => convert_assistant_message(blocks),
        "user" => Ok(convert_user_message(blocks)),
        _ => Ok(vec![json!({
            "role": role,
            "content": content_to_text(content)
        })]),
    }
}

fn convert_assistant_message(blocks: &[Value]) -> Result<Vec<Value>, AppError> {
    let mut text = String::new();
    let mut tool_calls = Vec::new();

    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(value) = block.get("text").and_then(Value::as_str) {
                    text.push_str(value);
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4().simple()));
                let name = block.get("name").and_then(Value::as_str).ok_or_else(|| {
                    AppError::InvalidRequest("tool_use.name is required".to_owned())
                })?;
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&input)?
                    }
                }));
            }
            _ => {}
        }
    }

    let mut message = Map::new();
    message.insert("role".to_owned(), Value::String("assistant".to_owned()));
    message.insert("content".to_owned(), Value::String(text));
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_owned(), Value::Array(tool_calls));
    }
    Ok(vec![Value::Object(message)])
}

fn convert_user_message(blocks: &[Value]) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut text_parts = Vec::new();

    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    text_parts.push(text.to_owned());
                }
            }
            Some("tool_result") => {
                if !text_parts.is_empty() {
                    messages.push(json!({
                        "role": "user",
                        "content": text_parts.join("\n")
                    }));
                    text_parts.clear();
                }

                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("toolu_missing"),
                    "content": content_to_text(block.get("content").unwrap_or(&Value::Null))
                }));
            }
            _ => {}
        }
    }

    if !text_parts.is_empty() || messages.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": text_parts.join("\n")
        }));
    }

    messages
}

fn content_to_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }

    if let Some(blocks) = content.as_array() {
        return blocks
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                Some("tool_result") => Some(content_to_text(
                    block.get("content").unwrap_or(&Value::Null),
                )),
                Some("thinking") => block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
    }

    String::new()
}

fn convert_tools(tools: &Value) -> Result<Value, AppError> {
    let tools = tools
        .as_array()
        .ok_or_else(|| AppError::InvalidRequest("tools must be an array".to_owned()))?;

    Ok(Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.get("name").cloned().unwrap_or(Value::String("tool".to_owned())),
                        "description": tool.get("description").cloned().unwrap_or(Value::String(String::new())),
                        "parameters": tool
                            .get("input_schema")
                            .cloned()
                            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }))
                    }
                })
            })
            .collect(),
    ))
}

fn convert_tool_choice(tool_choice: &Value) -> Value {
    if let Some(kind) = tool_choice.get("type").and_then(Value::as_str) {
        match kind {
            "none" => return Value::String("none".to_owned()),
            "auto" => return Value::String("auto".to_owned()),
            "any" => return Value::String("required".to_owned()),
            "tool" => {
                if let Some(name) = tool_choice.get("name").and_then(Value::as_str) {
                    return json!({
                        "type": "function",
                        "function": {
                            "name": name
                        }
                    });
                }
            }
            _ => {}
        }
    }

    tool_choice.clone()
}

fn copy_optional(source: &Map<String, Value>, target: &mut Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_owned(), value.clone());
    }
}

fn map_finish_reason(reason: &str) -> &'static str {
    match reason {
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        "stop" => "end_turn",
        _ => "end_turn",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_anthropic_tools_to_openai_tools() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "mimo-v2.5-pro",
            "max_tokens": 128,
            "tools": [{
                "name": "read_file",
                "description": "Read a file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }],
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();

        let body = anthropic_to_openai_request(
            &request,
            "mimo-v2.5-pro",
            false,
            MaxTokensField::MaxCompletionTokens,
        )
        .unwrap();
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["max_completion_tokens"], 128);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn can_use_legacy_openai_max_tokens_field() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "qwen-plus",
            "max_tokens": 128,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();

        let body =
            anthropic_to_openai_request(&request, "qwen-plus", false, MaxTokensField::MaxTokens)
                .unwrap();

        assert_eq!(body["max_tokens"], 128);
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn converts_openai_tool_response_to_anthropic_blocks() {
        let response = json!({
            "id": "chatcmpl-1",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"Cargo.toml\"}"
                        }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 3
            }
        });

        let body = openai_response_to_anthropic(&response, "mimo-v2.5-pro").unwrap();
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["input"]["path"], "Cargo.toml");
    }
}
