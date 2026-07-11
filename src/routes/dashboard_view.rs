use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    control::ProviderUsageStats,
    error::AppError,
    metrics::{MessageMetricsSnapshot, MetricsSnapshot},
};

use super::{AppState, effective_config, now_millis, now_millis_string, provider_rows};

const HOUR_MS: u64 = 60 * 60 * 1_000;
const DAY_MS: u64 = 24 * HOUR_MS;
const MAX_DASHBOARD_TREND_MS: u64 = 90 * DAY_MS;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct DashboardQuery {
    range: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Clone)]
struct DashboardTrendWindow {
    range: String,
    start_ms: u64,
    end_ms: u64,
    bucket_ms: u64,
}

#[derive(Debug, Default)]
struct ModelRangeUsage {
    model: String,
    provider: String,
    requests: u64,
    tokens: u64,
    cost: f64,
}

#[derive(Debug)]
struct DashboardRangeUsage {
    has_any_persisted_usage: bool,
    matched_requests: u64,
    request_time_series: Vec<Value>,
    error_time_series: Vec<Value>,
    token_time_series: Vec<Value>,
    model_usage: Vec<Value>,
    summary: Value,
}

pub(super) fn dashboard_body(state: &AppState, query: &DashboardQuery) -> Result<Value, AppError> {
    let trend_window = dashboard_trend_window(query)?;
    let snapshot = state.metrics.snapshot();
    let total_requests = snapshot
        .messages
        .iter()
        .map(|message| message.requests_total)
        .sum::<u64>();
    let total_successes = snapshot
        .messages
        .iter()
        .map(|message| message.successes_total)
        .sum::<u64>();
    let total_failures = snapshot
        .messages
        .iter()
        .map(|message| message.failures_total)
        .sum::<u64>();
    let total_duration = snapshot
        .messages
        .iter()
        .map(|message| message.duration_ms_total)
        .sum::<u64>();
    let route_requests = snapshot
        .routes
        .iter()
        .map(|route| route.requests_total)
        .sum::<u64>();
    let route_successes = snapshot
        .routes
        .iter()
        .map(|route| route.successes_total)
        .sum::<u64>();
    let route_failures = snapshot
        .routes
        .iter()
        .map(|route| route.failures_total)
        .sum::<u64>();
    let route_duration = snapshot
        .routes
        .iter()
        .map(|route| route.duration_ms_total)
        .sum::<u64>();
    let busiest_route = snapshot
        .routes
        .iter()
        .max_by_key(|route| route.requests_total)
        .map(|route| route.route.as_str())
        .unwrap_or("none");

    let providers = provider_rows(state);
    let active_providers = providers
        .iter()
        .filter(|provider| provider.get("status").and_then(Value::as_str) == Some("active"))
        .count();
    let active_users = state.auth.active_user_count();
    let usage_summary = state.control.usage_summary_today();
    let usage_rows = state.control.usage_rows();
    let range_usage = dashboard_range_usage(&usage_rows, &trend_window);
    let now = now_millis();
    let process_start = now.saturating_sub(snapshot.uptime_seconds.saturating_mul(1_000));
    let metrics_cover_window = !range_usage.has_any_persisted_usage
        && trend_window.start_ms <= process_start
        && trend_window.end_ms >= now.saturating_sub(5_000);
    let metric_model_usage_rows = metric_model_usage(&snapshot);
    let mut metric_top_models = metric_model_usage_rows
        .iter()
        .map(|row| {
            json!({
                "model": row.get("model").cloned().unwrap_or(Value::Null),
                "provider": row.get("provider").cloned().unwrap_or(Value::Null),
                "requests": row.get("requests").cloned().unwrap_or_else(|| json!(0)),
            })
        })
        .collect::<Vec<_>>();
    sort_and_limit_top_models(&mut metric_top_models);
    let mut persisted_top_models = range_usage
        .model_usage
        .iter()
        .map(|row| {
            json!({
                "model": row.get("model").cloned().unwrap_or(Value::Null),
                "provider": row.get("provider").cloned().unwrap_or(Value::Null),
                "requests": row.get("requests").cloned().unwrap_or_else(|| json!(0)),
            })
        })
        .collect::<Vec<_>>();
    sort_and_limit_top_models(&mut persisted_top_models);
    let persisted_provider_usage = state.control.provider_usage_today();
    let recent_activity = state.control.activity_rows(8);
    let config = effective_config(state);

    Ok(json!({
        "uptimeSeconds": snapshot.uptime_seconds,
        "totalRequests": total_requests,
        "successRate": percent(total_successes, total_requests),
        "activeProviders": active_providers,
        "totalProviders": providers.len(),
        "activeUsers": active_users,
        "totalModels": config.model_list().len(),
        "avgLatencyMs": average(total_duration, total_requests),
        "apiKeysTotal": usage_summary.api_keys_total,
        "apiKeysActive": usage_summary.api_keys_active,
        "todayRequests": usage_summary.total_requests,
        "todayInputTokens": usage_summary.total_input_tokens,
        "todayOutputTokens": usage_summary.total_output_tokens,
        "todayCacheWriteTokens": usage_summary.total_cache_write_tokens,
        "todayCacheReadTokens": usage_summary.total_cache_read_tokens,
        "todayCostEstimate": usage_summary.total_cost_estimate,
        "trendRange": {
            "range": trend_window.range,
            "from": trend_window.start_ms.to_string(),
            "to": trend_window.end_ms.to_string(),
            "bucketMs": trend_window.bucket_ms,
        },
        "requestTimeSeries": if metrics_cover_window { time_series(total_requests, &trend_window) } else { range_usage.request_time_series },
        "errorTimeSeries": if metrics_cover_window { time_series(total_failures, &trend_window) } else { range_usage.error_time_series },
        "topModels": if metrics_cover_window { metric_top_models } else { persisted_top_models },
        "modelUsage": if metrics_cover_window { metric_model_usage_rows } else { range_usage.model_usage },
        "tokenTimeSeries": if metrics_cover_window { metric_token_time_series(&snapshot, &trend_window) } else { range_usage.token_time_series },
        "rangeSummary": if metrics_cover_window { metric_range_summary(&snapshot, total_requests, total_successes) } else { range_usage.summary },
        "rangeDataSource": if metrics_cover_window { "process-metrics-estimate" } else if range_usage.matched_requests > 0 { "persisted-usage" } else { "empty" },
        "rangeDataEstimated": metrics_cover_window,
        "rangeDataAtRetentionLimit": state.control.usage_retention_at_capacity(),
        "providerHealth": provider_health_rows(&providers, &snapshot, &persisted_provider_usage),
        "recentActivity": if recent_activity.is_empty() {
            fallback_activity(total_requests, total_failures, route_requests, route_successes, route_failures, route_duration, busiest_route)
        } else {
            recent_activity
        },
    }))
}

