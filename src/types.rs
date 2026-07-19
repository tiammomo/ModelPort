use axum::response::sse::Event;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{config::MaxTokensField, error::AppError, tool_use::ToolResponsePolicy};

pub use crate::fidelity::validate_anthropic_to_openai_fidelity;
pub use crate::tool_use::validate_anthropic_tooling;

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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AnthropicCountTokensRequest {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(default)]
    pub system: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AnthropicCountTokensRequest {
    pub fn as_message_request(&self) -> AnthropicRequest {
        AnthropicRequest {
            model: self.model.clone(),
            max_tokens: None,
            messages: self.messages.clone(),
            system: self.system.clone(),
            stream: None,
            extra: self.extra.clone(),
        }
    }
}

pub fn anthropic_count_tokens_request_value(
    request: &AnthropicCountTokensRequest,
    model: &str,
) -> Result<Value, AppError> {
    let mut value = serde_json::to_value(request)?;
    value["model"] = Value::String(model.to_owned());
    Ok(value)
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
    validate_anthropic_tooling(request)?;

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
    copy_optional(&request.extra, &mut body, "top_k");
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
        if let Some(disable_parallel) = tool_choice
            .get("disable_parallel_tool_use")
            .and_then(Value::as_bool)
        {
            body.insert("parallel_tool_calls".to_owned(), json!(!disable_parallel));
        }
    }

    Ok(Value::Object(body))
}

pub fn openai_response_to_anthropic(
    response: &Value,
    requested_model: &str,
    tool_policy: &ToolResponsePolicy,
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

    let finish_reason = choice.get("finish_reason").and_then(Value::as_str);
    let mut emitted_tool_call = false;
    let mut emitted_tool_call_count = 0usize;
    let mut emitted_tool_call_ids = std::collections::HashSet::new();
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            emitted_tool_call = true;
            emitted_tool_call_count += 1;
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4().simple()));
            if tool_policy.is_strict() && !emitted_tool_call_ids.insert(id.clone()) {
                return Err(AppError::UpstreamProtocol(format!(
                    "OpenAI-compatible upstream returned duplicate tool call id `{id}`"
                )));
            }
            let function = call.get("function").unwrap_or(&Value::Null);
            let name = tool_policy.validate_name(function.get("name").and_then(Value::as_str))?;
            let arguments = function
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input = tool_policy.parse_arguments(arguments)?;

            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }
    if !emitted_tool_call && let Some(function_call) = message.get("function_call") {
        emitted_tool_call_count = 1;
        let name = tool_policy.validate_name(function_call.get("name").and_then(Value::as_str))?;
        let arguments = function_call
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("{}");

        content.push(json!({
            "type": "tool_use",
            "id": format!("toolu_{}", Uuid::new_v4().simple()),
            "name": name,
            "input": tool_policy.parse_arguments(arguments)?
        }));
    }

    tool_policy.validate_call_summary(emitted_tool_call_count, finish_reason)?;

    let finish_reason = finish_reason.map(map_finish_reason).unwrap_or("end_turn");

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
        AppError::InvalidRequest(_)
        | AppError::IdempotencyConflict(_)
        | AppError::NotFound(_)
        | AppError::ProviderNotFound(_) => "invalid_request_error",
        AppError::Auth => "authentication_error",
        AppError::Forbidden(_) => "permission_error",
        AppError::QuotaExceeded(_) | AppError::RateLimited { .. } => "rate_limit_error",
        AppError::MissingSecret(_)
        | AppError::Config(_)
        | AppError::Database(_)
        | AppError::NotReady(_) => "server_error",
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
                    "content": tool_result_to_text(block)
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

fn tool_result_to_text(block: &Value) -> String {
    let content = content_to_text(block.get("content").unwrap_or(&Value::Null));
    if block.get("is_error").and_then(Value::as_bool) == Some(true) {
        if content.is_empty() {
            "ModelPort tool execution error".to_owned()
        } else {
            format!("ModelPort tool execution error:\n{content}")
        }
    } else {
        content
    }
}

