use serde_json::Value;

use crate::{error::AppError, types::AnthropicRequest};

pub fn validate_anthropic_to_openai_fidelity(request: &AnthropicRequest) -> Result<(), AppError> {
    let mut issues = Vec::new();

    if let Some(system) = &request.system {
        audit_system(system, &mut issues);
    }

    for (index, message) in request.messages.iter().enumerate() {
        audit_message(message, index, &mut issues);
    }

    audit_extra_fields(request, &mut issues);

    if issues.is_empty() {
        return Ok(());
    }

    Err(AppError::InvalidRequest(format!(
        "strict fidelity refused Anthropic -> OpenAI-compatible conversion: {}. Use fidelity_mode=\"best_effort\" or route this model to an Anthropic-compatible provider.",
        issues.join("; ")
    )))
}

fn audit_system(system: &Value, issues: &mut Vec<String>) {
    if system.is_string() {
        return;
    }

    let Some(blocks) = system.as_array() else {
        issues.push("system must be a string for strict OpenAI-compatible conversion".to_owned());
        return;
    };

    if blocks.len() != 1 {
        issues.push("system content block boundaries cannot be preserved".to_owned());
    }

    for (index, block) in blocks.iter().enumerate() {
        audit_block_keys(
            block,
            &format!("system[{index}]"),
            &["type", "text"],
            issues,
        );
        if block.get("type").and_then(Value::as_str) != Some("text") {
            issues.push(format!(
                "system[{index}] non-text block cannot be preserved"
            ));
        }
    }
}

fn audit_message(message: &Value, index: usize, issues: &mut Vec<String>) {
    let role = message.get("role").and_then(Value::as_str).unwrap_or("");
    let content = message.get("content").unwrap_or(&Value::Null);
    let path = format!("messages[{index}].content");

    if content.is_string() {
        return;
    }

    let Some(blocks) = content.as_array() else {
        issues.push(format!(
            "{path} must be a string or supported content blocks"
        ));
        return;
    };

    match role {
        "assistant" => audit_assistant_blocks(blocks, &path, issues),
        "user" => audit_user_blocks(blocks, &path, issues),
        _ => issues.push(format!(
            "role `{role}` with structured content cannot be preserved"
        )),
    }
}

fn audit_assistant_blocks(blocks: &[Value], path: &str, issues: &mut Vec<String>) {
    for (index, block) in blocks.iter().enumerate() {
        let block_path = format!("{path}[{index}]");
        match block.get("type").and_then(Value::as_str) {
            Some("text") => audit_block_keys(block, &block_path, &["type", "text"], issues),
            Some("tool_use") => {
                audit_block_keys(block, &block_path, &["type", "id", "name", "input"], issues);
            }
            Some(kind) => issues.push(format!("{block_path} `{kind}` block cannot be preserved")),
            None => issues.push(format!("{block_path} block type is missing")),
        }
    }
}

fn audit_user_blocks(blocks: &[Value], path: &str, issues: &mut Vec<String>) {
    for (index, block) in blocks.iter().enumerate() {
        let block_path = format!("{path}[{index}]");
        match block.get("type").and_then(Value::as_str) {
            Some("text") => audit_block_keys(block, &block_path, &["type", "text"], issues),
            Some("tool_result") => {
                audit_block_keys(
                    block,
                    &block_path,
                    &["type", "tool_use_id", "content"],
                    issues,
                );
                if let Some(content) = block.get("content") {
                    audit_tool_result_content(content, &block_path, issues);
                }
            }
            Some(kind) => issues.push(format!("{block_path} `{kind}` block cannot be preserved")),
            None => issues.push(format!("{block_path} block type is missing")),
        }
    }
}

fn audit_tool_result_content(content: &Value, path: &str, issues: &mut Vec<String>) {
    if content.is_string() {
        return;
    }

    let Some(blocks) = content.as_array() else {
        issues.push(format!("{path}.content cannot be converted without loss"));
        return;
    };

    for (index, block) in blocks.iter().enumerate() {
        let block_path = format!("{path}.content[{index}]");
        audit_block_keys(block, &block_path, &["type", "text"], issues);
        if block.get("type").and_then(Value::as_str) != Some("text") {
            issues.push(format!(
                "{block_path} non-text tool result cannot be preserved"
            ));
        }
    }
}

fn audit_extra_fields(request: &AnthropicRequest, issues: &mut Vec<String>) {
    const SUPPORTED: &[&str] = &[
        "temperature",
        "top_p",
        "top_k",
        "presence_penalty",
        "frequency_penalty",
        "seed",
        "stop_sequences",
        "tools",
        "tool_choice",
    ];

    for key in request.extra.keys() {
        if !SUPPORTED.contains(&key.as_str()) {
            issues.push(format!("request field `{key}` cannot be preserved"));
        }
    }

    if let Some(tools) = request.extra.get("tools").and_then(Value::as_array) {
        for (index, tool) in tools.iter().enumerate() {
            audit_block_keys(
                tool,
                &format!("tools[{index}]"),
                &["name", "description", "input_schema", "strict"],
                issues,
            );
        }
    }

    if let Some(tool_choice) = request.extra.get("tool_choice") {
        audit_tool_choice(tool_choice, issues);
    }
}

fn audit_tool_choice(tool_choice: &Value, issues: &mut Vec<String>) {
    let Some(choice) = tool_choice.as_object() else {
        return;
    };
    let allowed = ["type", "name", "disable_parallel_tool_use"];
    for key in choice.keys() {
        if !allowed.contains(&key.as_str()) {
            issues.push(format!("tool_choice field `{key}` cannot be preserved"));
        }
    }
}

fn audit_block_keys(block: &Value, path: &str, allowed: &[&str], issues: &mut Vec<String>) {
    let Some(object) = block.as_object() else {
        issues.push(format!("{path} must be an object"));
        return;
    };

    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            issues.push(format!("{path}.{key} cannot be preserved"));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const STANDARD_MODEL: &str = "deepseek-v4-flash";

    #[test]
    fn rejects_unknown_request_fields() {
        let request: AnthropicRequest = serde_json::from_value(json!({
            "model": STANDARD_MODEL,
            "max_tokens": 128,
            "metadata": { "user_id": "usr_1" },
            "messages": [{
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();

        let err = validate_anthropic_to_openai_fidelity(&request).unwrap_err();

        assert!(err.to_string().contains("request field `metadata`"));
    }

    #[test]
    fn rejects_non_text_tool_result_content_blocks() {
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
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_read",
                        "content": [{
                            "type": "image",
                            "source": { "type": "base64", "media_type": "image/png", "data": "AA==" }
                        }]
                    }]
                }
            ]
        }))
        .unwrap();

        let err = validate_anthropic_to_openai_fidelity(&request).unwrap_err();

        assert!(err.to_string().contains("non-text tool result"));
    }
}
