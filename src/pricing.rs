use axum::http::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;

pub const USAGE_HEADER: &str = "x-modelport-usage";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageBreakdown {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCharge {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_estimate: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_write_per_million: f64,
    pub cache_read_per_million: f64,
}

pub fn charge_for_model(model: &str, usage: TokenUsageBreakdown) -> UsageCharge {
    UsageCharge {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_write_tokens: usage.cache_write_tokens,
        cache_read_tokens: usage.cache_read_tokens,
        cost_estimate: cost_for_model(model, usage),
    }
}

pub fn cost_for_model(model: &str, usage: TokenUsageBreakdown) -> f64 {
    let pricing = pricing_for_model(model);
    cost_component(usage.input_tokens, pricing.input_per_million)
        + cost_component(usage.output_tokens, pricing.output_per_million)
        + cost_component(usage.cache_write_tokens, pricing.cache_write_per_million)
        + cost_component(usage.cache_read_tokens, pricing.cache_read_per_million)
}

pub fn usage_header_value(
    model: &str,
    usage: TokenUsageBreakdown,
) -> Result<HeaderValue, AppError> {
    let charge = charge_for_model(model, usage);
    HeaderValue::from_str(&serde_json::to_string(&charge)?)
        .map_err(|err| AppError::Config(format!("invalid usage header: {err}")))
}

pub fn usage_from_headers(headers: &HeaderMap) -> Option<UsageCharge> {
    headers
        .get(USAGE_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| serde_json::from_str(value).ok())
}

pub fn openai_usage(response: &Value) -> TokenUsageBreakdown {
    let Some(usage) = response.get("usage") else {
        return TokenUsageBreakdown::default();
    };

    let prompt_tokens = get_u64(usage, &["prompt_tokens", "input_tokens"]);
    let output_tokens = get_u64(usage, &["completion_tokens", "output_tokens"]);
    let deepseek_cache_hit = get_u64(usage, &["prompt_cache_hit_tokens"]);
    let deepseek_cache_miss = get_u64(usage, &["prompt_cache_miss_tokens"]);
    let cached_tokens = get_nested_u64(
        usage,
        &[
            &["prompt_tokens_details", "cached_tokens"],
            &["input_tokens_details", "cached_tokens"],
        ],
    );
    let cache_read_tokens = deepseek_cache_hit.max(cached_tokens);
    let input_tokens = if deepseek_cache_miss > 0 {
        deepseek_cache_miss
    } else {
        prompt_tokens.saturating_sub(cache_read_tokens)
    };

    TokenUsageBreakdown {
        input_tokens,
        output_tokens,
        cache_write_tokens: get_u64(
            usage,
            &["cache_creation_input_tokens", "prompt_cache_write_tokens"],
        ),
        cache_read_tokens,
    }
}

pub fn openai_usage_if_present(response: &Value) -> Option<TokenUsageBreakdown> {
    let usage = response.get("usage")?.as_object()?;
    let has_supported_field = [
        "prompt_tokens",
        "input_tokens",
        "completion_tokens",
        "output_tokens",
        "prompt_cache_hit_tokens",
        "prompt_cache_miss_tokens",
        "cache_creation_input_tokens",
        "prompt_cache_write_tokens",
        "prompt_tokens_details",
        "input_tokens_details",
    ]
    .iter()
    .any(|field| usage.contains_key(*field));
    has_supported_field.then(|| openai_usage(response))
}

pub fn anthropic_usage(response: &Value) -> TokenUsageBreakdown {
    let Some(usage) = response.get("usage") else {
        return TokenUsageBreakdown::default();
    };

    TokenUsageBreakdown {
        input_tokens: get_u64(usage, &["input_tokens"]),
        output_tokens: get_u64(usage, &["output_tokens"]),
        cache_write_tokens: get_u64(usage, &["cache_creation_input_tokens"]),
        cache_read_tokens: get_u64(usage, &["cache_read_input_tokens"]),
    }
}

pub fn anthropic_usage_if_present(response: &Value) -> Option<TokenUsageBreakdown> {
    let usage = response.get("usage")?.as_object()?;
    let has_supported_field = [
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .iter()
    .any(|field| usage.contains_key(*field));
    has_supported_field.then(|| anthropic_usage(response))
}

pub fn pricing_for_model(model: &str) -> ModelPricing {
    let normalized = model.to_ascii_lowercase();
    if normalized.contains("deepseek-v4-pro") {
        return ModelPricing {
            input_per_million: 0.435,
            output_per_million: 0.87,
            cache_write_per_million: 0.435,
            cache_read_per_million: 0.003625,
        };
    }
    if normalized.contains("deepseek-v4-flash")
        || normalized.contains("deepseek-chat")
        || normalized.contains("deepseek-reasoner")
    {
        return ModelPricing {
            input_per_million: 0.14,
            output_per_million: 0.28,
            cache_write_per_million: 0.14,
            cache_read_per_million: 0.0028,
        };
    }
    if normalized.contains("claude-fable-5") || normalized.contains("claude-mythos-5") {
        return ModelPricing {
            input_per_million: 10.0,
            output_per_million: 50.0,
            cache_write_per_million: 12.5,
            cache_read_per_million: 1.0,
        };
    }
    if normalized.contains("claude-opus-4") {
        return ModelPricing {
            input_per_million: 5.0,
            output_per_million: 25.0,
            cache_write_per_million: 6.25,
            cache_read_per_million: 0.5,
        };
    }
    if normalized.contains("claude-sonnet-4") {
        return ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_write_per_million: 3.75,
            cache_read_per_million: 0.3,
        };
    }
    if normalized.contains("claude-3-5-haiku") || normalized.contains("claude-haiku") {
        return ModelPricing {
            input_per_million: 0.8,
            output_per_million: 4.0,
            cache_write_per_million: 1.0,
            cache_read_per_million: 0.08,
        };
    }
    if normalized.contains("gpt-5.5-pro") || normalized.contains("gpt-5.4-pro") {
        return ModelPricing {
            input_per_million: 15.0,
            output_per_million: 90.0,
            cache_write_per_million: 15.0,
            cache_read_per_million: 15.0,
        };
    }
    if normalized.contains("gpt-5.5") {
        return ModelPricing {
            input_per_million: 2.5,
            output_per_million: 15.0,
            cache_write_per_million: 2.5,
            cache_read_per_million: 0.25,
        };
    }
    if normalized.contains("gpt-5.4-mini") {
        return ModelPricing {
            input_per_million: 0.375,
            output_per_million: 2.25,
            cache_write_per_million: 0.375,
            cache_read_per_million: 0.0375,
        };
    }
    if normalized.contains("gpt-5.4-nano") {
        return ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.625,
            cache_write_per_million: 0.10,
            cache_read_per_million: 0.01,
        };
    }
    if normalized.contains("gpt-5.4") {
        return ModelPricing {
            input_per_million: 1.25,
            output_per_million: 7.5,
            cache_write_per_million: 1.25,
            cache_read_per_million: 0.13,
        };
    }
    if normalized.contains("mimo-") {
        return ModelPricing {
            input_per_million: 0.14,
            output_per_million: 0.28,
            cache_write_per_million: 0.0,
            cache_read_per_million: 0.0028,
        };
    }
    if normalized.contains("gpt-4o") {
        return ModelPricing {
            input_per_million: 2.5,
            output_per_million: 10.0,
            cache_write_per_million: 2.5,
            cache_read_per_million: 1.25,
        };
    }
    if normalized.contains("gpt-") || normalized.contains("openai/") {
        return ModelPricing {
            input_per_million: 1.25,
            output_per_million: 7.5,
            cache_write_per_million: 1.25,
            cache_read_per_million: 0.125,
        };
    }
    if normalized.contains("gemini-") {
        return ModelPricing {
            input_per_million: 1.25,
            output_per_million: 10.0,
            cache_write_per_million: 1.25,
            cache_read_per_million: 0.125,
        };
    }
    if normalized.contains("qwen-") || normalized.contains("kimi-") || normalized.contains("glm-") {
        return ModelPricing {
            input_per_million: 0.6,
            output_per_million: 2.4,
            cache_write_per_million: 0.6,
            cache_read_per_million: 0.06,
        };
    }

    ModelPricing {
        input_per_million: 1.0,
        output_per_million: 4.0,
        cache_write_per_million: 1.0,
        cache_read_per_million: 0.1,
    }
}

