use std::collections::{HashMap, HashSet};

use jsonschema::Validator;
use serde_json::{Map, Value};

use crate::{
    config::{ToolResponseValidation, ToolUseConfig},
    error::AppError,
    types::AnthropicRequest,
};

#[derive(Debug, Clone)]
pub struct ToolResponsePolicy {
    validation: ToolResponseValidation,
    allowed_names: Option<HashSet<String>>,
    input_validators: HashMap<String, Validator>,
    minimum_calls: usize,
    maximum_calls: Option<usize>,
}

impl ToolResponsePolicy {
    pub fn for_anthropic_request(
        request: &AnthropicRequest,
        tool_use: &ToolUseConfig,
    ) -> Result<Self, AppError> {
        let tools = request.extra.get("tools").and_then(Value::as_array);
        let mut allowed_names = Some(
            tools
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                        .map(ToOwned::to_owned)
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default(),
        );
        let mut input_validators = HashMap::new();
        if tool_use.response_validation == ToolResponseValidation::Strict {
            for (index, tool) in tools.into_iter().flatten().enumerate() {
                let name = tool.get("name").and_then(Value::as_str).ok_or_else(|| {
                    AppError::InvalidRequest(format!("tools[{index}].name is required"))
                })?;
                let schema = tool.get("input_schema").ok_or_else(|| {
                    AppError::InvalidRequest(format!(
                        "tools[{index}].input_schema must be an object"
                    ))
                })?;
                input_validators.insert(
                    name.to_owned(),
                    compile_tool_schema(schema, &format!("tools[{index}].input_schema"))?,
                );
            }
        }
        let mut minimum_calls = 0;
        let mut maximum_calls = (!tool_use.parallel_tool_calls).then_some(1);

        if let Some(tool_choice) = request.extra.get("tool_choice") {
            match tool_choice.get("type").and_then(Value::as_str) {
                Some("none") => maximum_calls = Some(0),
                Some("any") => minimum_calls = 1,
                Some("tool") => {
                    minimum_calls = 1;
                    if let Some(name) = tool_choice.get("name").and_then(Value::as_str) {
                        allowed_names = Some(HashSet::from([name.to_owned()]));
                    }
                }
                _ => {}
            }
            if tool_choice
                .get("disable_parallel_tool_use")
                .and_then(Value::as_bool)
                == Some(true)
            {
                maximum_calls = Some(1);
            }
        }

        Ok(Self {
            validation: tool_use.response_validation,
            allowed_names,
            input_validators,
            minimum_calls,
            maximum_calls,
        })
    }

    #[cfg(test)]
    pub fn best_effort() -> Self {
        Self {
            validation: ToolResponseValidation::BestEffort,
            allowed_names: None,
            input_validators: HashMap::new(),
            minimum_calls: 0,
            maximum_calls: None,
        }
    }

    pub fn is_strict(&self) -> bool {
        self.validation == ToolResponseValidation::Strict
    }

    pub fn validate_name(&self, name: Option<&str>) -> Result<String, AppError> {
        if self.validation == ToolResponseValidation::BestEffort {
            return Ok(name.unwrap_or("tool").to_owned());
        }

        let name = name.filter(|name| !name.trim().is_empty()).ok_or_else(|| {
            AppError::UpstreamProtocol(
                "OpenAI-compatible tool call is missing a function name".to_owned(),
            )
        })?;
        if let Some(allowed_names) = &self.allowed_names
            && !allowed_names.contains(name)
        {
            return Err(AppError::UpstreamProtocol(format!(
                "OpenAI-compatible upstream returned undeclared tool `{name}`"
            )));
        }
        Ok(name.to_owned())
    }