fn convert_tools(tools: &Value) -> Result<Value, AppError> {
    let tools = tools
        .as_array()
        .ok_or_else(|| AppError::InvalidRequest("tools must be an array".to_owned()))?;

    Ok(Value::Array(
        tools
            .iter()
            .map(|tool| {
                let mut function = json!({
                    "name": tool.get("name").cloned().unwrap_or(Value::String("tool".to_owned())),
                    "description": tool.get("description").cloned().unwrap_or(Value::String(String::new())),
                    "parameters": tool
                        .get("input_schema")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }))
                });
                if let Some(strict) = tool.get("strict") {
                    function["strict"] = strict.clone();
                }
                json!({
                    "type": "function",
                    "function": function
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
    use crate::config::{ToolResponseValidation, ToolUseConfig};

    const STANDARD_MODEL: &str = "deepseek-v4-flash";

    #[test]
    fn anthropic_passthrough_preserves_deepseek_standard_request_shape() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "fast-chat",
            "max_tokens": 512,
            "system": "Keep answers concise.",
            "stream": true,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "Summarize this project."
                }]
            }],
            "tools": [{
                "name": "read_file",
                "description": "Read a file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }
            }],
            "tool_choice": { "type": "auto" }
        }))
        .unwrap();

        let body = anthropic_request_value(&request, STANDARD_MODEL).unwrap();

        assert_eq!(body["model"], STANDARD_MODEL);
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "Keep answers concise.");
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tool_choice"]["type"], "auto");
    }

    #[test]
    fn converts_deepseek_tool_conversation_to_openai_messages() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 256,
            "messages": [
                {
                    "role": "user",
                    "content": "Inspect the manifest."
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "text",
                            "text": "I'll inspect it."
                        },
                        {
                            "type": "tool_use",
                            "id": "toolu_read_manifest",
                            "name": "read_file",
                            "input": { "path": "Cargo.toml" }
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_read_manifest",
                        "content": [{
                            "type": "text",
                            "text": "name = \"model-port\""
                        }]
                    }]
                }
            ]
        }))
        .unwrap();

        let body = anthropic_to_openai_request(
            &request,
            STANDARD_MODEL,
            false,
            MaxTokensField::MaxCompletionTokens,
        )
        .unwrap();

        assert_eq!(body["model"], STANDARD_MODEL);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["id"],
            "toolu_read_manifest"
        );
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["name"],
            "read_file"
        );
        let arguments = body["messages"][1]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        let arguments: Value = serde_json::from_str(arguments).unwrap();
        assert_eq!(arguments["path"], "Cargo.toml");
        assert_eq!(body["messages"][2]["role"], "tool");
        assert_eq!(body["messages"][2]["tool_call_id"], "toolu_read_manifest");
        assert_eq!(body["messages"][2]["content"], "name = \"model-port\"");
    }

    #[test]
    fn converts_anthropic_tools_to_openai_tools() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
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
            STANDARD_MODEL,
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
    fn preserves_strict_tool_schema_for_openai_compatible_upstreams() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "tools": [{
                "name": "read_file",
                "strict": true,
                "input_schema": { "type": "object", "additionalProperties": false }
            }],
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .unwrap();

        let body = anthropic_to_openai_request(
            &request,
            STANDARD_MODEL,
            false,
            MaxTokensField::MaxCompletionTokens,
        )
        .unwrap();

        assert_eq!(body["tools"][0]["function"]["strict"], true);
    }

    #[test]
    fn converts_disable_parallel_tool_use_to_openai_parallel_tool_calls() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "tools": [{
                "name": "read_file",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    }
                }
            }],
            "tool_choice": {
                "type": "auto",
                "disable_parallel_tool_use": true
            },
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();

        let body = anthropic_to_openai_request(
            &request,
            STANDARD_MODEL,
            false,
            MaxTokensField::MaxCompletionTokens,
        )
        .unwrap();

        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], false);
    }

    #[test]
    fn rejects_duplicate_tool_names() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "tools": [
                {
                    "name": "read_file",
                    "input_schema": { "type": "object" }
                },
                {
                    "name": "read_file",
                    "input_schema": { "type": "object" }
                }
            ],
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();

        let err = validate_anthropic_tooling(&request).unwrap_err();

        assert!(err.to_string().contains("duplicates another tool"));
    }

    #[test]
    fn rejects_tool_choice_name_that_is_not_defined() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "tools": [{
                "name": "read_file",
                "input_schema": { "type": "object" }
            }],
            "tool_choice": {
                "type": "tool",
                "name": "write_file"
            },
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();

        let err = validate_anthropic_tooling(&request).unwrap_err();

        assert!(err.to_string().contains("must match a defined tool"));
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
    fn preserves_anthropic_top_k_for_openai_compatible_runtimes() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "qwen-plus",
            "max_tokens": 128,
            "top_k": 40,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .unwrap();

        let body =
            anthropic_to_openai_request(&request, "qwen-plus", false, MaxTokensField::MaxTokens)
                .unwrap();

        assert_eq!(body["top_k"], 40);
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

        let body = openai_response_to_anthropic(
            &response,
            STANDARD_MODEL,
            &ToolResponsePolicy::best_effort(),
        )
        .unwrap();
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["input"]["path"], "Cargo.toml");
    }

    #[test]
    fn strict_response_policy_rejects_undeclared_tool_name() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "tools": [{"name": "read_file", "input_schema": {"type": "object"}}],
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .unwrap();
        let tool_use = ToolUseConfig {
            response_validation: ToolResponseValidation::Strict,
            ..ToolUseConfig::default()
        };
        let policy = ToolResponsePolicy::for_anthropic_request(&request, &tool_use);
        let response = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {"tool_calls": [{
                    "id": "call_1",
                    "function": {"name": "delete_file", "arguments": "{}"}
                }]}
            }]
        });

        let error = openai_response_to_anthropic(&response, STANDARD_MODEL, &policy).unwrap_err();

        assert!(error.to_string().contains("undeclared tool `delete_file`"));
    }

    #[test]
    fn strict_response_policy_rejects_non_object_arguments() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "tools": [{"name": "read_file", "input_schema": {"type": "object"}}],
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .unwrap();
        let tool_use = ToolUseConfig {
            response_validation: ToolResponseValidation::Strict,
            ..ToolUseConfig::default()
        };
        let policy = ToolResponsePolicy::for_anthropic_request(&request, &tool_use);
        let response = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {"tool_calls": [{
                    "id": "call_1",
                    "function": {"name": "read_file", "arguments": "[\"Cargo.toml\"]"}
                }]}
            }]
        });

        let error = openai_response_to_anthropic(&response, STANDARD_MODEL, &policy).unwrap_err();

        assert!(error.to_string().contains("must be a JSON object"));
    }

    #[test]
    fn wraps_non_object_openai_tool_arguments_for_anthropic_input() {
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
                            "arguments": "[\"Cargo.toml\"]"
                        }
                    }]
                }
            }]
        });

        let body = openai_response_to_anthropic(
            &response,
            STANDARD_MODEL,
            &ToolResponsePolicy::best_effort(),
        )
        .unwrap();

        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(
            body["content"][0]["input"]["_raw_arguments"][0],
            "Cargo.toml"
        );
    }

    #[test]
    fn converts_legacy_openai_function_call_to_anthropic_tool_use() {
        let response = json!({
            "id": "chatcmpl-legacy",
            "choices": [{
                "finish_reason": "function_call",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "function_call": {
                        "name": "read_file",
                        "arguments": "{\"path\":\"Cargo.toml\"}"
                    }
                }
            }]
        });

        let body = openai_response_to_anthropic(
            &response,
            STANDARD_MODEL,
            &ToolResponsePolicy::best_effort(),
        )
        .unwrap();

        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "read_file");
        assert_eq!(body["content"][0]["input"]["path"], "Cargo.toml");
    }

    #[test]
    fn rejects_tool_use_without_id_before_openai_conversion() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "messages": [{
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "read_file",
                    "input": { "path": "Cargo.toml" }
                }]
            }]
        }))
        .unwrap();

        let err = anthropic_to_openai_request(
            &request,
            STANDARD_MODEL,
            false,
            MaxTokensField::MaxCompletionTokens,
        )
        .unwrap_err();

        assert!(err.to_string().contains("tool_use"));
        assert!(err.to_string().contains(".id is required"));
    }

    #[test]
    fn rejects_tool_result_without_prior_tool_use() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_missing",
                    "content": "missing"
                }]
            }]
        }))
        .unwrap();

        let err = validate_anthropic_tooling(&request).unwrap_err();

        assert!(
            err.to_string()
                .contains("does not match a previous tool_use id")
        );
    }

    #[test]
    fn rejects_duplicate_tool_result_for_same_tool_use() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_read",
                        "name": "read_file",
                        "input": { "path": "Cargo.toml" }
                    }]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_read",
                            "content": "first"
                        },
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_read",
                            "content": "second"
                        }
                    ]
                }
            ]
        }))
        .unwrap();

        let err = validate_anthropic_tooling(&request).unwrap_err();

        assert!(err.to_string().contains("has already been answered"));
    }

    #[test]
    fn marks_anthropic_tool_errors_in_openai_tool_content() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_read",
                        "name": "read_file",
                        "input": {"path": "missing"}
                    }]
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_read",
                        "content": "file not found",
                        "is_error": true
                    }]
                }
            ]
        }))
        .unwrap();

        let body = anthropic_to_openai_request(
            &request,
            STANDARD_MODEL,
            false,
            MaxTokensField::MaxCompletionTokens,
        )
        .unwrap();

        assert_eq!(
            body["messages"][1]["content"],
            "ModelPort tool execution error:\nfile not found"
        );
    }

    #[test]
    fn strict_fidelity_accepts_simple_text_and_tools() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
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

        validate_anthropic_to_openai_fidelity(&request).unwrap();
    }

    #[test]
    fn strict_fidelity_rejects_cache_control() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "hello",
                    "cache_control": { "type": "ephemeral" }
                }]
            }]
        }))
        .unwrap();

        let err = validate_anthropic_to_openai_fidelity(&request).unwrap_err();
        assert!(err.to_string().contains("cache_control"));
    }

    #[test]
    fn strict_fidelity_rejects_thinking_blocks() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "messages": [{
                "role": "assistant",
                "content": [{
                    "type": "thinking",
                    "thinking": "hidden chain"
                }]
            }]
        }))
        .unwrap();

        let err = validate_anthropic_to_openai_fidelity(&request).unwrap_err();
        assert!(err.to_string().contains("thinking"));
    }
}