fn sort_and_limit_top_models(rows: &mut Vec<Value>) {
    rows.sort_by(|left, right| {
        right
            .get("requests")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .cmp(&left.get("requests").and_then(Value::as_u64).unwrap_or(0))
    });
    rows.truncate(8);
}

fn fallback_activity(
    total_requests: u64,
    total_failures: u64,
    route_requests: u64,
    route_successes: u64,
    route_failures: u64,
    route_duration: u64,
    busiest_route: &str,
) -> Vec<Value> {
    let now = now_millis_string();
    vec![
        json!({
            "id": "act_health",
            "timestamp": now.clone(),
            "type": "request",
            "message": format!("ModelPort gateway is healthy; busiest route: {busiest_route}"),
            "severity": "info",
        }),
        json!({
            "id": "act_messages",
            "timestamp": now_millis_string(),
            "type": if total_failures > 0 { "error" } else { "request" },
            "message": format!("{total_requests} model message request(s), {total_failures} failure(s) since startup"),
            "severity": if total_failures > 0 { "warning" } else { "info" },
        }),
        json!({
            "id": "act_routes",
            "timestamp": now_millis_string(),
            "type": if route_failures > 0 { "error" } else { "request" },
            "message": format!("{route_requests} route request(s), {route_successes} success(es), avg {} ms", average(route_duration, route_requests)),
            "severity": if route_failures > 0 { "warning" } else { "info" },
        }),
    ]
}

