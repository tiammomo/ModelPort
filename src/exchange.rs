use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{
    config::{MaxTokensField, ResolvedProvider},
    domain::ClientProtocol,
    error::AppError,
    pricing::{self, TokenUsageBreakdown},
    tool_use::validate_anthropic_tool_capabilities,
    types::{AnthropicRequest, anthropic_request_value, anthropic_to_openai_request},
};

const DEFAULT_OPENAI_MAX_OUTPUT_TOKENS: u64 = 4_096;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct OpenAiChatRequest {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) messages: Vec<Value>,
    #[serde(default)]
    pub(crate) max_completion_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) max_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) stream: Option<bool>,
    #[serde(flatten)]
    pub(crate) extra: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub(crate) enum ClientRequest {
    Anthropic(AnthropicRequest),
    OpenAiChat(OpenAiChatRequest),
}

#[derive(Debug, Clone)]
pub(crate) struct ExchangeRequest {
    source: ClientRequest,
    pub(crate) requested_model: String,
    pub(crate) max_output_tokens: Option<u64>,
    pub(crate) stream: bool,
    messages: Vec<ExchangeMessage>,
    tools: Vec<ExchangeTool>,
    tool_choice: Option<ExchangeToolChoice>,
    parallel_tool_calls: Option<bool>,
    parameters: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExchangeRole {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
struct ExchangeMessage {
    role: ExchangeRole,
    content: Vec<ExchangeContent>,
}

#[derive(Debug, Clone)]
enum ExchangeContent {
    Text(String),
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone)]
struct ExchangeTool {
    name: String,
    description: Option<String>,
    input_schema: Value,
    strict: Option<bool>,
}

#[derive(Debug, Clone)]
enum ExchangeToolChoice {
    None,
    Auto,
    Required,
    Function(String),
}

impl ClientRequest {
    pub(crate) fn route_name(&self) -> &'static str {
        match self {
            Self::Anthropic(_) => "messages",
            Self::OpenAiChat(_) => "chat_completions",
        }
    }

    pub(crate) fn request_path(&self) -> &'static str {
        match self {
            Self::Anthropic(_) => "/v1/messages",
            Self::OpenAiChat(_) => "/v1/chat/completions",
        }
    }
}

impl ExchangeRequest {
    pub(crate) fn from_client(source: ClientRequest) -> Result<Self, AppError> {
        match source {
            ClientRequest::Anthropic(request) => Self::from_anthropic(request),
            ClientRequest::OpenAiChat(request) => Self::from_openai(request),
        }
    }

    fn from_anthropic(request: AnthropicRequest) -> Result<Self, AppError> {
        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(ExchangeMessage {
                role: ExchangeRole::System,
                content: vec![ExchangeContent::Text(anthropic_text(system)?)],
            });
        }

        for (index, message) in request.messages.iter().enumerate() {
            messages.push(parse_anthropic_message(message, index)?);
        }