    pub fn parse_arguments(&self, name: &str, arguments: &str) -> Result<Value, AppError> {
        let value = if arguments.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            match serde_json::from_str::<Value>(arguments) {
                Ok(value @ Value::Object(_)) => value,
                Ok(value) if self.validation == ToolResponseValidation::BestEffort => {
                    Value::Object(Map::from_iter([("_raw_arguments".to_owned(), value)]))
                }
                Err(_) if self.validation == ToolResponseValidation::BestEffort => {
                    Value::Object(Map::from_iter([(
                        "_raw_arguments".to_owned(),
                        Value::String(arguments.to_owned()),
                    )]))
                }
                Ok(_) => {
                    return Err(AppError::UpstreamProtocol(
                        "OpenAI-compatible tool arguments must be a JSON object".to_owned(),
                    ));
                }
                Err(error) => {
                    return Err(AppError::UpstreamProtocol(format!(
                        "OpenAI-compatible tool arguments are invalid JSON: {error}"
                    )));
                }
            }
        };

        if self.validation == ToolResponseValidation::Strict {
            let validator = self.input_validators.get(name).ok_or_else(|| {
                AppError::UpstreamProtocol(
                    "OpenAI-compatible tool call has no compiled input schema".to_owned(),
                )
            })?;
            if let Err(error) = validator.validate(&value) {
                return Err(AppError::UpstreamProtocol(format!(
                    "OpenAI-compatible tool arguments do not satisfy the declared input schema at {} (schema path {}; value [redacted])",
                    error.instance_path(),
                    error.schema_path()
                )));
            }
        }

        Ok(value)
    }

    pub fn validate_call_summary(
        &self,
        call_count: usize,
        finish_reason: Option<&str>,
    ) -> Result<(), AppError> {
        if self.validation == ToolResponseValidation::BestEffort {
            return Ok(());
        }
        if call_count < self.minimum_calls {
            return Err(AppError::UpstreamProtocol(format!(
                "OpenAI-compatible upstream returned {call_count} tool calls, but tool_choice requires at least {}",
                self.minimum_calls
            )));
        }
        if self
            .maximum_calls
            .is_some_and(|maximum| call_count > maximum)
        {
            return Err(AppError::UpstreamProtocol(format!(
                "OpenAI-compatible upstream returned {call_count} tool calls, exceeding the allowed maximum of {}",
                self.maximum_calls.unwrap_or_default()
            )));
        }

        let stopped_for_tools = matches!(finish_reason, Some("tool_calls" | "function_call"));
        if call_count > 0 && !stopped_for_tools {
            return Err(AppError::UpstreamProtocol(
                "OpenAI-compatible upstream returned tool calls without a tool_calls finish reason"
                    .to_owned(),
            ));
        }
        if call_count == 0 && stopped_for_tools {
            return Err(AppError::UpstreamProtocol(
                "OpenAI-compatible upstream returned a tool_calls finish reason without a tool call"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

pub fn validate_anthropic_tooling(request: &AnthropicRequest) -> Result<(), AppError> {
    let tool_names = if let Some(tools) = request.extra.get("tools") {
        Some(validate_tool_definitions(tools)?)
    } else {
        None
    };

    if let Some(tool_choice) = request.extra.get("tool_choice") {
        validate_tool_choice_shape(tool_choice, tool_names.as_ref())?;
    }

    for (index, message) in request.messages.iter().enumerate() {
        validate_message_tool_blocks(message, index)?;
    }
    validate_tool_turn_references(request, tool_names.as_ref())?;

    Ok(())
}

pub fn validate_anthropic_tool_capabilities(
    request: &AnthropicRequest,
    provider_id: &str,
    tool_use: &ToolUseConfig,
) -> Result<(), AppError> {
    if !request_uses_tools(request) {
        return Ok(());
    }

    if !tool_use.supported {
        return Err(AppError::InvalidRequest(format!(
            "provider `{provider_id}` does not support tool use"
        )));
    }

    if request.extra.contains_key("tool_choice") && !tool_use.tool_choice {
        return Err(AppError::InvalidRequest(format!(
            "provider `{provider_id}` does not support tool_choice"
        )));
    }

    if !tool_use.parallel_tool_calls
        && request
            .extra
            .get("tool_choice")
            .and_then(|value| value.get("disable_parallel_tool_use"))
            .and_then(Value::as_bool)
            == Some(false)
    {
        return Err(AppError::InvalidRequest(format!(
            "provider `{provider_id}` does not support parallel tool calls"
        )));
    }

    Ok(())
}

fn validate_tool_definitions(tools: &Value) -> Result<HashSet<String>, AppError> {
    let tools = tools
        .as_array()
        .ok_or_else(|| AppError::InvalidRequest("tools must be an array".to_owned()))?;
    let mut names = HashSet::new();

    for (index, tool) in tools.iter().enumerate() {
        let path = format!("tools[{index}]");
        let object = tool
            .as_object()
            .ok_or_else(|| AppError::InvalidRequest(format!("{path} must be an object")))?;

        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::InvalidRequest(format!("{path}.name is required")))?;
        validate_tool_name(name, &format!("{path}.name"))?;
        if !names.insert(name.to_owned()) {
            return Err(AppError::InvalidRequest(format!(
                "{path}.name duplicates another tool"
            )));
        }

        if object
            .get("description")
            .is_some_and(|description| !description.is_string())
        {
            return Err(AppError::InvalidRequest(format!(
                "{path}.description must be a string"
            )));
        }

        let schema = object
            .get("input_schema")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                AppError::InvalidRequest(format!("{path}.input_schema must be an object"))
            })?;
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(AppError::InvalidRequest(format!(
                "{path}.input_schema.type must be object"
            )));
        }
        compile_tool_schema(
            &Value::Object(schema.clone()),
            &format!("{path}.input_schema"),
        )?;
        if object
            .get("strict")
            .is_some_and(|strict| !strict.is_boolean())
        {
            return Err(AppError::InvalidRequest(format!(
                "{path}.strict must be a boolean"
            )));
        }
    }

    Ok(names)
}

