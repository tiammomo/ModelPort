use std::{cmp::Reverse, collections::BTreeMap};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    error::AppError,
    metrics::{MessageMetricsSnapshot, MetricsSnapshot},
};

use super::{AppState, effective_config, now_millis_string, provider_protocol_value};

const DEFAULT_LOG_PAGE_SIZE: usize = 20;
const MAX_LOG_PAGE_SIZE: usize = 500;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LogsQuery {
    page: Option<usize>,
    page_size: Option<usize>,
    status: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    user_id: Option<String>,
    api_key_id: Option<String>,
    date_from: Option<u64>,
    date_to: Option<u64>,
    search: Option<String>,
    // These fields keep the existing dashboard filters server-side as well.
    username: Option<String>,
    group: Option<String>,
    stream: Option<String>,
}

impl LogsQuery {
    pub(super) fn validate(&self) -> Result<(), AppError> {
        for (name, value) in [
            ("status", self.status.as_deref()),
            ("provider", self.provider.as_deref()),
            ("model", self.model.as_deref()),
            ("userId", self.user_id.as_deref()),
            ("apiKeyId", self.api_key_id.as_deref()),
            ("username", self.username.as_deref()),
            ("group", self.group.as_deref()),
            ("stream", self.stream.as_deref()),
        ] {
            if value.is_some_and(|value| value.chars().count() > 256) {
                return Err(AppError::InvalidRequest(format!(
                    "{name} must be at most 256 characters"
                )));
            }
        }
        if self
            .search
            .as_deref()
            .is_some_and(|search| search.chars().count() > 512)
        {
            return Err(AppError::InvalidRequest(
                "search must be at most 512 characters".to_owned(),
            ));
        }
        if self
            .status
            .as_deref()
            .is_some_and(|status| !matches!(status, "success" | "error" | "timeout"))
        {
            return Err(AppError::InvalidRequest(
                "status must be success, error, or timeout".to_owned(),
            ));
        }
        if self
            .stream
            .as_deref()
            .is_some_and(|stream| !matches!(stream, "stream" | "non-stream"))
        {
            return Err(AppError::InvalidRequest(
                "stream must be stream or non-stream".to_owned(),
            ));
        }
        if self
            .date_from
            .zip(self.date_to)
            .is_some_and(|(from, to)| from > to)
        {
            return Err(AppError::InvalidRequest(
                "dateFrom must not be later than dateTo".to_owned(),
            ));
        }
        Ok(())
    }
}

pub(super) fn logs_body(state: &AppState, query: &LogsQuery) -> Value {
    logs_body_from_rows(log_rows(state), query)
}

pub(super) fn log_body(state: &AppState, id: &str) -> Option<Value> {
    log_rows(state)
        .into_iter()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
}

fn log_rows(state: &AppState) -> Vec<Value> {
    let mut logs = state.control.usage_rows();
    if logs.is_empty() {
        logs = fallback_log_rows(state);
    }
    logs
}

fn logs_body_from_rows(mut logs: Vec<Value>, query: &LogsQuery) -> Value {
    logs.retain(|row| log_matches(row, query));
    logs.sort_by_key(|row| Reverse(timestamp_millis(row)));
    let total = logs.len();
    let summary = summarize_logs(&logs);
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query
        .page_size
        .unwrap_or(DEFAULT_LOG_PAGE_SIZE)
        .clamp(1, MAX_LOG_PAGE_SIZE);
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let logs = logs
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    json!({
        "logs": logs,
        "total": total,
        "summary": summary,
    })
}

fn log_matches(row: &Value, query: &LogsQuery) -> bool {
    if query
        .status
        .as_deref()
        .is_some_and(|expected| !field_equals(row, "status", expected))
        || query
            .provider
            .as_deref()
            .is_some_and(|expected| !field_equals(row, "provider", expected))
        || query
            .user_id
            .as_deref()
            .is_some_and(|expected| !field_equals(row, "userId", expected))
        || query
            .api_key_id
            .as_deref()
            .is_some_and(|expected| !field_equals(row, "apiKeyId", expected))
        || query
            .stream
            .as_deref()
            .is_some_and(|expected| !field_equals(row, "stream", expected))
    {
        return false;
    }

    if query
        .model
        .as_deref()
        .is_some_and(|expected| !any_field_contains(row, &["model", "resolvedModel"], expected))
        || query
            .username
            .as_deref()
            .is_some_and(|expected| !field_contains(row, "username", expected))
        || query
            .group
            .as_deref()
            .is_some_and(|expected| !any_field_contains(row, &["group", "apiKeyGroup"], expected))
    {
        return false;
    }

    let timestamp = timestamp_millis(row);
    if query
        .date_from
        .is_some_and(|date_from| timestamp.is_none_or(|value| value < date_from))
        || query
            .date_to
            .is_some_and(|date_to| timestamp.is_none_or(|value| value > date_to))
    {
        return false;
    }

    query.search.as_deref().is_none_or(|search| {
        any_field_contains(
            row,
            &[
                "id",
                "requestId",
                "attemptId",
                "provider",
                "channelId",
                "channelName",
                "model",
                "resolvedModel",
                "userId",
                "username",
                "apiKeyId",
                "apiKeyName",
                "tokenName",
                "group",
                "apiKeyGroup",
                "teamId",
                "teamName",
                "errorMessage",
                "terminalReason",
                "detail",
                "requestPath",
                "clientProtocol",
                "protocol",
            ],
            search,
        )
    })
}