fn provider_health_rows(
    providers: &[Value],
    snapshot: &MetricsSnapshot,
    persisted_provider_usage: &BTreeMap<String, ProviderUsageStats>,
) -> Vec<Value> {
    providers
        .iter()
        .map(|provider| {
            let id = provider.get("id").and_then(Value::as_str).unwrap_or("");
            let provider_messages = snapshot
                .messages
                .iter()
                .filter(|message| message.provider == id)
                .collect::<Vec<_>>();
            let usage = provider_dashboard_usage(&provider_messages, persisted_provider_usage.get(id));
            let success_rate = percent(usage.successes, usage.requests);
            let provider_status = provider
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("inactive");
            let runtime_status = provider
                .get("runtimeStatus")
                .and_then(Value::as_str)
                .unwrap_or("healthy");
            let provider_health = provider.get("health").unwrap_or(&Value::Null);
            let health_status = if provider_status != "active" {
                "down"
            } else if runtime_status == "cooldown" {
                "cooldown"
            } else if runtime_status == "degraded" || (usage.requests > 0 && success_rate < 99.0) {
                "degraded"
            } else {
                "healthy"
            };
            json!({
                "providerId": id,
                "displayName": provider.get("displayName").cloned().unwrap_or_else(|| json!(id)),
                "status": health_status,
                "requestsTotal": usage.requests,
                "successRate": success_rate,
                "avgLatencyMs": average(usage.duration_ms, usage.requests),
                "inputTokensTotal": usage.input_tokens,
                "outputTokensTotal": usage.output_tokens,
                "cacheWriteTokensTotal": usage.cache_write_tokens,
                "cacheReadTokensTotal": usage.cache_read_tokens,
                "costEstimateUsdTotal": usage.cost_estimate,
                "accountIssue": provider_health.get("accountIssue").cloned().unwrap_or_else(|| json!("none")),
                "rechargeRequired": provider_health.get("rechargeRequired").and_then(Value::as_bool).unwrap_or(false),
                "rechargeBadge": provider_health.get("rechargeBadge").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
struct DashboardProviderUsage {
    requests: u64,
    successes: u64,
    duration_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_write_tokens: u64,
    cache_read_tokens: u64,
    cost_estimate: f64,
}

fn provider_dashboard_usage(
    metric_messages: &[&MessageMetricsSnapshot],
    persisted_usage: Option<&ProviderUsageStats>,
) -> DashboardProviderUsage {
    if let Some(stats) = persisted_usage
        && stats.requests_total > 0
    {
        return DashboardProviderUsage {
            requests: stats.requests_total,
            successes: stats.successes_total,
            duration_ms: stats.duration_ms_total,
            input_tokens: stats.input_tokens_total,
            output_tokens: stats.output_tokens_total,
            cache_write_tokens: stats.cache_write_tokens_total,
            cache_read_tokens: stats.cache_read_tokens_total,
            cost_estimate: stats.cost_estimate_usd_total,
        };
    }

    DashboardProviderUsage {
        requests: metric_messages
            .iter()
            .map(|message| message.requests_total)
            .sum(),
        successes: metric_messages
            .iter()
            .map(|message| message.successes_total)
            .sum(),
        duration_ms: metric_messages
            .iter()
            .map(|message| message.duration_ms_total)
            .sum(),
        input_tokens: metric_messages
            .iter()
            .map(|message| message.input_tokens_total)
            .sum(),
        output_tokens: metric_messages
            .iter()
            .map(|message| message.output_tokens_total)
            .sum(),
        cache_write_tokens: metric_messages
            .iter()
            .map(|message| message.cache_write_tokens_total)
            .sum(),
        cache_read_tokens: metric_messages
            .iter()
            .map(|message| message.cache_read_tokens_total)
            .sum(),
        cost_estimate: metric_messages
            .iter()
            .map(|message| message.cost_estimate_usd_total)
            .sum(),
    }
}

fn dashboard_range_usage(rows: &[Value], window: &DashboardTrendWindow) -> DashboardRangeUsage {
    let bucket_count = usize::try_from(bucket_count(
        window.start_ms,
        window.end_ms,
        window.bucket_ms,
    ))
    .unwrap_or(1)
    .max(1);
    let mut requests = vec![0u64; bucket_count];
    let mut errors = vec![0u64; bucket_count];
    let mut input_tokens = vec![0u64; bucket_count];
    let mut output_tokens = vec![0u64; bucket_count];
    let mut cache_write_tokens = vec![0u64; bucket_count];
    let mut cache_read_tokens = vec![0u64; bucket_count];
    let mut models = BTreeMap::<(String, String), ModelRangeUsage>::new();
    let mut matched_requests = 0u64;
    let mut success_requests = 0u64;
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;
    let mut total_cache_write_tokens = 0u64;
    let mut total_cache_read_tokens = 0u64;
    let mut total_cost_estimate = 0.0f64;

    for row in rows {
        let Some(timestamp) = dashboard_usage_timestamp(row) else {
            continue;
        };
        if timestamp < window.start_ms || timestamp > window.end_ms {
            continue;
        }

        let index =
            usize::try_from(timestamp.saturating_sub(window.start_ms) / window.bucket_ms.max(1))
                .unwrap_or(bucket_count.saturating_sub(1))
                .min(bucket_count.saturating_sub(1));
        let row_input = dashboard_usage_u64(row, "inputTokens");
        let row_output = dashboard_usage_u64(row, "outputTokens");
        let row_cache_write = dashboard_usage_u64(row, "cacheWriteTokens");
        let row_cache_read = dashboard_usage_u64(row, "cacheReadTokens");
        let row_tokens = row_input
            .saturating_add(row_output)
            .saturating_add(row_cache_write)
            .saturating_add(row_cache_read);
        let row_cost = row
            .get("costEstimate")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);

        matched_requests = matched_requests.saturating_add(1);
        requests[index] = requests[index].saturating_add(1);
        if row.get("status").and_then(Value::as_str) == Some("success") {
            success_requests = success_requests.saturating_add(1);
        } else {
            errors[index] = errors[index].saturating_add(1);
        }
        input_tokens[index] = input_tokens[index].saturating_add(row_input);
        output_tokens[index] = output_tokens[index].saturating_add(row_output);
        cache_write_tokens[index] = cache_write_tokens[index].saturating_add(row_cache_write);
        cache_read_tokens[index] = cache_read_tokens[index].saturating_add(row_cache_read);
        total_input_tokens = total_input_tokens.saturating_add(row_input);
        total_output_tokens = total_output_tokens.saturating_add(row_output);
        total_cache_write_tokens = total_cache_write_tokens.saturating_add(row_cache_write);
        total_cache_read_tokens = total_cache_read_tokens.saturating_add(row_cache_read);
        total_cost_estimate += row_cost;

        let model = row
            .get("resolvedModel")
            .or_else(|| row.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let provider = row
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let model_usage = models
            .entry((model.clone(), provider.clone()))
            .or_insert_with(|| ModelRangeUsage {
                model,
                provider,
                ..ModelRangeUsage::default()
            });
        model_usage.requests = model_usage.requests.saturating_add(1);
        model_usage.tokens = model_usage.tokens.saturating_add(row_tokens);
        model_usage.cost += row_cost;
    }

    let request_time_series = dashboard_value_series(&requests, window);
    let error_time_series = dashboard_value_series(&errors, window);
    let token_time_series = (0..bucket_count)
        .map(|index| {
            let billed_input = input_tokens[index]
                .saturating_add(cache_write_tokens[index])
                .saturating_add(cache_read_tokens[index]);
            json!({
                "timestamp": dashboard_bucket_timestamp(window, index),
                "inputTokens": input_tokens[index],
                "outputTokens": output_tokens[index],
                "cacheWriteTokens": cache_write_tokens[index],
                "cacheReadTokens": cache_read_tokens[index],
                "cacheHitRate": if billed_input == 0 { 0.0 } else { (cache_read_tokens[index] as f64 / billed_input as f64) * 100.0 },
            })
        })
        .collect();
    let mut model_usage = models.into_values().collect::<Vec<_>>();
    model_usage.sort_by(|left, right| {
        right
            .tokens
            .cmp(&left.tokens)
            .then_with(|| right.requests.cmp(&left.requests))
            .then_with(|| left.model.cmp(&right.model))
    });
    let model_usage = model_usage
        .into_iter()
        .map(|row| {
            json!({
                "model": row.model,
                "provider": row.provider,
                "requests": row.requests,
                "tokens": row.tokens,
                "cost": row.cost,
            })
        })
        .collect();
    let total_tokens = total_input_tokens
        .saturating_add(total_output_tokens)
        .saturating_add(total_cache_write_tokens)
        .saturating_add(total_cache_read_tokens);
    let minutes = (window.end_ms.saturating_sub(window.start_ms) as f64 / 60_000.0).max(1.0);

    DashboardRangeUsage {
        has_any_persisted_usage: !rows.is_empty(),
        matched_requests,
        request_time_series,
        error_time_series,
        token_time_series,
        model_usage,
        summary: json!({
            "totalRequests": matched_requests,
            "successRequests": success_requests,
            "totalInputTokens": total_input_tokens,
            "totalOutputTokens": total_output_tokens,
            "totalCacheWriteTokens": total_cache_write_tokens,
            "totalCacheReadTokens": total_cache_read_tokens,
            "totalTokens": total_tokens,
            "totalCostEstimate": total_cost_estimate,
            "rpm": matched_requests as f64 / minutes,
            "tpm": total_tokens as f64 / minutes,
        }),
    }
}

fn metric_model_usage(snapshot: &MetricsSnapshot) -> Vec<Value> {
    let mut models = BTreeMap::<(String, String), ModelRangeUsage>::new();
    for message in &snapshot.messages {
        let row = models
            .entry((message.model.clone(), message.provider.clone()))
            .or_insert_with(|| ModelRangeUsage {
                model: message.model.clone(),
                provider: message.provider.clone(),
                ..ModelRangeUsage::default()
            });
        row.requests = row.requests.saturating_add(message.requests_total);
        row.tokens = row
            .tokens
            .saturating_add(message.input_tokens_total)
            .saturating_add(message.output_tokens_total)
            .saturating_add(message.cache_write_tokens_total)
            .saturating_add(message.cache_read_tokens_total);
        row.cost += message.cost_estimate_usd_total;
    }
    let mut rows = models.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .tokens
            .cmp(&left.tokens)
            .then_with(|| right.requests.cmp(&left.requests))
    });
    rows.into_iter()
        .map(|row| {
            json!({
                "model": row.model,
                "provider": row.provider,
                "requests": row.requests,
                "tokens": row.tokens,
                "cost": row.cost,
            })
        })
        .collect()
}

fn metric_token_time_series(
    snapshot: &MetricsSnapshot,
    window: &DashboardTrendWindow,
) -> Vec<Value> {
    let bucket_count = usize::try_from(bucket_count(
        window.start_ms,
        window.end_ms,
        window.bucket_ms,
    ))
    .unwrap_or(1)
    .max(1);
    let input = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.input_tokens_total)
    });
    let output = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.output_tokens_total)
    });
    let cache_write = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.cache_write_tokens_total)
    });
    let cache_read = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.cache_read_tokens_total)
    });
    let billed_input = input.saturating_add(cache_write).saturating_add(cache_read);

    (0..bucket_count)
        .map(|index| {
            let latest = index + 1 == bucket_count;
            json!({
                "timestamp": dashboard_bucket_timestamp(window, index),
                "inputTokens": if latest { input } else { 0 },
                "outputTokens": if latest { output } else { 0 },
                "cacheWriteTokens": if latest { cache_write } else { 0 },
                "cacheReadTokens": if latest { cache_read } else { 0 },
                "cacheHitRate": if latest && billed_input > 0 { (cache_read as f64 / billed_input as f64) * 100.0 } else { 0.0 },
            })
        })
        .collect()
}

