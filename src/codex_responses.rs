use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{domain::ClientProtocol, error::AppError};

const MAX_CLIENT_METADATA_FIELDS: usize = 32;
const MAX_CLIENT_METADATA_KEY_CHARS: usize = 128;
const MAX_CLIENT_METADATA_VALUE_CHARS: usize = 2_048;
const MAX_PROMPT_CACHE_KEY_CHARS: usize = 512;

pub(crate) const REQUEST_PATH: &str = "/v1/responses";
pub(crate) const CLIENT_PROTOCOL: ClientProtocol = ClientProtocol::OpenAiResponses;

/// The bounded request emitted by Codex custom providers at the pinned version.
/// This parser contract does not register an HTTP endpoint or normalize into Exchange.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodexResponsesRequest {
    model: String,
    #[serde(default)]
    instructions: String,
    input: Vec<InputItem>,
    #[serde(default)]
    tools: Option<Vec<FunctionTool>>,
    tool_choice: ToolChoice,
    parallel_tool_calls: bool,
    #[serde(default)]
    reasoning: Option<Reasoning>,
    store: bool,
    stream: bool,
    #[serde(default)]
    stream_options: Option<StreamOptions>,
    include: Vec<Include>,
    #[serde(default)]
    service_tier: Option<ServiceTier>,
    #[serde(default)]
    prompt_cache_key: Option<String>,
    #[serde(default)]
    text: Option<TextControl>,
    #[serde(default)]
    client_metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum InputItem {
    Message {
        #[serde(default)]
        id: Option<String>,
        role: MessageRole,
        content: Vec<InputContent>,
    },
    FunctionCall {
        #[serde(default)]
        id: Option<String>,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        #[serde(default)]
        id: Option<String>,
        call_id: String,
        output: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum MessageRole {
    Developer,
    User,
    Assistant,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum InputContent {
    InputText { text: String },
    OutputText { text: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FunctionTool {
    #[serde(rename = "type")]
    kind: FunctionType,
    name: String,
    #[serde(default)]
    description: Option<String>,
    parameters: Value,
    strict: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum FunctionType {
    Function,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ToolChoice {
    Auto,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reasoning {
    #[serde(default)]
    effort: Option<ReasoningEffort>,
    #[serde(default)]
    summary: Option<ReasoningSummary>,
    #[serde(default)]
    context: Option<ReasoningContext>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningSummary {
    Auto,
    Concise,
    Detailed,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningContext {
    Auto,
    CurrentTurn,
    AllTurns,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamOptions {
    reasoning_summary_delivery: ReasoningSummaryDelivery,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReasoningSummaryDelivery {
    SequentialCutoff,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
enum Include {
    #[serde(rename = "reasoning.encrypted_content")]
    ReasoningEncryptedContent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TextControl {
    #[serde(default)]
    verbosity: Option<Verbosity>,
    #[serde(default)]
    format: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Verbosity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ServiceTier {
    Flex,
    Priority,
}

pub(crate) fn parse(body: &[u8]) -> Result<CodexResponsesRequest, AppError> {
    let request: CodexResponsesRequest = serde_json::from_slice(body).map_err(|error| {
        AppError::InvalidRequest(format!("invalid Codex Responses request: {error}"))
    })?;
    request.validate()?;
    Ok(request)
}

impl CodexResponsesRequest {
    fn validate(&self) -> Result<(), AppError> {
        if self.model.trim().is_empty() {
            return invalid("model is required");
        }
        if self.input.is_empty() {
            return invalid("input must not be empty");
        }
        if !self.stream {
            return invalid("Codex Responses requires stream=true");
        }
        if self.store {
            return invalid("Codex Responses requires store=false");
        }
        if !matches!(self.tool_choice, ToolChoice::Auto) {
            return invalid("Codex Responses requires tool_choice=auto");
        }
        if !matches!(
            self.include.as_slice(),
            [Include::ReasoningEncryptedContent]
        ) {
            return invalid("include must contain exactly reasoning.encrypted_content");
        }
        if self.prompt_cache_key.as_ref().is_some_and(|key| {
            key.trim().is_empty() || key.chars().count() > MAX_PROMPT_CACHE_KEY_CHARS
        }) {
            return invalid(format!(
                "prompt_cache_key must contain 1 to {MAX_PROMPT_CACHE_KEY_CHARS} characters"
            ));
        }
        if self.text.as_ref().is_some_and(|text| text.format.is_some()) {
            return invalid("text.format structured output is outside the bounded contract");
        }
        self.validate_metadata()?;
        self.validate_tools()?;
        self.validate_input()
    }

    fn validate_metadata(&self) -> Result<(), AppError> {
        let Some(metadata) = &self.client_metadata else {
            return Ok(());
        };
        if metadata.len() > MAX_CLIENT_METADATA_FIELDS {
            return invalid(format!(
                "client_metadata supports at most {MAX_CLIENT_METADATA_FIELDS} fields"
            ));
        }
        for (key, value) in metadata {
            if key.is_empty() || key.chars().count() > MAX_CLIENT_METADATA_KEY_CHARS {
                return invalid(format!(
                    "client_metadata keys must contain 1 to {MAX_CLIENT_METADATA_KEY_CHARS} characters"
                ));
            }
            if value.chars().count() > MAX_CLIENT_METADATA_VALUE_CHARS {
                return invalid(format!(
                    "client_metadata.{key} exceeds {MAX_CLIENT_METADATA_VALUE_CHARS} characters"
                ));
            }
        }
        Ok(())
    }

    fn validate_tools(&self) -> Result<(), AppError> {
        let mut names = HashSet::new();
        for (index, tool) in self.tools.as_deref().unwrap_or_default().iter().enumerate() {
            if tool.name.trim().is_empty() {
                return invalid(format!("tools[{index}].name must not be empty"));
            }
            if !names.insert(tool.name.as_str()) {
                return invalid(format!("tools[{index}].name `{}` is duplicated", tool.name));
            }
            if !tool.parameters.is_object() {
                return invalid(format!("tools[{index}].parameters must be an object"));
            }
        }
        Ok(())
    }

    fn validate_input(&self) -> Result<(), AppError> {
        for (index, item) in self.input.iter().enumerate() {
            match item {
                InputItem::Message { role, content, .. } => {
                    if content.is_empty() {
                        return invalid(format!("input[{index}].content must not be empty"));
                    }
                    let valid = content.iter().all(|part| {
                        matches!(
                            (role, part),
                            (
                                MessageRole::Developer | MessageRole::User,
                                InputContent::InputText { .. }
                            ) | (MessageRole::Assistant, InputContent::OutputText { .. })
                        )
                    });
                    if !valid {
                        return invalid(format!(
                            "input[{index}] content type is not valid for its role"
                        ));
                    }
                }
                InputItem::FunctionCall { call_id, name, .. }
                    if call_id.trim().is_empty() || name.trim().is_empty() =>
                {
                    return invalid(format!(
                        "input[{index}] function_call requires non-empty call_id and name"
                    ));
                }
                InputItem::FunctionCallOutput { call_id, .. } if call_id.trim().is_empty() => {
                    return invalid(format!(
                        "input[{index}] function_call_output requires a non-empty call_id"
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, AppError> {
    Err(AppError::InvalidRequest(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &[u8] =
        include_bytes!("../fixtures/codex/codex-responses-request-v0.148.0-alpha.21.json");

    fn fixture_value() -> Value {
        serde_json::from_slice(FIXTURE).unwrap()
    }

    #[test]
    fn pinned_fixture_covers_the_builder_and_parses() {
        parse(FIXTURE).unwrap();
        assert_eq!(CLIENT_PROTOCOL.as_str(), "openai-responses");
        assert_eq!(REQUEST_PATH, "/v1/responses");

        let actual = fixture_value()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let expected = [
            "model",
            "instructions",
            "input",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "reasoning",
            "store",
            "stream",
            "stream_options",
            "include",
            "service_tier",
            "prompt_cache_key",
            "text",
            "client_metadata",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn accepts_observed_omissions_and_item_ids() {
        let mut value = fixture_value();
        for field in [
            "instructions",
            "tools",
            "reasoning",
            "stream_options",
            "service_tier",
            "prompt_cache_key",
            "text",
            "client_metadata",
        ] {
            value.as_object_mut().unwrap().remove(field);
        }
        value["input"][0].as_object_mut().unwrap().remove("id");
        parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[test]
    fn invariants_and_hint_bounds_fail_closed() {
        for (field, replacement, expected) in [
            ("stream", json!(false), "stream=true"),
            ("store", json!(true), "store=false"),
            ("tool_choice", json!("required"), "unknown variant"),
            ("include", json!([]), "exactly reasoning.encrypted_content"),
            ("prompt_cache_key", json!("x".repeat(513)), "1 to 512"),
            (
                "client_metadata",
                json!({"key": "x".repeat(2049)}),
                "exceeds 2048",
            ),
        ] {
            let mut value = fixture_value();
            value[field] = replacement;
            let error = parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
            assert!(error.to_string().contains(expected), "{field}: {error}");
        }
    }

    #[test]
    fn rejects_unknown_multimodal_hosted_and_structured_shapes() {
        let cases = [
            ("unknown", json!({"unsupported": true})),
            (
                "multimodal",
                json!([{"type":"input_image","image_url":"fixture"}]),
            ),
            ("hosted", json!([{"type":"web_search_preview"}])),
            (
                "structured",
                json!({"verbosity":"medium","format":{"type":"json_schema"}}),
            ),
            ("metadata type", json!({"fixture": 1})),
        ];
        for (case, replacement) in cases {
            let mut value = fixture_value();
            match case {
                "unknown" => value["unsupported"] = json!(true),
                "multimodal" => value["input"][0]["content"] = replacement,
                "hosted" => value["tools"] = replacement,
                "structured" => value["text"] = replacement,
                "metadata type" => value["client_metadata"] = replacement,
                _ => unreachable!(),
            }
            assert!(
                parse(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{case}"
            );
        }
    }

    #[test]
    fn fixture_is_content_free_and_explicitly_synthetic() {
        let fixture = std::str::from_utf8(FIXTURE).unwrap();
        for forbidden in ["sk-", "Bearer ", "Authorization", "user@example", "/home/"] {
            assert!(!fixture.contains(forbidden));
        }
        for pointer in [
            "/model",
            "/instructions",
            "/input/0/content/0/text",
            "/input/1/arguments",
            "/input/2/output",
            "/tools/0/description",
            "/prompt_cache_key",
            "/client_metadata/session_id",
        ] {
            assert!(
                fixture_value()
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains("fixture")),
                "{pointer}"
            );
        }
    }
}