fn field_equals(row: &Value, field: &str, expected: &str) -> bool {
    row.get(field).and_then(Value::as_str) == Some(expected)
}

fn field_contains(row: &Value, field: &str, expected: &str) -> bool {
    let expected = expected.trim().to_lowercase();
    expected.is_empty() || field_contains_normalized(row, field, &expected)
}

fn any_field_contains(row: &Value, fields: &[&str], expected: &str) -> bool {
    let expected = expected.trim().to_lowercase();
    expected.is_empty()
        || fields
            .iter()
            .any(|field| field_contains_normalized(row, field, &expected))
}

fn field_contains_normalized(row: &Value, field: &str, expected: &str) -> bool {
    row.get(field)
        .and_then(Value::as_str)
        .is_some_and(|value| value.to_lowercase().contains(expected))
}

fn timestamp_millis(row: &Value) -> Option<u64> {
    row.get("timestamp").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
    })
}

fn summarize_logs(logs: &[Value]) -> Value {
    let mut success_requests = 0usize;
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;
    let mut total_cache_write_tokens = 0u64;
    let mut total_cache_read_tokens = 0u64;
    let mut total_cost_estimate = 0.0f64;
    let mut first_timestamp = None::<u64>;
    let mut last_timestamp = None::<u64>;

    for log in logs {
        if log.get("status").and_then(Value::as_str) == Some("success") {
            success_requests += 1;
        }
        total_input_tokens = total_input_tokens.saturating_add(field_u64(log, "inputTokens"));
        total_output_tokens = total_output_tokens.saturating_add(field_u64(log, "outputTokens"));
        total_cache_write_tokens =
            total_cache_write_tokens.saturating_add(field_u64(log, "cacheWriteTokens"));
        total_cache_read_tokens =
            total_cache_read_tokens.saturating_add(field_u64(log, "cacheReadTokens"));
        total_cost_estimate += log
            .get("costEstimate")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if let Some(timestamp) = timestamp_millis(log) {
            first_timestamp = Some(first_timestamp.map_or(timestamp, |value| value.min(timestamp)));
            last_timestamp = Some(last_timestamp.map_or(timestamp, |value| value.max(timestamp)));
        }
    }

    let total_tokens = total_input_tokens
        .saturating_add(total_output_tokens)
        .saturating_add(total_cache_write_tokens)
        .saturating_add(total_cache_read_tokens);
    let minutes = match (first_timestamp, last_timestamp) {
        (Some(first), Some(last)) if last > first => ((last - first) as f64 / 60_000.0).max(1.0),
        _ => 1.0,
    };

    json!({
        "totalRequests": logs.len(),
        "successRequests": success_requests,
        "totalInputTokens": total_input_tokens,
        "totalOutputTokens": total_output_tokens,
        "totalCacheWriteTokens": total_cache_write_tokens,
        "totalCacheReadTokens": total_cache_read_tokens,
        "totalTokens": total_tokens,
        "totalCostEstimate": total_cost_estimate,
        "rpm": logs.len() as f64 / minutes,
        "tpm": total_tokens as f64 / minutes,
    })
}

fn field_u64(row: &Value, field: &str) -> u64 {
    row.get(field).and_then(Value::as_u64).unwrap_or(0)
}

pub(super) fn latency_body(state: &AppState) -> Value {
    let usage = state.control.usage_rows();
    if usage.is_empty() {
        latency_body_from_snapshot(&state.metrics.snapshot())
    } else {
        latency_body_from_usage(&usage)
    }
}