fn metric_range_summary(
    snapshot: &MetricsSnapshot,
    total_requests: u64,
    total_successes: u64,
) -> Value {
    let total_input_tokens = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.input_tokens_total)
    });
    let total_output_tokens = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.output_tokens_total)
    });
    let total_cache_write_tokens = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.cache_write_tokens_total)
    });
    let total_cache_read_tokens = snapshot.messages.iter().fold(0u64, |total, row| {
        total.saturating_add(row.cache_read_tokens_total)
    });
    let total_tokens = total_input_tokens
        .saturating_add(total_output_tokens)
        .saturating_add(total_cache_write_tokens)
        .saturating_add(total_cache_read_tokens);
    let total_cost_estimate = snapshot
        .messages
        .iter()
        .map(|row| row.cost_estimate_usd_total)
        .sum::<f64>();
    let minutes = (snapshot.uptime_seconds as f64 / 60.0).max(1.0);

    json!({
        "totalRequests": total_requests,
        "successRequests": total_successes,
        "totalInputTokens": total_input_tokens,
        "totalOutputTokens": total_output_tokens,
        "totalCacheWriteTokens": total_cache_write_tokens,
        "totalCacheReadTokens": total_cache_read_tokens,
        "totalTokens": total_tokens,
        "totalCostEstimate": total_cost_estimate,
        "rpm": total_requests as f64 / minutes,
        "tpm": total_tokens as f64 / minutes,
    })
}