        let tools = request
            .extra
            .get("tools")
            .map(parse_anthropic_tools)
            .transpose()?
            .unwrap_or_default();
        let tool_choice = request
            .extra
            .get("tool_choice")
            .map(parse_anthropic_tool_choice)
            .transpose()?;
        let parallel_tool_calls = request
            .extra
            .get("tool_choice")
            .and_then(|choice| choice.get("disable_parallel_tool_use"))
            .and_then(Value::as_bool)
            .map(|disabled| !disabled);
        let parameters = request
            .extra
            .iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "temperature"
                        | "top_p"
                        | "presence_penalty"
                        | "frequency_penalty"
                        | "seed"
                        | "stop_sequences"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        Ok(Self {
            requested_model: request.model.clone(),
            max_output_tokens: request.max_tokens,
            stream: request.stream.unwrap_or(false),
            source: ClientRequest::Anthropic(request),
            messages,
            tools,
            tool_choice,
            parallel_tool_calls,
            parameters,
        })
    }

    fn from_openai(request: OpenAiChatRequest) -> Result<Self, AppError> {
        validate_openai_request_shape(&request)?;
        let messages = request
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| parse_openai_message(message, index))
            .collect::<Result<Vec<_>, _>>()?;
        let tools = request
            .extra
            .get("tools")
            .map(parse_openai_tools)
            .transpose()?
            .unwrap_or_default();
        let tool_choice = request
            .extra
            .get("tool_choice")
            .map(parse_openai_tool_choice)
            .transpose()?;
        if let Some(ExchangeToolChoice::Function(name)) = &tool_choice
            && !tools.iter().any(|tool| &tool.name == name)
        {
            return Err(AppError::InvalidRequest(format!(
                "tool_choice function `{name}` is not declared in tools"
            )));
        }
        if matches!(
            &tool_choice,
            Some(ExchangeToolChoice::Required | ExchangeToolChoice::Function(_))
        ) && tools.is_empty()
        {
            return Err(AppError::InvalidRequest(
                "required or named tool_choice requires at least one tool".to_owned(),
            ));
        }
        let parallel_tool_calls = request
            .extra
            .get("parallel_tool_calls")
            .map(|value| {
                value.as_bool().ok_or_else(|| {
                    AppError::InvalidRequest("parallel_tool_calls must be a boolean".to_owned())
                })
            })
            .transpose()?;
        if parallel_tool_calls.is_some() && tools.is_empty() {
            return Err(AppError::InvalidRequest(
                "parallel_tool_calls requires at least one tool".to_owned(),
            ));
        }
        let parameters = request
            .extra
            .iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "temperature"
                        | "top_p"
                        | "presence_penalty"
                        | "frequency_penalty"
                        | "seed"
                        | "stop"
                        | "response_format"
                        | "stream_options"
                        | "n"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let max_output_tokens = request.max_completion_tokens.or(request.max_tokens);

        Ok(Self {
            requested_model: request.model.clone(),
            max_output_tokens,
            stream: request.stream.unwrap_or(false),
            source: ClientRequest::OpenAiChat(request),
            messages,
            tools,
            tool_choice,
            parallel_tool_calls,
            parameters,
        })
    }

    pub(crate) fn client_protocol(&self) -> ClientProtocol {
        match &self.source {
            ClientRequest::Anthropic(_) => ClientProtocol::AnthropicMessages,
            ClientRequest::OpenAiChat(_) => ClientProtocol::OpenAiChatCompletions,
        }
    }

    pub(crate) fn request_path(&self) -> &'static str {
        self.source.request_path()
    }

    pub(crate) fn estimated_output_tokens(&self) -> u64 {
        self.max_output_tokens
            .unwrap_or(DEFAULT_OPENAI_MAX_OUTPUT_TOKENS)
    }

    pub(crate) fn serialized_input_chars(&self) -> usize {
        match &self.source {
            ClientRequest::Anthropic(request) => serde_json::to_string(request),
            ClientRequest::OpenAiChat(request) => serde_json::to_string(request),
        }
        .map(|value| value.chars().count())
        .unwrap_or(0)
    }

    pub(crate) fn request_fingerprint(&self) -> Result<String, AppError> {
        let body = match &self.source {
            ClientRequest::Anthropic(request) => serde_json::to_vec(request)?,
            ClientRequest::OpenAiChat(request) => serde_json::to_vec(request)?,
        };
        let mut hasher = Sha256::new();
        hasher.update(self.client_protocol().as_str().as_bytes());
        hasher.update([0]);
        hasher.update(body);
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub(crate) fn validate_provider(&self, resolved: &ResolvedProvider) -> Result<(), AppError> {
        match &self.source {
            ClientRequest::Anthropic(request) => validate_anthropic_tool_capabilities(
                request,
                &resolved.provider_id,
                &resolved.provider.tool_use,
            )?,
            ClientRequest::OpenAiChat(_) => {
                if self.uses_tools() && !resolved.provider.tool_use.supported {
                    return Err(AppError::InvalidRequest(format!(
                        "provider `{}` does not support tool use",
                        resolved.provider_id
                    )));
                }
                if self.tool_choice.is_some() && !resolved.provider.tool_use.tool_choice {
                    return Err(AppError::InvalidRequest(format!(
                        "provider `{}` does not support tool_choice",
                        resolved.provider_id
                    )));
                }
                if self.parallel_tool_calls == Some(true)
                    && !resolved.provider.tool_use.parallel_tool_calls
                {
                    return Err(AppError::InvalidRequest(format!(
                        "provider `{}` does not support parallel tool calls",
                        resolved.provider_id
                    )));
                }
            }
        }

        if resolved.provider.protocol == crate::config::ProviderProtocol::Anthropic
            && matches!(&self.source, ClientRequest::OpenAiChat(_))
        {
            self.validate_openai_to_anthropic_fidelity()?;
        }
        Ok(())
    }

    fn validate_openai_to_anthropic_fidelity(&self) -> Result<(), AppError> {
        let unsupported_parameters = ["presence_penalty", "frequency_penalty", "seed"]
            .into_iter()
            .filter(|key| {
                self.parameters
                    .get(*key)
                    .is_some_and(|value| !value.is_null())
            })
            .collect::<Vec<_>>();
        if !unsupported_parameters.is_empty() {
            return Err(AppError::InvalidRequest(format!(
                "parameter(s) {} cannot be preserved by the selected Anthropic provider",
                unsupported_parameters.join(", ")
            )));
        }
        if self
            .parameters
            .get("response_format")
            .is_some_and(|format| format.get("type").and_then(Value::as_str) != Some("text"))
        {
            return Err(AppError::InvalidRequest(
                "response_format is not supported when routing Chat Completions to an Anthropic provider"
                    .to_owned(),
            ));
        }
        if self.tools.iter().any(|tool| tool.strict == Some(true)) {
            return Err(AppError::InvalidRequest(
                "strict function schemas cannot be preserved by the selected Anthropic provider"
                    .to_owned(),
            ));
        }
        let first_conversation = self.messages.iter().position(|message| {
            matches!(
                message.role,
                ExchangeRole::User | ExchangeRole::Assistant | ExchangeRole::Tool
            )
        });
        if let Some(first_conversation) = first_conversation
            && self.messages[first_conversation..].iter().any(|message| {
                matches!(message.role, ExchangeRole::System | ExchangeRole::Developer)
            })
        {
            return Err(AppError::InvalidRequest(
                "system/developer messages after conversation start cannot be preserved by an Anthropic provider"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn uses_tools(&self) -> bool {
        !self.tools.is_empty()
            || self.tool_choice.is_some()
            || self.messages.iter().any(|message| {
                message.content.iter().any(|content| {
                    matches!(
                        content,
                        ExchangeContent::ToolCall { .. } | ExchangeContent::ToolResult { .. }
                    )
                })
            })
    }

    pub(crate) fn to_openai_request(
        &self,
        model: &str,
        stream: bool,
        max_tokens_field: MaxTokensField,
    ) -> Result<Value, AppError> {
        match &self.source {
            ClientRequest::Anthropic(request) => {
                anthropic_to_openai_request(request, model, stream, max_tokens_field)
            }
            ClientRequest::OpenAiChat(request) => {
                let mut body = serde_json::to_value(request)?;
                let object = body.as_object_mut().ok_or_else(|| {
                    AppError::InvalidRequest("Chat Completions body must be an object".to_owned())
                })?;
                object.insert("model".to_owned(), Value::String(model.to_owned()));
                object.insert("stream".to_owned(), Value::Bool(stream));
                if !stream {
                    object.remove("stream_options");
                }
                object.remove("max_completion_tokens");
                object.remove("max_tokens");
                if let Some(max_tokens) = self.max_output_tokens {
                    match max_tokens_field {
                        MaxTokensField::MaxCompletionTokens => {
                            object.insert("max_completion_tokens".to_owned(), json!(max_tokens));
                        }
                        MaxTokensField::MaxTokens => {
                            object.insert("max_tokens".to_owned(), json!(max_tokens));
                        }
                        MaxTokensField::Both => {
                            object.insert("max_completion_tokens".to_owned(), json!(max_tokens));
                            object.insert("max_tokens".to_owned(), json!(max_tokens));
                        }
                    }
                }
                Ok(body)
            }
        }
    }

    pub(crate) fn to_anthropic_request(
        &self,
        model: &str,
        stream: bool,
    ) -> Result<Value, AppError> {
        if let ClientRequest::Anthropic(request) = &self.source {
            let mut request = request.clone();
            request.stream = Some(stream);
            return anthropic_request_value(&request, model);
        }

        let mut system_parts = Vec::new();
        let mut messages = Vec::new();
        for message in &self.messages {
            match message.role {
                ExchangeRole::System | ExchangeRole::Developer => {
                    system_parts.extend(message.content.iter().filter_map(|content| match content {
                        ExchangeContent::Text(text) => Some(text.clone()),
                        _ => None,
                    }));
                }
                ExchangeRole::User => messages.push(json!({
                    "role": "user",
                    "content": message.content.iter().filter_map(|content| match content {
                        ExchangeContent::Text(text) => Some(json!({ "type": "text", "text": text })),
                        _ => None,
                    }).collect::<Vec<_>>()
                })),
                ExchangeRole::Assistant => {
                    let blocks = message
                        .content
                        .iter()
                        .map(|content| match content {
                            ExchangeContent::Text(text) => Ok(json!({
                                "type": "text",
                                "text": text
                            })),
                            ExchangeContent::ToolCall { id, name, arguments } => {
                                let input = serde_json::from_str::<Value>(arguments).map_err(|error| {
                                    AppError::InvalidRequest(format!(
                                        "tool call `{id}` arguments must be valid JSON for an Anthropic provider: {error}"
                                    ))
                                })?;
                                if !input.is_object() {
                                    return Err(AppError::InvalidRequest(format!(
                                        "tool call `{id}` arguments must be a JSON object for an Anthropic provider"
                                    )));
                                }
                                Ok(json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input
                                }))
                            }
                            ExchangeContent::ToolResult { .. } => Err(AppError::InvalidRequest(
                                "tool results cannot appear in an assistant message".to_owned(),
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    messages.push(json!({ "role": "assistant", "content": blocks }));
                }
                ExchangeRole::Tool => {
                    let blocks = message
                        .content
                        .iter()
                        .filter_map(|content| match content {
                            ExchangeContent::ToolResult {
                                tool_call_id,
                                content,
                            } => Some(json!({
                                "type": "tool_result",
                                "tool_use_id": tool_call_id,
                                "content": content
                            })),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    messages.push(json!({ "role": "user", "content": blocks }));
                }
            }
        }

        let mut body = Map::new();
        body.insert("model".to_owned(), Value::String(model.to_owned()));
        body.insert(
            "max_tokens".to_owned(),
            json!(
                self.max_output_tokens
                    .unwrap_or(DEFAULT_OPENAI_MAX_OUTPUT_TOKENS)
            ),
        );
        body.insert("messages".to_owned(), Value::Array(messages));
        body.insert("stream".to_owned(), Value::Bool(stream));
        if !system_parts.is_empty() {
            body.insert(
                "system".to_owned(),
                Value::String(system_parts.join("\n\n")),
            );
        }
        for key in ["temperature", "top_p"] {
            if let Some(value) = self.parameters.get(key).filter(|value| !value.is_null()) {
                body.insert(key.to_owned(), value.clone());
            }
        }
        if let Some(stop) = self.parameters.get("stop").filter(|value| !value.is_null()) {
            body.insert(
                "stop_sequences".to_owned(),
                stop.as_str()
                    .map(|value| json!([value]))
                    .unwrap_or_else(|| stop.clone()),
            );
        }
        let tools_disabled = matches!(&self.tool_choice, Some(ExchangeToolChoice::None));
        if !self.tools.is_empty() && !tools_disabled {
            body.insert(
                "tools".to_owned(),
                Value::Array(
                    self.tools
                        .iter()
                        .map(|tool| {
                            json!({
                                "name": tool.name,
                                "description": tool.description,
                                "input_schema": tool.input_schema
                            })
                        })
                        .collect(),
                ),
            );
        }
        if let Some(choice) = &self.tool_choice
            && !matches!(choice, ExchangeToolChoice::None)
        {
            let mut choice = match choice {
                ExchangeToolChoice::None => unreachable!("disabled tools are omitted above"),
                ExchangeToolChoice::Auto => json!({ "type": "auto" }),
                ExchangeToolChoice::Required => json!({ "type": "any" }),
                ExchangeToolChoice::Function(name) => {
                    json!({ "type": "tool", "name": name })
                }
            };
            if let Some(parallel) = self.parallel_tool_calls {
                choice["disable_parallel_tool_use"] = Value::Bool(!parallel);
            }
            body.insert("tool_choice".to_owned(), choice);
        } else if !tools_disabled && let Some(parallel) = self.parallel_tool_calls {
            body.insert(
                "tool_choice".to_owned(),
                json!({
                    "type": "auto",
                    "disable_parallel_tool_use": !parallel
                }),
            );
        }
        Ok(Value::Object(body))
    }

    pub(crate) fn is_anthropic_client(&self) -> bool {
        matches!(&self.source, ClientRequest::Anthropic(_))
    }

    pub(crate) fn into_source(self) -> ClientRequest {
        self.source
    }

    pub(crate) fn include_stream_usage(&self) -> bool {
        self.parameters
            .get("stream_options")
            .and_then(|options| options.get("include_usage"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

pub(crate) fn anthropic_response_to_openai(
    response: &Value,
    requested_model: &str,
) -> Result<Value, AppError> {
    let blocks = response
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::UpstreamProtocol("Anthropic response has no content array".to_owned())
        })?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("tool_use") => tool_calls.push(json!({
                "id": block.get("id").and_then(Value::as_str).unwrap_or("call_modelport"),
                "type": "function",
                "function": {
                    "name": block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                    "arguments": serde_json::to_string(
                        block.get("input").unwrap_or(&Value::Object(Map::new()))
                    )?
                }
            })),
            _ => {}
        }
    }
    let stop_reason = response
        .get("stop_reason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn");
    let finish_reason = match stop_reason {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    };
    let usage = pricing::anthropic_usage(response);
    let mut message = json!({
        "role": "assistant",
        "content": if text.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            Value::String(text)
        }
    });
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    Ok(json!({
        "id": openai_completion_id(response.get("id").and_then(Value::as_str)),
        "object": "chat.completion",
        "created": unix_timestamp_seconds(),
        "model": requested_model,
        "choices": [{
            "index": 0,
            "message": message,
            "logprobs": null,
            "finish_reason": finish_reason
        }],
        "usage": openai_usage_value(usage)
    }))
}

pub(crate) fn openai_usage_value(usage: TokenUsageBreakdown) -> Value {
    json!({
        "prompt_tokens": usage
            .input_tokens
            .saturating_add(usage.cache_write_tokens)
            .saturating_add(usage.cache_read_tokens),
        "completion_tokens": usage.output_tokens,
        "total_tokens": usage
            .input_tokens
            .saturating_add(usage.cache_write_tokens)
            .saturating_add(usage.cache_read_tokens)
            .saturating_add(usage.output_tokens),
        "prompt_tokens_details": {
            "cached_tokens": usage.cache_read_tokens
        }
    })
}

pub(crate) fn openai_completion_id(upstream_id: Option<&str>) -> String {
    upstream_id
        .filter(|id| id.starts_with("chatcmpl-"))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()))
}

pub(crate) fn unix_timestamp_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn validate_openai_request_shape(request: &OpenAiChatRequest) -> Result<(), AppError> {
    if request.model.trim().is_empty() {
        return Err(AppError::InvalidRequest("model is required".to_owned()));
    }
    if request.messages.is_empty() {
        return Err(AppError::InvalidRequest(
            "messages must not be empty".to_owned(),
        ));
    }
    if let (Some(max_completion_tokens), Some(max_tokens)) =
        (request.max_completion_tokens, request.max_tokens)
        && max_completion_tokens != max_tokens
    {
        return Err(AppError::InvalidRequest(
            "max_completion_tokens and max_tokens must not conflict".to_owned(),
        ));
    }
    if request
        .max_completion_tokens
        .or(request.max_tokens)
        .is_some_and(|value| value == 0)
    {
        return Err(AppError::InvalidRequest(
            "max_completion_tokens/max_tokens must be greater than 0".to_owned(),
        ));
    }
    const SUPPORTED_FIELDS: &[&str] = &[
        "temperature",
        "top_p",
        "presence_penalty",
        "frequency_penalty",
        "seed",
        "stop",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "response_format",
        "stream_options",
        "n",
    ];
    let unsupported = request
        .extra
        .keys()
        .filter(|key| !SUPPORTED_FIELDS.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        return Err(AppError::InvalidRequest(format!(
            "unsupported Chat Completions field(s): {}",
            unsupported.join(", ")
        )));
    }
    if let Some(n) = request.extra.get("n")
        && n.as_u64() != Some(1)
    {
        return Err(AppError::InvalidRequest("only n=1 is supported".to_owned()));
    }
    if let Some(stream_options) = request.extra.get("stream_options") {
        if request.stream != Some(true) {
            return Err(AppError::InvalidRequest(
                "stream_options requires stream=true".to_owned(),
            ));
        }
        let options = stream_options.as_object().ok_or_else(|| {
            AppError::InvalidRequest("stream_options must be an object".to_owned())
        })?;
        if options.keys().any(|key| key.as_str() != "include_usage")
            || options
                .get("include_usage")
                .is_some_and(|value| !value.is_boolean())
        {
            return Err(AppError::InvalidRequest(
                "only stream_options.include_usage is supported".to_owned(),
            ));
        }
    }
    if let Some(response_format) = request.extra.get("response_format") {
        let format = response_format.as_object().ok_or_else(|| {
            AppError::InvalidRequest("response_format must be an object".to_owned())
        })?;
        if format.len() != 1 || format.get("type").and_then(Value::as_str) != Some("text") {
            return Err(AppError::InvalidRequest(
                "only response_format.type=text is supported in the current Chat Completions compatibility slice"
                    .to_owned(),
            ));
        }
    }
    validate_optional_number(request, "temperature")?;
    validate_optional_number(request, "top_p")?;
    validate_optional_number(request, "presence_penalty")?;
    validate_optional_number(request, "frequency_penalty")?;
    if let Some(seed) = request.extra.get("seed")
        && !seed.is_null()
        && !seed.is_i64()
        && !seed.is_u64()
    {
        return Err(AppError::InvalidRequest(
            "seed must be an integer or null".to_owned(),
        ));
    }
    if let Some(stop) = request.extra.get("stop") {
        let valid = stop.is_null()
            || stop.is_string()
            || stop.as_array().is_some_and(|values| {
                !values.is_empty() && values.len() <= 4 && values.iter().all(Value::is_string)
            });
        if !valid {
            return Err(AppError::InvalidRequest(
                "stop must be null, a string, or an array of 1 to 4 strings".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_optional_number(request: &OpenAiChatRequest, field: &str) -> Result<(), AppError> {
    if request
        .extra
        .get(field)
        .is_some_and(|value| !value.is_null() && !value.is_number())
    {
        return Err(AppError::InvalidRequest(format!(
            "{field} must be a number or null"
        )));
    }
    Ok(())
}

fn parse_openai_message(message: &Value, index: usize) -> Result<ExchangeMessage, AppError> {
    let object = message
        .as_object()
        .ok_or_else(|| AppError::InvalidRequest(format!("messages[{index}] must be an object")))?;
    let role = match object.get("role").and_then(Value::as_str) {
        Some("system") => ExchangeRole::System,
        Some("developer") => ExchangeRole::Developer,
        Some("user") => ExchangeRole::User,
        Some("assistant") => ExchangeRole::Assistant,
        Some("tool") => ExchangeRole::Tool,
        Some(role) => {
            return Err(AppError::InvalidRequest(format!(
                "messages[{index}].role `{role}` is not supported"
            )));
        }
        None => {
            return Err(AppError::InvalidRequest(format!(
                "messages[{index}].role is required"
            )));
        }
    };
    let allowed_keys: &[&str] = match role {
        ExchangeRole::Assistant => &["role", "content", "tool_calls"],
        ExchangeRole::Tool => &["role", "content", "tool_call_id"],
        _ => &["role", "content"],
    };
    if let Some(key) = object
        .keys()
        .find(|key| !allowed_keys.contains(&key.as_str()))
    {
        return Err(AppError::InvalidRequest(format!(
            "messages[{index}].{key} is not supported"
        )));
    }

    let mut content = Vec::new();
    if role == ExchangeRole::Tool {
        let tool_call_id = object
            .get("tool_call_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::InvalidRequest(format!("messages[{index}].tool_call_id is required"))
            })?;
        content.push(ExchangeContent::ToolResult {
            tool_call_id: tool_call_id.to_owned(),
            content: openai_text_content(
                object.get("content").unwrap_or(&Value::Null),
                &format!("messages[{index}].content"),
            )?,
        });
    } else if let Some(value) = object.get("content")
        && !value.is_null()
    {
        let text = openai_text_content(value, &format!("messages[{index}].content"))?;
        if !text.is_empty() {
            content.push(ExchangeContent::Text(text));
        }
    }
    if role == ExchangeRole::Assistant
        && let Some(tool_calls) = object.get("tool_calls")
    {
        let tool_calls = tool_calls.as_array().ok_or_else(|| {
            AppError::InvalidRequest(format!("messages[{index}].tool_calls must be an array"))
        })?;
        for (call_index, call) in tool_calls.iter().enumerate() {
            let call_object = call.as_object().ok_or_else(|| {
                AppError::InvalidRequest(format!(
                    "messages[{index}].tool_calls[{call_index}] must be an object"
                ))
            })?;
            if let Some(key) = call_object
                .keys()
                .find(|key| !matches!(key.as_str(), "id" | "type" | "function"))
            {
                return Err(AppError::InvalidRequest(format!(
                    "messages[{index}].tool_calls[{call_index}].{key} is not supported"
                )));
            }
            let function = call.get("function").ok_or_else(|| {
                AppError::InvalidRequest(format!(
                    "messages[{index}].tool_calls[{call_index}].function is required"
                ))
            })?;
            if call.get("type").and_then(Value::as_str) != Some("function") {
                return Err(AppError::InvalidRequest(format!(
                    "messages[{index}].tool_calls[{call_index}].type must be function"
                )));
            }
            let function_object = function.as_object().ok_or_else(|| {
                AppError::InvalidRequest(format!(
                    "messages[{index}].tool_calls[{call_index}].function must be an object"
                ))
            })?;
            if let Some(key) = function_object
                .keys()
                .find(|key| !matches!(key.as_str(), "name" | "arguments"))
            {
                return Err(AppError::InvalidRequest(format!(
                    "messages[{index}].tool_calls[{call_index}].function.{key} is not supported"
                )));
            }
            content.push(ExchangeContent::ToolCall {
                id: required_string(
                    call,
                    "id",
                    &format!("messages[{index}].tool_calls[{call_index}]"),
                )?,
                name: required_string(
                    function,
                    "name",
                    &format!("messages[{index}].tool_calls[{call_index}].function"),
                )?,
                arguments: required_string(
                    function,
                    "arguments",
                    &format!("messages[{index}].tool_calls[{call_index}].function"),
                )?,
            });
        }
    }
    if content.is_empty() && role != ExchangeRole::Assistant {
        return Err(AppError::InvalidRequest(format!(
            "messages[{index}].content must not be empty"
        )));
    }
    Ok(ExchangeMessage { role, content })
}

fn parse_anthropic_message(message: &Value, index: usize) -> Result<ExchangeMessage, AppError> {
    let role = match message.get("role").and_then(Value::as_str) {
        Some("user") => ExchangeRole::User,
        Some("assistant") => ExchangeRole::Assistant,
        _ => {
            return Err(AppError::InvalidRequest(format!(
                "messages[{index}].role must be user or assistant"
            )));
        }
    };
    let value = message.get("content").unwrap_or(&Value::Null);
    if let Some(text) = value.as_str() {
        return Ok(ExchangeMessage {
            role,
            content: vec![ExchangeContent::Text(text.to_owned())],
        });
    }
    let blocks = value.as_array().ok_or_else(|| {
        AppError::InvalidRequest(format!(
            "messages[{index}].content must be a string or array"
        ))
    })?;
    let mut content = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => content.push(ExchangeContent::Text(required_string(
                block,
                "text",
                &format!("messages[{index}].content[{block_index}]"),
            )?)),
            Some("tool_use") => content.push(ExchangeContent::ToolCall {
                id: required_string(
                    block,
                    "id",
                    &format!("messages[{index}].content[{block_index}]"),
                )?,
                name: required_string(
                    block,
                    "name",
                    &format!("messages[{index}].content[{block_index}]"),
                )?,
                arguments: serde_json::to_string(
                    block.get("input").unwrap_or(&Value::Object(Map::new())),
                )?,
            }),
            Some("tool_result") => content.push(ExchangeContent::ToolResult {
                tool_call_id: required_string(
                    block,
                    "tool_use_id",
                    &format!("messages[{index}].content[{block_index}]"),
                )?,
                content: anthropic_text(block.get("content").unwrap_or(&Value::Null))?,
            }),
            Some(kind) => {
                return Err(AppError::InvalidRequest(format!(
                    "messages[{index}].content[{block_index}] type `{kind}` is not in the current Exchange IR"
                )));
            }
            None => {
                return Err(AppError::InvalidRequest(format!(
                    "messages[{index}].content[{block_index}].type is required"
                )));
            }
        }
    }
    Ok(ExchangeMessage { role, content })
}

fn parse_openai_tools(value: &Value) -> Result<Vec<ExchangeTool>, AppError> {
    let tools = value
        .as_array()
        .ok_or_else(|| AppError::InvalidRequest("tools must be an array".to_owned()))?;
    let mut names = HashSet::new();
    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let tool_object = tool.as_object().ok_or_else(|| {
                AppError::InvalidRequest(format!("tools[{index}] must be an object"))
            })?;
            if let Some(key) = tool_object
                .keys()
                .find(|key| !matches!(key.as_str(), "type" | "function"))
            {
                return Err(AppError::InvalidRequest(format!(
                    "tools[{index}].{key} is not supported"
                )));
            }
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                return Err(AppError::InvalidRequest(format!(
                    "tools[{index}].type must be function"
                )));
            }
            let function = tool.get("function").ok_or_else(|| {
                AppError::InvalidRequest(format!("tools[{index}].function is required"))
            })?;
            let function_object = function.as_object().ok_or_else(|| {
                AppError::InvalidRequest(format!("tools[{index}].function must be an object"))
            })?;
            if let Some(key) = function_object.keys().find(|key| {
                !matches!(
                    key.as_str(),
                    "name" | "description" | "parameters" | "strict"
                )
            }) {
                return Err(AppError::InvalidRequest(format!(
                    "tools[{index}].function.{key} is not supported"
                )));
            }
            let name = required_string(function, "name", &format!("tools[{index}].function"))?;
            validate_tool_name(&name, &format!("tools[{index}].function.name"))?;
            if !names.insert(name.clone()) {
                return Err(AppError::InvalidRequest(format!(
                    "tools[{index}].function.name duplicates another tool"
                )));
            }
            let description = function
                .get("description")
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        AppError::InvalidRequest(format!(
                            "tools[{index}].function.description must be a string"
                        ))
                    })
                })
                .transpose()?;
            let input_schema = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            if !input_schema.is_object() {
                return Err(AppError::InvalidRequest(format!(
                    "tools[{index}].function.parameters must be an object"
                )));
            }
            let strict = function
                .get("strict")
                .map(|value| {
                    value.as_bool().ok_or_else(|| {
                        AppError::InvalidRequest(format!(
                            "tools[{index}].function.strict must be a boolean"
                        ))
                    })
                })
                .transpose()?;
            Ok(ExchangeTool {
                name,
                description,
                input_schema,
                strict,
            })
        })
        .collect()
}