fn latency_body_from_snapshot(snapshot: &MetricsSnapshot) -> Value {
    let total_requests = snapshot
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    let total_duration = snapshot
        .messages
        .iter()
        .map(|message| message.duration_ms_total)
        .sum::<u64>();
    let avg = average(total_duration, total_requests);

    json!({
        "p50": avg,
        "p90": avg,
        "p95": avg,
        "p99": avg,
        "avg": avg,
        "max": avg,
        "byModel": {},
        "byProvider": {},
        "sampleCount": 0,
        "percentilesEstimated": true,
    })
}

fn latency_body_from_usage(rows: &[Value]) -> Value {
    let mut all = Vec::with_capacity(rows.len());
    let mut by_model = BTreeMap::<String, Vec<u64>>::new();
    let mut by_provider = BTreeMap::<String, Vec<u64>>::new();

    for row in rows {
        let Some(latency) = row.get("latencyMs").and_then(Value::as_u64) else {
            continue;
        };
        all.push(latency);
        if let Some(model) = row.get("resolvedModel").and_then(Value::as_str) {
            by_model.entry(model.to_owned()).or_default().push(latency);
        }
        if let Some(provider) = row.get("provider").and_then(Value::as_str) {
            by_provider
                .entry(provider.to_owned())
                .or_default()
                .push(latency);
        }
    }

    let overall = latency_stats(&all);
    json!({
        "p50": overall["p50"],
        "p90": overall["p90"],
        "p95": overall["p95"],
        "p99": overall["p99"],
        "avg": overall["avg"],
        "max": overall["max"],
        "byModel": grouped_latency_stats(by_model),
        "byProvider": grouped_latency_stats(by_provider),
        "sampleCount": all.len(),
        "percentilesEstimated": false,
    })
}

fn grouped_latency_stats(groups: BTreeMap<String, Vec<u64>>) -> Value {
    Value::Object(
        groups
            .into_iter()
            .map(|(name, values)| (name, latency_stats(&values)))
            .collect::<Map<String, Value>>(),
    )
}

fn latency_stats(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({
            "p50": 0,
            "p90": 0,
            "p95": 0,
            "p99": 0,
            "avg": 0,
            "max": 0,
            "count": 0,
        });
    }

    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let total = sorted.iter().copied().fold(0u64, u64::saturating_add);
    json!({
        "p50": percentile(&sorted, 50),
        "p90": percentile(&sorted, 90),
        "p95": percentile(&sorted, 95),
        "p99": percentile(&sorted, 99),
        "avg": average(total, sorted.len() as u64),
        "max": sorted.last().copied().unwrap_or(0),
        "count": sorted.len(),
    })
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = percentile
        .saturating_mul(sorted.len())
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[rank]
}

fn fallback_log_rows(state: &AppState) -> Vec<Value> {
    let config = effective_config(state);
    let snapshot = state.metrics.snapshot();
    snapshot
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let protocol = config
                .providers
                .get(&message.provider)
                .map(|provider| provider_protocol_value(provider.protocol))
                .unwrap_or("openai-compat");
            fallback_log_row(message, index, protocol, now_millis_string())
        })
        .collect()
}

fn fallback_log_row(
    message: &MessageMetricsSnapshot,
    index: usize,
    protocol: &str,
    timestamp: String,
) -> Value {
    let requests = message.requests_total.max(1);
    json!({
        "id": format!("log_{}_{}_{}", message.provider, message.model.replace('/', "_"), if message.stream { "stream" } else { "nonstream" }),
        "requestId": null,
        "timestamp": timestamp,
        "userId": "usr_local_admin",
        "username": "local-admin",
        "apiKeyId": null,
        "apiKeyName": "MODELPORT_AUTH_TOKEN",
        "apiKeyGroup": "legacy",
        "tokenName": "MODELPORT_AUTH_TOKEN",
        "group": "legacy",
        "channelId": message.provider,
        "channelName": message.provider,
        "model": message.model,
        "resolvedModel": message.model,
        "provider": message.provider,
        "protocol": protocol,
        "clientProtocol": "anthropic-messages",
        "requestType": if message.failures_total > 0 { "error" } else { "consume" },
        "stream": if message.stream { "stream" } else { "non-stream" },
        "status": if message.failures_total > 0 { "error" } else { "success" },
        "statusCode": if message.failures_total > 0 { 502 } else { 200 },
        "inputTokens": 0,
        "outputTokens": 0,
        "cacheWriteTokens": 0,
        "cacheReadTokens": 0,
        "billedInputTokens": 0,
        "totalTokens": 0,
        "cacheHitRate": 0.0,
        "costEstimate": 0.0,
        "costBreakdown": {
            "inputCost": 0.0,
            "outputCost": 0.0,
            "cacheWriteCost": 0.0,
            "cacheReadCost": 0.0,
            "totalCost": 0.0,
        },
        "latencyMs": average(message.duration_ms_total, requests),
        "firstByteLatencyMs": average(message.duration_ms_total, requests),
        "retryCount": 0,
        "clientIp": null,
        "requestPath": "/v1/messages",
        "billingMode": "metrics-fallback",
        "detail": format!("进程内指标回退日志 · provider={} · model={}", message.provider, message.model),
        "errorMessage": if message.failures_total > 0 { Some(format!("{} failure(s) recorded", message.failures_total)) } else { None },
        "sortIndex": index,
    })
}