pub fn cost_component(tokens: u64, price_per_million: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * price_per_million
}

fn get_u64(value: &Value, fields: &[&str]) -> u64 {
    fields
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn get_nested_u64(value: &Value, paths: &[&[&str]]) -> u64 {
    for path in paths {
        let mut current = value;
        for segment in *path {
            let Some(next) = current.get(*segment) else {
                current = &Value::Null;
                break;
            };
            current = next;
        }
        if let Some(result) = current.as_u64() {
            return result;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn deepseek_cache_hit_tokens_are_discounted() {
        let usage = openai_usage(&json!({
            "usage": {
                "prompt_cache_hit_tokens": 1_000_000_u64,
                "prompt_cache_miss_tokens": 1_000_000_u64,
                "completion_tokens": 1_000_000_u64
            }
        }));
        let charge = charge_for_model("deepseek-v4-flash", usage);

        assert_eq!(charge.input_tokens, 1_000_000);
        assert_eq!(charge.cache_read_tokens, 1_000_000);
        assert!((charge.cost_estimate - 0.4228).abs() < 0.000001);
    }

    #[test]
    fn anthropic_cache_write_and_read_are_separate() {
        let usage = anthropic_usage(&json!({
            "usage": {
                "input_tokens": 1_000_000_u64,
                "cache_creation_input_tokens": 1_000_000_u64,
                "cache_read_input_tokens": 1_000_000_u64,
                "output_tokens": 1_000_000_u64
            }
        }));
        let charge = charge_for_model("claude-sonnet-4-20250514", usage);

        assert!((charge.cost_estimate - 22.05).abs() < 0.000001);
    }

    #[test]
    fn usage_metadata_must_contain_supported_token_fields() {
        assert!(openai_usage_if_present(&json!({ "id": "response" })).is_none());
        assert!(openai_usage_if_present(&json!({ "usage": {} })).is_none());
        assert!(anthropic_usage_if_present(&json!({ "usage": null })).is_none());
        assert!(
            openai_usage_if_present(&json!({
                "usage": { "prompt_tokens": 0, "completion_tokens": 0 }
            }))
            .is_some()
        );
    }
}