fn parse_anthropic_tools(value: &Value) -> Result<Vec<ExchangeTool>, AppError> {
    let tools = value
        .as_array()
        .ok_or_else(|| AppError::InvalidRequest("tools must be an array".to_owned()))?;
    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let name = required_string(tool, "name", &format!("tools[{index}]"))?;
            Ok(ExchangeTool {
                name,
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                input_schema: tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                strict: None,
            })
        })
        .collect()
}

fn parse_openai_tool_choice(value: &Value) -> Result<ExchangeToolChoice, AppError> {
    if let Some(choice) = value.as_str() {
        return match choice {
            "none" => Ok(ExchangeToolChoice::None),
            "auto" => Ok(ExchangeToolChoice::Auto),
            "required" => Ok(ExchangeToolChoice::Required),
            _ => Err(AppError::InvalidRequest(
                "tool_choice must be none, auto, required, or a function object".to_owned(),
            )),
        };
    }
    let choice = value.as_object().ok_or_else(|| {
        AppError::InvalidRequest("tool_choice must be a string or object".to_owned())
    })?;
    if let Some(key) = choice
        .keys()
        .find(|key| !matches!(key.as_str(), "type" | "function"))
    {
        return Err(AppError::InvalidRequest(format!(
            "tool_choice.{key} is not supported"
        )));
    }
    let function = value
        .get("function")
        .ok_or_else(|| AppError::InvalidRequest("tool_choice.function is required".to_owned()))?;
    let function_object = function.as_object().ok_or_else(|| {
        AppError::InvalidRequest("tool_choice.function must be an object".to_owned())
    })?;
    if let Some(key) = function_object.keys().find(|key| key.as_str() != "name") {
        return Err(AppError::InvalidRequest(format!(
            "tool_choice.function.{key} is not supported"
        )));
    }
    if value.get("type").and_then(Value::as_str) != Some("function") {
        return Err(AppError::InvalidRequest(
            "tool_choice.type must be function".to_owned(),
        ));
    }
    Ok(ExchangeToolChoice::Function(required_string(
        function,
        "name",
        "tool_choice.function",
    )?))
}