fn dashboard_usage_timestamp(row: &Value) -> Option<u64> {
    row.get("timestamp").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
    })
}

fn dashboard_usage_u64(row: &Value, field: &str) -> u64 {
    row.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn dashboard_bucket_timestamp(window: &DashboardTrendWindow, index: usize) -> String {
    window
        .start_ms
        .saturating_add(
            u64::try_from(index)
                .unwrap_or(u64::MAX)
                .saturating_mul(window.bucket_ms),
        )
        .to_string()
}

fn dashboard_value_series(values: &[u64], window: &DashboardTrendWindow) -> Vec<Value> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            json!({
                "timestamp": dashboard_bucket_timestamp(window, index),
                "value": value,
            })
        })
        .collect()
}

fn dashboard_trend_window(query: &DashboardQuery) -> Result<DashboardTrendWindow, AppError> {
    let now = now_millis();
    let range = query.range.as_deref().unwrap_or("1d");
    let (range, start_ms, end_ms) = match range {
        "custom" => {
            let start_ms = query
                .from
                .as_deref()
                .and_then(parse_dashboard_time)
                .ok_or_else(|| {
                    AppError::InvalidRequest("custom dashboard range requires from".to_owned())
                })?;
            let end_ms = query
                .to
                .as_deref()
                .and_then(parse_dashboard_time)
                .ok_or_else(|| {
                    AppError::InvalidRequest("custom dashboard range requires to".to_owned())
                })?;
            if start_ms >= end_ms {
                return Err(AppError::InvalidRequest(
                    "custom dashboard range requires from before to".to_owned(),
                ));
            }
            ("custom".to_owned(), start_ms, end_ms.min(now))
        }
        "3d" => ("3d".to_owned(), now.saturating_sub(3 * DAY_MS), now),
        "7d" => ("7d".to_owned(), now.saturating_sub(7 * DAY_MS), now),
        _ => ("1d".to_owned(), now.saturating_sub(DAY_MS), now),
    };
    let duration_ms = end_ms.saturating_sub(start_ms).max(HOUR_MS);
    if duration_ms > MAX_DASHBOARD_TREND_MS {
        return Err(AppError::InvalidRequest(
            "dashboard range cannot exceed 90 days".to_owned(),
        ));
    }

    Ok(DashboardTrendWindow {
        range,
        start_ms,
        end_ms,
        bucket_ms: dashboard_bucket_ms(duration_ms),
    })
}