fn average(total: u64, count: u64) -> u64 {
    total.checked_div(count).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_body_uses_average_duration_across_messages() {
        let snapshot = MetricsSnapshot {
            uptime_seconds: 1,
            routes: vec![],
            messages: vec![
                test_message("deepseek", "deepseek-v4-flash", 2, 80, false, 0),
                test_message("mimo", "mimo-v2.5-pro", 2, 120, false, 0),
            ],
        };

        let body = latency_body_from_snapshot(&snapshot);

        assert_eq!(body["avg"], 50);
        assert_eq!(body["p95"], 50);
        assert_eq!(body["byModel"], json!({}));
        assert_eq!(body["percentilesEstimated"], true);
    }

    #[test]
    fn latency_body_calculates_percentiles_from_persisted_usage() {
        let rows = vec![
            json!({ "latencyMs": 10, "resolvedModel": "model-a", "provider": "one" }),
            json!({ "latencyMs": 20, "resolvedModel": "model-a", "provider": "one" }),
            json!({ "latencyMs": 30, "resolvedModel": "model-b", "provider": "two" }),
            json!({ "latencyMs": 100, "resolvedModel": "model-b", "provider": "two" }),
        ];

        let body = latency_body_from_usage(&rows);

        assert_eq!(body["p50"], 20);
        assert_eq!(body["p90"], 100);
        assert_eq!(body["avg"], 40);
        assert_eq!(body["max"], 100);
        assert_eq!(body["sampleCount"], 4);
        assert_eq!(body["byModel"]["model-a"]["p95"], 20);
        assert_eq!(body["byProvider"]["two"]["avg"], 65);
        assert_eq!(body["percentilesEstimated"], false);
    }

    #[test]
    fn logs_query_filters_all_supported_dimensions_and_epoch_millis() {
        let rows = vec![
            test_log(
                "log-one",
                "req-one",
                1_000,
                "success",
                "provider-one",
                "model-one",
                "user-one",
                "key-one",
                None,
            ),
            test_log(
                "log-two",
                "req-two",
                2_000,
                "error",
                "provider-two",
                "model-two",
                "user-two",
                "key-two",
                Some("Upstream exploded"),
            ),
        ];
        let query = LogsQuery {
            status: Some("error".to_owned()),
            provider: Some("provider-two".to_owned()),
            model: Some("MODEL-TWO".to_owned()),
            user_id: Some("user-two".to_owned()),
            api_key_id: Some("key-two".to_owned()),
            date_from: Some(2_000),
            date_to: Some(2_000),
            search: Some("UPSTREAM EXPLODED".to_owned()),
            ..LogsQuery::default()
        };

        let body = logs_body_from_rows(rows.clone(), &query);

        assert_eq!(body["total"], 1);
        assert_eq!(body["logs"][0]["id"], "log-two");
        for search in [
            "LOG-TWO",
            "REQ-TWO",
            "provider-two",
            "model-two",
            "user-two",
            "key-two",
            "Upstream exploded",
        ] {
            assert!(log_matches(
                &rows[1],
                &LogsQuery {
                    search: Some(search.to_owned()),
                    ..LogsQuery::default()
                }
            ));
        }
    }

    #[test]
    fn logs_query_summarizes_filtered_rows_before_pagination() {
        let rows = vec![
            test_log(
                "log-one", "req-one", 0, "success", "provider", "model", "user", "key", None,
            ),
            test_log(
                "log-two",
                "req-two",
                60_000,
                "error",
                "provider",
                "model",
                "user",
                "key",
                Some("failed"),
            ),
            test_log(
                "log-three",
                "req-three",
                120_000,
                "success",
                "provider",
                "model",
                "user",
                "key",
                None,
            ),
        ];
        let query = LogsQuery {
            page: Some(2),
            page_size: Some(1),
            ..LogsQuery::default()
        };

        let body = logs_body_from_rows(rows, &query);

        assert_eq!(body["logs"].as_array().unwrap().len(), 1);
        assert_eq!(body["logs"][0]["id"], "log-two");
        assert_eq!(body["total"], 3);
        assert_eq!(body["summary"]["totalRequests"], 3);
        assert_eq!(body["summary"]["successRequests"], 2);
        assert_eq!(body["summary"]["totalTokens"], 30);
        assert_eq!(body["summary"]["totalCostEstimate"], 0.75);
        assert_eq!(body["summary"]["rpm"], 1.5);
        assert_eq!(body["summary"]["tpm"], 15.0);
    }

    #[test]
    fn logs_query_clamps_page_size_to_server_limit() {
        let rows = (0..501)
            .map(|index| {
                test_log(
                    &format!("log-{index}"),
                    &format!("req-{index}"),
                    index,
                    "success",
                    "provider",
                    "model",
                    "user",
                    "key",
                    None,
                )
            })
            .collect();
        let query: LogsQuery = serde_json::from_value(json!({
            "page": 0,
            "pageSize": 999,
        }))
        .unwrap();

        let body = logs_body_from_rows(rows, &query);

        assert_eq!(body["logs"].as_array().unwrap().len(), MAX_LOG_PAGE_SIZE);
        assert_eq!(body["logs"][0]["id"], "log-500");
        assert_eq!(body["total"], 501);
    }

    #[test]
    fn logs_query_rejects_invalid_enums_and_reversed_date_range() {
        for query in [
            LogsQuery {
                status: Some("unknown".to_owned()),
                ..LogsQuery::default()
            },
            LogsQuery {
                stream: Some("sometimes".to_owned()),
                ..LogsQuery::default()
            },
            LogsQuery {
                date_from: Some(2),
                date_to: Some(1),
                ..LogsQuery::default()
            },
            LogsQuery {
                search: Some("x".repeat(513)),
                ..LogsQuery::default()
            },
        ] {
            assert!(matches!(query.validate(), Err(AppError::InvalidRequest(_))));
        }
    }

    #[test]
    fn fallback_log_row_marks_failures_and_sanitizes_model_for_id() {
        let row = fallback_log_row(
            &test_message("openai", "foo/bar", 3, 90, true, 2),
            7,
            "openai-compat",
            "12345".to_owned(),
        );

        assert_eq!(row["id"], "log_openai_foo_bar_stream");
        assert_eq!(row["timestamp"], "12345");
        assert_eq!(row["status"], "error");
        assert_eq!(row["statusCode"], 502);
        assert_eq!(row["latencyMs"], 30);
        assert_eq!(row["errorMessage"], "2 failure(s) recorded");
        assert_eq!(row["sortIndex"], 7);
    }

    #[allow(clippy::too_many_arguments)]
    fn test_log(
        id: &str,
        request_id: &str,
        timestamp: u64,
        status: &str,
        provider: &str,
        model: &str,
        user_id: &str,
        api_key_id: &str,
        error_message: Option<&str>,
    ) -> Value {
        json!({
            "id": id,
            "requestId": request_id,
            "timestamp": timestamp.to_string(),
            "status": status,
            "provider": provider,
            "channelId": provider,
            "model": model,
            "resolvedModel": model,
            "userId": user_id,
            "username": format!("{user_id} name"),
            "apiKeyId": api_key_id,
            "apiKeyName": format!("{api_key_id} name"),
            "group": "test-group",
            "stream": "non-stream",
            "inputTokens": 4,
            "outputTokens": 3,
            "cacheWriteTokens": 2,
            "cacheReadTokens": 1,
            "costEstimate": 0.25,
            "errorMessage": error_message,
        })
    }

    fn test_message(
        provider: &str,
        model: &str,
        requests_total: u64,
        duration_ms_total: u64,
        stream: bool,
        failures_total: u64,
    ) -> MessageMetricsSnapshot {
        MessageMetricsSnapshot {
            provider: provider.to_owned(),
            model: model.to_owned(),
            stream,
            requests_total,
            successes_total: requests_total.saturating_sub(failures_total),
            failures_total,
            duration_ms_total,
            input_tokens_total: 0,
            output_tokens_total: 0,
            cache_write_tokens_total: 0,
            cache_read_tokens_total: 0,
            cost_estimate_usd_total: 0.0,
        }
    }
}