fn parse_anthropic_tool_choice(value: &Value) -> Result<ExchangeToolChoice, AppError> {
    match value.get("type").and_then(Value::as_str) {
        Some("none") => Ok(ExchangeToolChoice::None),
        Some("auto") => Ok(ExchangeToolChoice::Auto),
        Some("any") => Ok(ExchangeToolChoice::Required),
        Some("tool") => Ok(ExchangeToolChoice::Function(required_string(
            value,
            "name",
            "tool_choice",
        )?)),
        _ => Err(AppError::InvalidRequest(
            "tool_choice.type must be none, auto, any, or tool".to_owned(),
        )),
    }
}

fn openai_text_content(value: &Value, path: &str) -> Result<String, AppError> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_owned());
    }
    let blocks = value
        .as_array()
        .ok_or_else(|| AppError::InvalidRequest(format!("{path} must be a string or array")))?;
    let mut text = String::new();
    for (index, block) in blocks.iter().enumerate() {
        let object = block.as_object().ok_or_else(|| {
            AppError::InvalidRequest(format!("{path}[{index}] must be an object"))
        })?;
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "type" | "text"))
        {
            return Err(AppError::InvalidRequest(format!(
                "{path}[{index}] contains unsupported fields"
            )));
        }
        if block.get("type").and_then(Value::as_str) != Some("text") {
            return Err(AppError::InvalidRequest(format!(
                "{path}[{index}] only supports text content in the current Exchange IR"
            )));
        }
        text.push_str(block.get("text").and_then(Value::as_str).ok_or_else(|| {
            AppError::InvalidRequest(format!("{path}[{index}].text is required"))
        })?);
    }
    Ok(text)
}