fn parse_dashboard_time(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

fn dashboard_bucket_ms(duration_ms: u64) -> u64 {
    if duration_ms <= DAY_MS {
        HOUR_MS
    } else if duration_ms <= 3 * DAY_MS {
        3 * HOUR_MS
    } else if duration_ms <= 7 * DAY_MS {
        6 * HOUR_MS
    } else if duration_ms <= 31 * DAY_MS {
        DAY_MS
    } else {
        7 * DAY_MS
    }
}

fn time_series(value: u64, window: &DashboardTrendWindow) -> Vec<Value> {
    let bucket_count = bucket_count(window.start_ms, window.end_ms, window.bucket_ms);
    (0..bucket_count)
        .map(|offset| {
            let timestamp = window
                .start_ms
                .saturating_add(offset.saturating_mul(window.bucket_ms));
            json!({
                "timestamp": timestamp.to_string(),
                "value": if offset + 1 == bucket_count { value } else { 0 },
            })
        })
        .collect()
}

fn bucket_count(start_ms: u64, end_ms: u64, bucket_ms: u64) -> u64 {
    if bucket_ms == 0 || end_ms <= start_ms {
        return 1;
    }
    end_ms.saturating_sub(start_ms) / bucket_ms + 1
}

fn percent(successes: u64, total: u64) -> f64 {
    if total == 0 {
        100.0
    } else {
        (successes as f64 / total as f64) * 100.0
    }
}

fn average(total: u64, count: u64) -> u64 {
    total.checked_div(count).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{MetricsSnapshot, RouteMetricsSnapshot};

    #[test]
    fn time_series_places_fallback_value_in_latest_bucket() {
        let window = DashboardTrendWindow {
            range: "custom".to_owned(),
            start_ms: 1_000,
            end_ms: 4_000,
            bucket_ms: 1_000,
        };

        let series = time_series(42, &window);

        assert_eq!(series.len(), 4);
        assert_eq!(series[0]["value"], 0);
        assert_eq!(series[3]["timestamp"], "4000");
        assert_eq!(series[3]["value"], 42);
    }

    #[test]
    fn provider_health_prefers_persisted_usage_and_keeps_recharge_badge() {
        let providers = vec![json!({
            "id": "deepseek",
            "displayName": "DeepSeek",
            "status": "active",
            "runtimeStatus": "healthy",
            "health": {
                "accountIssue": "insufficient_balance",
                "rechargeRequired": true,
                "rechargeBadge": "等待充值",
            },
        })];
        let snapshot = MetricsSnapshot {
            uptime_seconds: 1,
            routes: vec![RouteMetricsSnapshot {
                route: "messages".to_owned(),
                requests_total: 1,
                successes_total: 1,
                failures_total: 0,
                duration_ms_total: 10,
            }],
            messages: vec![MessageMetricsSnapshot {
                provider: "deepseek".to_owned(),
                model: "deepseek-v4-flash".to_owned(),
                stream: false,
                requests_total: 1,
                successes_total: 1,
                failures_total: 0,
                duration_ms_total: 10,
                input_tokens_total: 1,
                output_tokens_total: 1,
                cache_write_tokens_total: 0,
                cache_read_tokens_total: 0,
                cost_estimate_usd_total: 0.01,
            }],
        };
        let mut persisted = BTreeMap::new();
        persisted.insert(
            "deepseek".to_owned(),
            ProviderUsageStats {
                requests_total: 4,
                successes_total: 3,
                duration_ms_total: 120,
                input_tokens_total: 11,
                output_tokens_total: 22,
                cache_write_tokens_total: 33,
                cache_read_tokens_total: 44,
                cost_estimate_usd_total: 0.5,
            },
        );

        let rows = provider_health_rows(&providers, &snapshot, &persisted);
        let row = &rows[0];

        assert_eq!(row["requestsTotal"], 4);
        assert_eq!(row["successRate"], 75.0);
        assert_eq!(row["avgLatencyMs"], 30);
        assert_eq!(row["status"], "degraded");
        assert_eq!(row["inputTokensTotal"], 11);
        assert_eq!(row["rechargeRequired"], true);
        assert_eq!(row["rechargeBadge"], "等待充值");
    }

    #[test]
    fn range_usage_aggregates_every_matching_persisted_row() {
        let window = DashboardTrendWindow {
            range: "custom".to_owned(),
            start_ms: 1_000,
            end_ms: 4_000,
            bucket_ms: 1_000,
        };
        let rows = vec![
            json!({
                "timestamp": "1500",
                "status": "success",
                "resolvedModel": "model-a",
                "provider": "provider-a",
                "inputTokens": 10,
                "outputTokens": 20,
                "cacheWriteTokens": 2,
                "cacheReadTokens": 3,
                "costEstimate": 0.25,
            }),
            json!({
                "timestamp": "2500",
                "status": "error",
                "resolvedModel": "model-a",
                "provider": "provider-a",
                "inputTokens": 5,
                "outputTokens": 0,
                "cacheWriteTokens": 0,
                "cacheReadTokens": 0,
                "costEstimate": 0.05,
            }),
            json!({
                "timestamp": "9000",
                "status": "success",
                "resolvedModel": "outside",
                "provider": "provider-b",
                "inputTokens": 999,
                "outputTokens": 999,
                "costEstimate": 99.0,
            }),
        ];

        let usage = dashboard_range_usage(&rows, &window);

        assert!(usage.has_any_persisted_usage);
        assert_eq!(usage.matched_requests, 2);
        assert_eq!(usage.request_time_series[0]["value"], 1);
        assert_eq!(usage.request_time_series[1]["value"], 1);
        assert_eq!(usage.error_time_series[1]["value"], 1);
        assert_eq!(usage.model_usage.len(), 1);
        assert_eq!(usage.model_usage[0]["requests"], 2);
        assert_eq!(usage.model_usage[0]["tokens"], 40);
        assert_eq!(usage.summary["totalRequests"], 2);
        assert_eq!(usage.summary["successRequests"], 1);
        assert_eq!(usage.summary["totalTokens"], 40);
        assert_eq!(usage.summary["totalCostEstimate"], 0.3);
    }

    #[test]
    fn empty_historical_window_does_not_look_like_missing_persistence() {
        let window = DashboardTrendWindow {
            range: "custom".to_owned(),
            start_ms: 1_000,
            end_ms: 2_000,
            bucket_ms: 1_000,
        };
        let usage = dashboard_range_usage(
            &[json!({
                "timestamp": "9000",
                "status": "success",
                "resolvedModel": "outside",
                "provider": "provider-a",
            })],
            &window,
        );

        assert!(usage.has_any_persisted_usage);
        assert_eq!(usage.matched_requests, 0);
        assert_eq!(usage.summary["totalRequests"], 0);
    }
}