fn compile_tool_schema(schema: &Value, path: &str) -> Result<Validator, AppError> {
    reject_external_schema_references(schema, path)?;
    jsonschema::validator_for(schema).map_err(|error| {
        AppError::InvalidRequest(format!("{path} is not a valid JSON Schema: {error}"))
    })
}

fn reject_external_schema_references(value: &Value, path: &str) -> Result<(), AppError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if matches!(key.as_str(), "$ref" | "$dynamicRef")
                    && child
                        .as_str()
                        .is_some_and(|reference| !reference.starts_with('#'))
                {
                    return Err(AppError::InvalidRequest(format!(
                        "{child_path} must be a local JSON Pointer; external schema references are disabled"
                    )));
                }
                reject_external_schema_references(child, &child_path)?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_external_schema_references(child, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn request_uses_tools(request: &AnthropicRequest) -> bool {
    request.extra.contains_key("tools")
        || request.extra.contains_key("tool_choice")
        || request.messages.iter().any(message_has_tool_block)
}

fn message_has_tool_block(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks.iter().any(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("tool_use" | "tool_result")
                )
            })
        })
}

fn validate_tool_choice_shape(
    tool_choice: &Value,
    tool_names: Option<&HashSet<String>>,
) -> Result<(), AppError> {
    let object = tool_choice
        .as_object()
        .ok_or_else(|| AppError::InvalidRequest("tool_choice must be an object".to_owned()))?;
    let choice_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::InvalidRequest("tool_choice.type is required".to_owned()))?;

    if !matches!(choice_type, "auto" | "any" | "none" | "tool") {
        return Err(AppError::InvalidRequest(
            "tool_choice.type must be auto, any, none, or tool".to_owned(),
        ));
    }
    if choice_type == "tool" {
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::InvalidRequest("tool_choice.name is required".to_owned()))?;
        validate_tool_name(name, "tool_choice.name")?;
        validate_tool_name_is_defined(name, tool_names, "tool_choice.name")?;
    } else if let Some(name) = object.get("name") {
        let Some(name) = name.as_str() else {
            return Err(AppError::InvalidRequest(
                "tool_choice.name must be a string".to_owned(),
            ));
        };
        validate_tool_name(name, "tool_choice.name")?;
        validate_tool_name_is_defined(name, tool_names, "tool_choice.name")?;
    }
    if matches!(choice_type, "any" | "tool") && tool_names.is_none_or(HashSet::is_empty) {
        return Err(AppError::InvalidRequest(format!(
            "tool_choice.type={choice_type} requires at least one tool"
        )));
    }

    if object
        .get("disable_parallel_tool_use")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(AppError::InvalidRequest(
            "tool_choice.disable_parallel_tool_use must be a boolean".to_owned(),
        ));
    }

    Ok(())
}