fn anthropic_text(value: &Value) -> Result<String, AppError> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_owned());
    }
    let blocks = value.as_array().ok_or_else(|| {
        AppError::InvalidRequest("Anthropic content must be a string or array".to_owned())
    })?;
    let mut text = String::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("tool_result") => {
                text.push_str(&anthropic_text(
                    block.get("content").unwrap_or(&Value::Null),
                )?);
            }
            Some(kind) => {
                return Err(AppError::InvalidRequest(format!(
                    "Anthropic content type `{kind}` is not in the current Exchange IR"
                )));
            }
            None => {
                return Err(AppError::InvalidRequest(
                    "Anthropic content block type is required".to_owned(),
                ));
            }
        }
    }
    Ok(text)
}

fn required_string(value: &Value, key: &str, path: &str) -> Result<String, AppError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::InvalidRequest(format!("{path}.{key} is required")))
}

fn validate_tool_name(name: &str, path: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(AppError::InvalidRequest(format!(
            "{path} must be 1-64 ASCII letters, digits, underscores, or hyphens"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_text_and_tools_parse_into_exchange_and_render_for_anthropic() {
        let request: OpenAiChatRequest = serde_json::from_value(json!({
            "model": "chat-model",
            "messages": [
                { "role": "developer", "content": "Be concise." },
                { "role": "user", "content": "Read the manifest." },
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"Cargo.toml\"}"
                        }
                    }]
                },
                { "role": "tool", "tool_call_id": "call_1", "content": "model-port" }
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "read_file",
                    "parameters": { "type": "object" }
                }
            }],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stop": "END"
        }))
        .unwrap();

        let exchange = ExchangeRequest::from_client(ClientRequest::OpenAiChat(request)).unwrap();
        let rendered = exchange.to_anthropic_request("claude-test", false).unwrap();

        assert_eq!(rendered["model"], "claude-test");
        assert_eq!(rendered["system"], "Be concise.");
        assert_eq!(rendered["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(
            rendered["messages"][2]["content"][0]["tool_use_id"],
            "call_1"
        );
        assert_eq!(rendered["tools"][0]["name"], "read_file");
        assert_eq!(rendered["tool_choice"]["disable_parallel_tool_use"], true);
        assert_eq!(rendered["stop_sequences"], json!(["END"]));
    }

    #[test]
    fn unsupported_openai_fields_fail_instead_of_disappearing() {
        let request: OpenAiChatRequest = serde_json::from_value(json!({
            "model": "chat-model",
            "messages": [{ "role": "user", "content": "hello" }],
            "logprobs": true
        }))
        .unwrap();

        let error = ExchangeRequest::from_client(ClientRequest::OpenAiChat(request)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported Chat Completions field")
        );
        assert!(error.to_string().contains("logprobs"));
    }

    #[test]
    fn unsupported_response_format_fails_in_current_slice() {
        let request: OpenAiChatRequest = serde_json::from_value(json!({
            "model": "chat-model",
            "messages": [{ "role": "user", "content": "hello" }],
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "answer", "schema": { "type": "object" } }
            }
        }))
        .unwrap();

        let error = ExchangeRequest::from_client(ClientRequest::OpenAiChat(request)).unwrap_err();
        assert!(error.to_string().contains("response_format.type=text"));
    }

    #[test]
    fn anthropic_response_renders_openai_chat_shape() {
        let response = json!({
            "id": "msg_1",
            "content": [{ "type": "text", "text": "hello" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 3, "output_tokens": 2 }
        });

        let rendered = anthropic_response_to_openai(&response, "client-model").unwrap();

        assert_eq!(rendered["object"], "chat.completion");
        assert_eq!(rendered["model"], "client-model");
        assert_eq!(rendered["choices"][0]["message"]["content"], "hello");
        assert_eq!(rendered["usage"]["total_tokens"], 5);
    }

    #[test]
    fn request_fingerprint_is_stable_and_body_sensitive() {
        let request = |content: &str| {
            serde_json::from_value::<OpenAiChatRequest>(json!({
                "model": "chat-model",
                "messages": [{ "role": "user", "content": content }]
            }))
            .unwrap()
        };
        let first = ExchangeRequest::from_client(ClientRequest::OpenAiChat(request("hello")))
            .unwrap()
            .request_fingerprint()
            .unwrap();
        let same = ExchangeRequest::from_client(ClientRequest::OpenAiChat(request("hello")))
            .unwrap()
            .request_fingerprint()
            .unwrap();
        let different = ExchangeRequest::from_client(ClientRequest::OpenAiChat(request("goodbye")))
            .unwrap()
            .request_fingerprint()
            .unwrap();

        assert_eq!(first.len(), 64);
        assert_eq!(first, same);
        assert_ne!(first, different);
    }
}