fn validate_tool_name_is_defined(
    name: &str,
    tool_names: Option<&HashSet<String>>,
    path: &str,
) -> Result<(), AppError> {
    if let Some(tool_names) = tool_names
        && !tool_names.contains(name)
    {
        return Err(AppError::InvalidRequest(format!(
            "{path} `{name}` must match a defined tool"
        )));
    }

    Ok(())
}

fn validate_tool_turn_references(
    request: &AnthropicRequest,
    tool_names: Option<&HashSet<String>>,
) -> Result<(), AppError> {
    let mut seen_tool_use_ids = HashSet::new();
    let mut pending_tool_use_ids = HashSet::new();

    for (message_index, message) in request.messages.iter().enumerate() {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let Some(blocks) = message.get("content").and_then(Value::as_array) else {
            if !pending_tool_use_ids.is_empty() {
                return Err(AppError::InvalidRequest(format!(
                    "messages[{message_index}] must immediately return all pending tool_result blocks"
                )));
            }
            continue;
        };

        let resolving_pending = !pending_tool_use_ids.is_empty();
        if resolving_pending && role != "user" {
            return Err(AppError::InvalidRequest(format!(
                "messages[{message_index}] must be a user message immediately returning pending tool results"
            )));
        }

        for (block_index, block) in blocks.iter().enumerate() {
            let path = format!("messages[{message_index}].content[{block_index}]");
            let Some(object) = block.as_object() else {
                continue;
            };

            match object.get("type").and_then(Value::as_str) {
                Some("tool_use") if role == "assistant" => {
                    let id = object.get("id").and_then(Value::as_str).unwrap_or("");
                    if !seen_tool_use_ids.insert(id.to_owned()) {
                        return Err(AppError::InvalidRequest(format!(
                            "{path}.id duplicates a previous tool_use id"
                        )));
                    }
                    pending_tool_use_ids.insert(id.to_owned());

                    let name = object.get("name").and_then(Value::as_str).unwrap_or("");
                    validate_tool_name_is_defined(name, tool_names, &format!("{path}.name"))?;
                }
                Some("tool_result") if role == "user" => {
                    let tool_use_id = object
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !seen_tool_use_ids.contains(tool_use_id) {
                        return Err(AppError::InvalidRequest(format!(
                            "{path}.tool_use_id `{tool_use_id}` does not match a previous tool_use id"
                        )));
                    }
                    if !pending_tool_use_ids.remove(tool_use_id) {
                        return Err(AppError::InvalidRequest(format!(
                            "{path}.tool_use_id `{tool_use_id}` has already been answered"
                        )));
                    }
                }
                _ => {}
            }
        }

        if resolving_pending && !pending_tool_use_ids.is_empty() {
            return Err(AppError::InvalidRequest(format!(
                "messages[{message_index}] must return every pending tool_result in one user message"
            )));
        }
    }

    if !pending_tool_use_ids.is_empty() {
        return Err(AppError::InvalidRequest(
            "the final assistant tool_use must be followed by a user tool_result message"
                .to_owned(),
        ));
    }

    Ok(())
}

fn validate_message_tool_blocks(message: &Value, message_index: usize) -> Result<(), AppError> {
    let Some(object) = message.as_object() else {
        return Ok(());
    };
    let role = object.get("role").and_then(Value::as_str).unwrap_or("");
    let Some(blocks) = object.get("content").and_then(Value::as_array) else {
        return Ok(());
    };

    let mut saw_non_tool_result = false;
    for (block_index, block) in blocks.iter().enumerate() {
        let path = format!("messages[{message_index}].content[{block_index}]");
        let object = block
            .as_object()
            .ok_or_else(|| AppError::InvalidRequest(format!("{path} must be an object")))?;
        let block_type = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::InvalidRequest(format!("{path}.type is required")))?;

        match block_type {
            "text" => {
                let Some(text) = object.get("text") else {
                    return Err(AppError::InvalidRequest(format!("{path}.text is required")));
                };
                if !text.is_string() {
                    return Err(AppError::InvalidRequest(format!(
                        "{path}.text must be a string"
                    )));
                }
            }
            "tool_use" => {
                saw_non_tool_result = true;
                validate_tool_use_block(role, object, &path)?;
            }
            "tool_result" => {
                if saw_non_tool_result {
                    return Err(AppError::InvalidRequest(format!(
                        "{path} tool_result blocks must come before other content"
                    )));
                }
                validate_tool_result_block(role, object, &path)?;
            }
            _ => {}
        }
        if block_type != "tool_result" {
            saw_non_tool_result = true;
        }
    }

    Ok(())
}

fn validate_tool_use_block(
    role: &str,
    block: &Map<String, Value>,
    path: &str,
) -> Result<(), AppError> {
    if role != "assistant" {
        return Err(AppError::InvalidRequest(format!(
            "{path} tool_use blocks are only valid in assistant messages"
        )));
    }

    let id = block
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::InvalidRequest(format!("{path}.id is required for tool_use")))?;
    if id.trim().is_empty() {
        return Err(AppError::InvalidRequest(format!(
            "{path}.id must not be empty for tool_use"
        )));
    }

    let name = block
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::InvalidRequest(format!("{path}.name is required")))?;
    validate_tool_name(name, &format!("{path}.name"))?;

    if !block.get("input").is_some_and(Value::is_object) {
        return Err(AppError::InvalidRequest(format!(
            "{path}.input must be an object"
        )));
    }

    Ok(())
}

fn validate_tool_result_block(
    role: &str,
    block: &Map<String, Value>,
    path: &str,
) -> Result<(), AppError> {
    if role != "user" {
        return Err(AppError::InvalidRequest(format!(
            "{path} tool_result blocks are only valid in user messages"
        )));
    }

    let tool_use_id = block
        .get("tool_use_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::InvalidRequest(format!("{path}.tool_use_id is required")))?;
    if tool_use_id.trim().is_empty() {
        return Err(AppError::InvalidRequest(format!(
            "{path}.tool_use_id must not be empty"
        )));
    }

    if let Some(content) = block.get("content") {
        if !content.is_string() && !content.is_array() {
            return Err(AppError::InvalidRequest(format!(
                "{path}.content must be a string or array"
            )));
        }

        if let Some(blocks) = content.as_array() {
            for (index, block) in blocks.iter().enumerate() {
                if !block.is_object() {
                    return Err(AppError::InvalidRequest(format!(
                        "{path}.content[{index}] must be an object"
                    )));
                }
            }
        }
    }

    if block
        .get("is_error")
        .is_some_and(|is_error| !is_error.is_boolean())
    {
        return Err(AppError::InvalidRequest(format!(
            "{path}.is_error must be a boolean"
        )));
    }

    Ok(())
}

fn validate_tool_name(name: &str, path: &str) -> Result<(), AppError> {
    let len = name.chars().count();
    if len == 0 || len > 64 {
        return Err(AppError::InvalidRequest(format!(
            "{path} must be 1-64 characters"
        )));
    }

    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err(AppError::InvalidRequest(format!(
            "{path} may only contain letters, numbers, '_' and '-'"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn strict_policy(schema: Value) -> ToolResponsePolicy {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "test",
            "tools": [{"name": "lookup", "input_schema": schema}],
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .unwrap();
        ToolResponsePolicy::for_anthropic_request(
            &request,
            &ToolUseConfig {
                response_validation: ToolResponseValidation::Strict,
                ..ToolUseConfig::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn skips_capability_gate_when_request_does_not_use_tools() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 128,
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();
        let tool_use = ToolUseConfig {
            supported: false,
            ..ToolUseConfig::default()
        };

        validate_anthropic_tool_capabilities(&request, "no_tools", &tool_use).unwrap();
    }

    #[test]
    fn rejects_parallel_tool_calls_when_provider_disallows_them() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 128,
            "tools": [{
                "name": "read_file",
                "input_schema": { "type": "object" }
            }],
            "tool_choice": {
                "type": "auto",
                "disable_parallel_tool_use": false
            },
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();
        let tool_use = ToolUseConfig {
            supported: true,
            tool_choice: true,
            parallel_tool_calls: false,
            ..ToolUseConfig::default()
        };

        let err =
            validate_anthropic_tool_capabilities(&request, "single_tool", &tool_use).unwrap_err();

        assert!(
            err.to_string()
                .contains("does not support parallel tool calls")
        );
    }

    #[test]
    fn rejects_tool_definition_without_required_object_schema() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "test",
            "tools": [{"name": "read_file"}],
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .unwrap();

        let error = validate_anthropic_tooling(&request).unwrap_err();

        assert!(error.to_string().contains("input_schema must be an object"));
    }

    #[test]
    fn strict_response_validation_accepts_nested_schema_conformant_arguments() {
        let policy = strict_policy(json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1},
                "filters": {
                    "type": "object",
                    "properties": {"limit": {"type": "integer", "minimum": 1}},
                    "required": ["limit"],
                    "additionalProperties": false
                }
            },
            "required": ["query", "filters"],
            "additionalProperties": false
        }));

        let value = policy
            .parse_arguments("lookup", r#"{"query":"rust","filters":{"limit":3}}"#)
            .unwrap();

        assert_eq!(value["filters"]["limit"], 3);
    }

    #[test]
    fn strict_response_validation_rejects_missing_required_and_wrong_types() {
        let policy = strict_policy(json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "mode": {"enum": ["fast", "deep"]}
            },
            "required": ["query", "mode"],
            "additionalProperties": false
        }));

        for arguments in [
            r#"{"query":"rust"}"#,
            r#"{"query":7,"mode":"fast"}"#,
            r#"{"query":"rust","mode":"unknown"}"#,
            r#"{"query":"rust","mode":"fast","extra":true}"#,
        ] {
            let error = policy.parse_arguments("lookup", arguments).unwrap_err();
            assert!(error.to_string().contains("declared input schema"));
        }
    }

    #[test]
    fn strict_response_validation_reports_location_without_leaking_values() {
        let policy = strict_policy(json!({
            "type": "object",
            "properties": {"token": {"type": "integer"}},
            "required": ["token"],
            "additionalProperties": false
        }));

        let error = policy
            .parse_arguments("lookup", r#"{"token":"secret-value"}"#)
            .unwrap_err()
            .to_string();

        assert!(error.contains("/token"));
        assert!(error.contains("[redacted]"));
        assert!(!error.contains("secret-value"));
    }

    #[test]
    fn rejects_invalid_or_external_tool_schemas_before_routing() {
        for schema in [
            json!({"type": "object", "properties": {"x": {"type": "not-a-type"}}}),
            json!({"type": "object", "$ref": "https://example.com/schema.json"}),
        ] {
            let request: AnthropicRequest = serde_json::from_value(json!({
                "model": "test",
                "tools": [{"name": "lookup", "input_schema": schema}],
                "messages": [{"role": "user", "content": "hello"}]
            }))
            .unwrap();

            assert!(validate_anthropic_tooling(&request).is_err());
        }
    }

    #[test]
    fn rejects_text_before_tool_result() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "test",
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_read",
                        "name": "read_file",
                        "input": {}
                    }]
                },
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "result follows"},
                        {"type": "tool_result", "tool_use_id": "toolu_read", "content": "ok"}
                    ]
                }
            ]
        }))
        .unwrap();

        let error = validate_anthropic_tooling(&request).unwrap_err();

        assert!(error.to_string().contains("must come before other content"));
    }

    #[test]
    fn rejects_intervening_message_before_tool_result() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": "test",
            "messages": [
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_read",
                        "name": "read_file",
                        "input": {}
                    }]
                },
                {"role": "user", "content": "wait"},
                {
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": "toolu_read", "content": "ok"}]
                }
            ]
        }))
        .unwrap();

        let error = validate_anthropic_tooling(&request).unwrap_err();

        assert!(error.to_string().contains("must immediately return"));
    }
}
