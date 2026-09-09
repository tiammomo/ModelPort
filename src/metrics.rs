use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::control::UsageEstimate;

const MAX_MESSAGE_SERIES: usize = 512;
const TRAFFIC_CLASS_COUNT: usize = 3;
const MESSAGE_LATENCY_BUCKETS_MS: [u64; 10] = [
    100, 250, 500, 1_000, 2_500, 5_000, 10_000, 30_000, 60_000, 120_000,
];
const OVERFLOW_PROVIDER_LABEL: &str = "__overflow__";
const OVERFLOW_MODEL_LABEL: &str = "__other__";
const OVERFLOW_TRAFFIC_LABEL: &str = "__overflow__";

#[derive(Debug)]
pub struct Metrics {
    started_at: Instant,
    inner: Mutex<MetricsInner>,
    max_message_series: usize,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub messages: Vec<MessageMetricsSnapshot>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MessageMetricsSnapshot {
    pub provider: String,
    pub model: String,
    pub traffic_class: String,
    pub stream: bool,
    pub requests_total: u64,
    pub successes_total: u64,
    pub failures_total: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct MessageMetricLabels<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub traffic_class: &'a str,
    pub stream: bool,
}

#[derive(Debug, Default)]
struct MetricsInner {
    routes: BTreeMap<String, CounterSet>,
    messages: BTreeMap<MessageKey, MessageCounterSet>,
    rejections: BTreeMap<RejectionKey, u64>,
    ledger_operations: BTreeMap<String, LedgerOperationMetrics>,
    runtime_adapter_collections: BTreeMap<String, RuntimeAdapterCollectionMetrics>,
    routing_decisions: BTreeMap<RoutingDecisionKey, u64>,
    routing_shadow_disagreements_total: u64,
    reconciled_requests_total: u64,
    reconciled_attempts_total: u64,
    message_latency: LatencyHistogram,
}

#[derive(Debug, Default)]
struct LatencyHistogram {
    cumulative_buckets: [u64; MESSAGE_LATENCY_BUCKETS_MS.len()],
    count: u64,
    sum_ms: u64,
}

#[derive(Debug, Default)]
struct CounterSet {
    requests_total: u64,
    successes_total: u64,
    failures_total: u64,
    duration_ms_total: u64,
}

#[derive(Debug, Default)]
struct MessageCounterSet {
    counters: CounterSet,
    usage: UsageCounterSet,
}

#[derive(Debug, Default)]
struct UsageCounterSet {
    input_tokens_total: u64,
    output_tokens_total: u64,
    cache_write_tokens_total: u64,
    cache_read_tokens_total: u64,
    cost_estimate_usd_total: f64,
}

#[derive(Debug, Default)]
struct LedgerOperationMetrics {
    failures_total: u64,
    degraded: bool,
}

#[derive(Debug, Default)]
struct RuntimeAdapterCollectionMetrics {
    successes_total: u64,
    failures_total: BTreeMap<String, u64>,
    last_attempt_timestamp_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MessageKey {
    provider: String,
    model: String,
    traffic_class: String,
    stream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RejectionKey {
    route: String,
    phase: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RoutingDecisionKey {
    mode: String,
    profile: String,
    provider: String,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            inner: Mutex::new(MetricsInner::default()),
            max_message_series: MAX_MESSAGE_SERIES,
        }
    }

    pub fn record_route(&self, route: &str, success: bool, duration: Duration) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        inner
            .routes
            .entry(route.to_owned())
            .or_default()
            .record(success, duration);
    }

    pub fn record_rejection(&self, route: &str, phase: &'static str, reason: &'static str) {
        debug_assert!(matches!(
            phase,
            "authentication"
                | "validation"
                | "identity"
                | "routing"
                | "rate_limit"
                | "admission"
                | "concurrency"
                | "ledger"
                | "draining"
        ));
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        let count = inner
            .rejections
            .entry(RejectionKey {
                route: route.to_owned(),
                phase: phase.to_owned(),
                reason: reason.to_owned(),
            })
            .or_default();
        *count = count.saturating_add(1);
    }

    pub fn record_message(
        &self,
        labels: MessageMetricLabels<'_>,
        success: bool,
        duration: Duration,
        usage: UsageEstimate,
    ) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        inner.message_latency.record(duration);
        let mut key = MessageKey {
            provider: labels.provider.to_owned(),
            model: labels.model.to_owned(),
            traffic_class: labels.traffic_class.to_owned(),
            stream: labels.stream,
        };
        let overflow_slots = self
            .max_message_series
            .saturating_sub(1)
            .min(TRAFFIC_CLASS_COUNT);
        let regular_series_limit = self.max_message_series.saturating_sub(overflow_slots);
        if !inner.messages.contains_key(&key) && inner.messages.len() >= regular_series_limit {
            key.provider = OVERFLOW_PROVIDER_LABEL.to_owned();
            key.model = OVERFLOW_MODEL_LABEL.to_owned();
            if overflow_slots < TRAFFIC_CLASS_COUNT {
                key.traffic_class = OVERFLOW_TRAFFIC_LABEL.to_owned();
            }
            key.stream = false;
        }
        inner
            .messages
            .entry(key)
            .or_default()
            .record(success, duration, usage);
    }

    pub fn record_ledger_operation(&self, operation: &'static str, success: bool) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        let metrics = inner
            .ledger_operations
            .entry(operation.to_owned())
            .or_default();
        metrics.degraded = !success;
        if !success {
            metrics.failures_total = metrics.failures_total.saturating_add(1);
        }
    }

    pub(crate) fn record_runtime_adapter_collection_attempt(&self, adapter_id: &str) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        inner
            .runtime_adapter_collections
            .entry(adapter_id.to_owned())
            .or_default()
            .last_attempt_timestamp_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    pub(crate) fn record_runtime_adapter_collection_result(
        &self,
        adapter_id: &str,
        result: Result<(), &'static str>,
    ) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        let metrics = inner
            .runtime_adapter_collections
            .entry(adapter_id.to_owned())
            .or_default();
        match result {
            Ok(()) => {
                metrics.successes_total = metrics.successes_total.saturating_add(1);
            }
            Err(error) => {
                let failures = metrics.failures_total.entry(error.to_owned()).or_default();
                *failures = failures.saturating_add(1);
            }
        }
    }

    pub fn record_routing_decision(
        &self,
        mode: &str,
        profile: &str,
        provider: &str,
        shadow_disagreement: bool,
    ) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        let count = inner
            .routing_decisions
            .entry(RoutingDecisionKey {
                mode: mode.to_owned(),
                profile: profile.to_owned(),
                provider: provider.to_owned(),
            })
            .or_default();
        *count = count.saturating_add(1);
        if shadow_disagreement {
            inner.routing_shadow_disagreements_total =
                inner.routing_shadow_disagreements_total.saturating_add(1);
        }
    }

    pub fn record_reconciliation(&self, requests: u64, attempts: u64) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        inner.reconciled_requests_total = inner.reconciled_requests_total.saturating_add(requests);
        inner.reconciled_attempts_total = inner.reconciled_attempts_total.saturating_add(attempts);
    }

    pub fn degraded_ledger_operations(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("metrics lock poisoned")
            .ledger_operations
            .iter()
            .filter(|(_, metrics)| metrics.degraded)
            .map(|(operation, _)| operation.clone())
            .collect()
    }

    pub fn render_prometheus(&self) -> String {
        let inner = self.inner.lock().expect("metrics lock poisoned");
        let mut output = String::new();

        output.push_str("# HELP modelport_uptime_seconds Seconds since ModelPort started.\n");
        output.push_str("# TYPE modelport_uptime_seconds gauge\n");
        output.push_str(&format!(
            "modelport_uptime_seconds {}\n\n",
            self.started_at.elapsed().as_secs()
        ));
        output.push_str("# HELP modelport_build_info Static release and source-build identity.\n");
        output.push_str("# TYPE modelport_build_info gauge\n");
        output.push_str(&format!(
            "modelport_build_info{{version=\"{}\",revision=\"{}\",source_state=\"{}\"}} 1\n\n",
            escape_label_value(crate::version::VERSION),
            escape_label_value(crate::version::REVISION),
            escape_label_value(crate::version::SOURCE_STATE),
        ));

        output.push_str(
            "# HELP modelport_route_requests_total Total route requests handled by ModelPort.\n",
        );
        output.push_str("# TYPE modelport_route_requests_total counter\n");
        output.push_str("# HELP modelport_route_successes_total Total successful route requests handled by ModelPort.\n");
        output.push_str("# TYPE modelport_route_successes_total counter\n");
        output.push_str("# HELP modelport_route_failures_total Total failed route requests handled by ModelPort.\n");
        output.push_str("# TYPE modelport_route_failures_total counter\n");
        output.push_str("# HELP modelport_route_duration_ms_total Total route handling duration in milliseconds.\n");
        output.push_str("# TYPE modelport_route_duration_ms_total counter\n");
        for (route, counters) in &inner.routes {
            let labels = format!("route=\"{}\"", escape_label_value(route));
            push_counter_set(&mut output, "modelport_route", &labels, counters);
        }
        output.push('\n');

        output.push_str(
            "# HELP modelport_routing_decisions_total Routing decisions observed by this process, by mode, profile, and selected provider.\n",
        );
        output.push_str("# TYPE modelport_routing_decisions_total counter\n");
        for (key, count) in &inner.routing_decisions {
            output.push_str(&format!(
                "modelport_routing_decisions_total{{mode=\"{}\",profile=\"{}\",provider=\"{}\"}} {count}\n",
                escape_label_value(&key.mode),
                escape_label_value(&key.profile),
                escape_label_value(&key.provider),
            ));
        }
        output.push_str(
            "# HELP modelport_routing_shadow_disagreements_total Decisions where the recommendation differed from the configured baseline selection.\n",
        );
        output.push_str("# TYPE modelport_routing_shadow_disagreements_total counter\n");
        output.push_str(&format!(
            "modelport_routing_shadow_disagreements_total {}\n",
            inner.routing_shadow_disagreements_total
        ));
        output.push('\n');

        output.push_str(
            "# HELP modelport_inference_rejections_total Inference requests rejected before a Provider result, by bounded phase and reason.\n",
        );
        output.push_str("# TYPE modelport_inference_rejections_total counter\n");
        for (key, count) in &inner.rejections {
            let labels = format!(
                "route=\"{}\",phase=\"{}\",reason=\"{}\"",
                escape_label_value(&key.route),
                escape_label_value(&key.phase),
                escape_label_value(&key.reason),
            );
            output.push_str(&format!(
                "modelport_inference_rejections_total{{{labels}}} {count}\n"
            ));
        }
        output.push('\n');

        output.push_str("# HELP modelport_message_requests_total Total message requests by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_requests_total counter\n");
        output.push_str("# HELP modelport_message_successes_total Total successful message requests by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_successes_total counter\n");
        output.push_str("# HELP modelport_message_failures_total Total failed message requests by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_failures_total counter\n");
        output.push_str("# HELP modelport_message_duration_ms_total Total message request lifecycle duration in milliseconds by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_duration_ms_total counter\n");
        output.push_str("# HELP modelport_message_input_tokens_total Total input tokens by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_input_tokens_total counter\n");
        output.push_str("# HELP modelport_message_output_tokens_total Total output tokens by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_output_tokens_total counter\n");
        output.push_str("# HELP modelport_message_cache_write_tokens_total Total cache write tokens by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_cache_write_tokens_total counter\n");
        output.push_str("# HELP modelport_message_cache_read_tokens_total Total cache read tokens by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_cache_read_tokens_total counter\n");
        output.push_str("# HELP modelport_message_cost_estimate_usd_total Total estimated message cost in USD by provider/model/traffic class/stream.\n");
        output.push_str("# TYPE modelport_message_cost_estimate_usd_total counter\n");
        for (key, counters) in &inner.messages {
            let labels = format!(
                "provider=\"{}\",model=\"{}\",traffic_class=\"{}\",stream=\"{}\"",
                escape_label_value(&key.provider),
                escape_label_value(&key.model),
                escape_label_value(&key.traffic_class),
                key.stream
            );
            push_message_counter_set(&mut output, &labels, counters);
        }
        output.push('\n');

        output.push_str(
            "# HELP modelport_message_latency_ms End-to-end inference request latency in milliseconds.\n",
        );
        output.push_str("# TYPE modelport_message_latency_ms histogram\n");
        for (index, upper_bound) in MESSAGE_LATENCY_BUCKETS_MS.iter().enumerate() {
            output.push_str(&format!(
                "modelport_message_latency_ms_bucket{{le=\"{upper_bound}\"}} {}\n",
                inner.message_latency.cumulative_buckets[index]
            ));
        }
        output.push_str(&format!(
            "modelport_message_latency_ms_bucket{{le=\"+Inf\"}} {}\n",
            inner.message_latency.count
        ));
        output.push_str(&format!(
            "modelport_message_latency_ms_sum {}\n",
            inner.message_latency.sum_ms
        ));
        output.push_str(&format!(
            "modelport_message_latency_ms_count {}\n\n",
            inner.message_latency.count
        ));

        output.push_str(
            "# HELP modelport_ledger_operation_failures_total Ledger operation failures by bounded operation.\n",
        );
        output.push_str("# TYPE modelport_ledger_operation_failures_total counter\n");
        output.push_str(
            "# HELP modelport_ledger_operation_degraded Whether the latest ledger operation failed.\n",
        );
        output.push_str("# TYPE modelport_ledger_operation_degraded gauge\n");
        for (operation, metrics) in &inner.ledger_operations {
            let operation = escape_label_value(operation);
            output.push_str(&format!(
                "modelport_ledger_operation_failures_total{{operation=\"{operation}\"}} {}\n",
                metrics.failures_total
            ));
            output.push_str(&format!(
                "modelport_ledger_operation_degraded{{operation=\"{operation}\"}} {}\n",
                u8::from(metrics.degraded)
            ));
        }
        output.push_str(
            "# HELP modelport_ledger_reconciled_requests_total Expired request leases reconciled by this process.\n",
        );
        output.push_str("# TYPE modelport_ledger_reconciled_requests_total counter\n");
        output.push_str(&format!(
            "modelport_ledger_reconciled_requests_total {}\n",
            inner.reconciled_requests_total
        ));
        output.push_str(
            "# HELP modelport_ledger_reconciled_attempts_total Expired attempt leases reconciled by this process.\n",
        );
        output.push_str("# TYPE modelport_ledger_reconciled_attempts_total counter\n");
        output.push_str(&format!(
            "modelport_ledger_reconciled_attempts_total {}\n",
            inner.reconciled_attempts_total
        ));
        output.push('\n');

        output.push_str("# HELP modelport_runtime_adapter_collection_successes_total Successful Runtime Adapter Compute collection attempts.\n");
        output.push_str("# TYPE modelport_runtime_adapter_collection_successes_total counter\n");
        output.push_str("# HELP modelport_runtime_adapter_collection_failures_total Failed Runtime Adapter Compute collection attempts by bounded error class.\n");
        output.push_str("# TYPE modelport_runtime_adapter_collection_failures_total counter\n");
        output.push_str("# HELP modelport_runtime_adapter_collection_last_attempt_timestamp_seconds Unix timestamp of the latest Runtime Adapter collection attempt.\n");
        output.push_str(
            "# TYPE modelport_runtime_adapter_collection_last_attempt_timestamp_seconds gauge\n",
        );
        for (adapter_id, metrics) in &inner.runtime_adapter_collections {
            let adapter_id = escape_label_value(adapter_id);
            output.push_str(&format!(
                "modelport_runtime_adapter_collection_successes_total{{adapter_id=\"{adapter_id}\"}} {}\n",
                metrics.successes_total
            ));
            for (error, count) in &metrics.failures_total {
                output.push_str(&format!(
                    "modelport_runtime_adapter_collection_failures_total{{adapter_id=\"{adapter_id}\",error=\"{}\"}} {count}\n",
                    escape_label_value(error)
                ));
            }
            output.push_str(&format!(
                "modelport_runtime_adapter_collection_last_attempt_timestamp_seconds{{adapter_id=\"{adapter_id}\"}} {}\n",
                metrics.last_attempt_timestamp_seconds
            ));
        }

        output
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> MetricsSnapshot {
        let inner = self.inner.lock().expect("metrics lock poisoned");

        MetricsSnapshot {
            messages: inner
                .messages
                .iter()
                .map(|(key, message)| MessageMetricsSnapshot {
                    provider: key.provider.clone(),
                    model: key.model.clone(),
                    traffic_class: key.traffic_class.clone(),
                    stream: key.stream,
                    requests_total: message.counters.requests_total,
                    successes_total: message.counters.successes_total,
                    failures_total: message.counters.failures_total,
                })
                .collect(),
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl CounterSet {
    fn record(&mut self, success: bool, duration: Duration) {
        self.requests_total = self.requests_total.saturating_add(1);
        if success {
            self.successes_total = self.successes_total.saturating_add(1);
        } else {
            self.failures_total = self.failures_total.saturating_add(1);
        }
        self.duration_ms_total = self.duration_ms_total.saturating_add(duration_ms(duration));
    }
}

impl MessageCounterSet {
    fn record(&mut self, success: bool, duration: Duration, usage: UsageEstimate) {
        self.counters.record(success, duration);
        self.usage.record(usage);
    }
}

impl UsageCounterSet {
    fn record(&mut self, usage: UsageEstimate) {
        self.input_tokens_total = self.input_tokens_total.saturating_add(usage.input_tokens);
        self.output_tokens_total = self.output_tokens_total.saturating_add(usage.output_tokens);
        self.cache_write_tokens_total = self
            .cache_write_tokens_total
            .saturating_add(usage.cache_write_tokens);
        self.cache_read_tokens_total = self
            .cache_read_tokens_total
            .saturating_add(usage.cache_read_tokens);
        self.cost_estimate_usd_total += usage.cost_estimate.max(0.0);
    }
}

impl LatencyHistogram {
    fn record(&mut self, duration: Duration) {
        let duration_ms = duration_ms(duration);
        self.count = self.count.saturating_add(1);
        self.sum_ms = self.sum_ms.saturating_add(duration_ms);
        for (index, upper_bound) in MESSAGE_LATENCY_BUCKETS_MS.iter().enumerate() {
            if duration_ms <= *upper_bound {
                self.cumulative_buckets[index] = self.cumulative_buckets[index].saturating_add(1);
            }
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn push_counter_set(output: &mut String, prefix: &str, labels: &str, counters: &CounterSet) {
    output.push_str(&format!(
        "{prefix}_requests_total{{{labels}}} {}\n",
        counters.requests_total
    ));
    output.push_str(&format!(
        "{prefix}_successes_total{{{labels}}} {}\n",
        counters.successes_total
    ));
    output.push_str(&format!(
        "{prefix}_failures_total{{{labels}}} {}\n",
        counters.failures_total
    ));
    output.push_str(&format!(
        "{prefix}_duration_ms_total{{{labels}}} {}\n",
        counters.duration_ms_total
    ));
}

fn push_message_counter_set(output: &mut String, labels: &str, message: &MessageCounterSet) {
    push_counter_set(output, "modelport_message", labels, &message.counters);
    output.push_str(&format!(
        "modelport_message_input_tokens_total{{{labels}}} {}\n",
        message.usage.input_tokens_total
    ));
    output.push_str(&format!(
        "modelport_message_output_tokens_total{{{labels}}} {}\n",
        message.usage.output_tokens_total
    ));
    output.push_str(&format!(
        "modelport_message_cache_write_tokens_total{{{labels}}} {}\n",
        message.usage.cache_write_tokens_total
    ));
    output.push_str(&format!(
        "modelport_message_cache_read_tokens_total{{{labels}}} {}\n",
        message.usage.cache_read_tokens_total
    ));
    output.push_str(&format!(
        "modelport_message_cost_estimate_usd_total{{{labels}}} {}\n",
        message.usage.cost_estimate_usd_total
    ));
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prometheus_metrics() {
        let metrics = Metrics::new();
        metrics.record_route("messages", true, Duration::from_millis(12));
        metrics.record_rejection("messages", "validation", "invalid_request");
        metrics.record_ledger_operation("request_finalization", false);
        metrics.record_reconciliation(2, 3);
        metrics.record_runtime_adapter_collection_attempt("edge-1");
        metrics.record_runtime_adapter_collection_result("edge-1", Err("transport"));
        metrics.record_runtime_adapter_collection_attempt("edge-1");
        metrics.record_runtime_adapter_collection_result("edge-1", Ok(()));
        metrics.record_routing_decision("shadow", "balanced", "mimo", true);
        metrics.record_message(
            MessageMetricLabels {
                provider: "mimo",
                model: "mimo-v2.5-pro",
                traffic_class: "business",
                stream: false,
            },
            true,
            Duration::from_millis(12),
            UsageEstimate {
                input_tokens: 3,
                output_tokens: 4,
                cache_write_tokens: 5,
                cache_read_tokens: 6,
                cost_estimate: 0.000123,
                actual_cost: None,
                billable_cost: None,
            },
        );

        let rendered = metrics.render_prometheus();

        assert!(rendered.contains("modelport_uptime_seconds"));
        assert!(rendered.contains(&format!(
            r#"modelport_build_info{{version="{}""#,
            crate::version::VERSION
        )));
        assert!(rendered.contains(r#"modelport_route_requests_total{route="messages"} 1"#));
        assert_eq!(
            rendered
                .matches(r#"modelport_route_requests_total{route="messages"} 1"#)
                .count(),
            1,
            "a Prometheus scrape must not contain duplicate route samples"
        );
        assert!(rendered.contains(
            r#"modelport_message_requests_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 1"#
        ));
        assert!(rendered.contains(
            r#"modelport_message_input_tokens_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 3"#
        ));
        assert!(rendered.contains(
            r#"modelport_message_output_tokens_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 4"#
        ));
        assert!(rendered.contains(
            r#"modelport_message_cache_write_tokens_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 5"#
        ));
        assert!(rendered.contains(
            r#"modelport_message_cache_read_tokens_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 6"#
        ));
        assert!(rendered.contains(
            r#"modelport_message_cost_estimate_usd_total{provider="mimo",model="mimo-v2.5-pro",traffic_class="business",stream="false"} 0.000123"#
        ));
        assert!(rendered.contains(r#"modelport_message_latency_ms_bucket{le="100"} 1"#));
        assert!(rendered.contains(r#"modelport_message_latency_ms_bucket{le="+Inf"} 1"#));
        assert!(rendered.contains("modelport_message_latency_ms_sum 12"));
        assert!(rendered.contains("modelport_message_latency_ms_count 1"));
        assert!(rendered.contains(
            r#"modelport_inference_rejections_total{route="messages",phase="validation",reason="invalid_request"} 1"#
        ));
        assert!(rendered.contains(
            r#"modelport_ledger_operation_failures_total{operation="request_finalization"} 1"#
        ));
        assert!(rendered.contains(
            r#"modelport_ledger_operation_degraded{operation="request_finalization"} 1"#
        ));
        assert!(rendered.contains("modelport_ledger_reconciled_requests_total 2"));
        assert!(rendered.contains("modelport_ledger_reconciled_attempts_total 3"));
        assert!(rendered.contains(
            r#"modelport_runtime_adapter_collection_successes_total{adapter_id="edge-1"} 1"#
        ));
        assert!(rendered.contains(
            r#"modelport_runtime_adapter_collection_failures_total{adapter_id="edge-1",error="transport"} 1"#
        ));
        let last_attempt = rendered
            .lines()
            .find(|line| {
                line.starts_with(
                    "modelport_runtime_adapter_collection_last_attempt_timestamp_seconds{adapter_id=\"edge-1\"}",
                )
            })
            .expect("runtime adapter last-attempt metric");
        assert!(
            last_attempt
                .rsplit_once(' ')
                .unwrap()
                .1
                .parse::<u64>()
                .unwrap()
                > 0
        );
        assert!(rendered.contains(
            r#"modelport_routing_decisions_total{mode="shadow",profile="balanced",provider="mimo"} 1"#
        ));
        assert!(rendered.contains("modelport_routing_shadow_disagreements_total 1"));
        assert_eq!(
            metrics.degraded_ledger_operations(),
            vec!["request_finalization"]
        );
    }

    #[test]
    fn escapes_label_values() {
        assert_eq!(escape_label_value("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn bounds_user_controlled_model_series() {
        let metrics = Metrics {
            started_at: Instant::now(),
            inner: Mutex::new(MetricsInner::default()),
            max_message_series: 2,
        };
        for model in ["model-a", "model-b", "model-c", "model-d"] {
            metrics.record_message(
                MessageMetricLabels {
                    provider: "custom",
                    model,
                    traffic_class: "business",
                    stream: false,
                },
                true,
                Duration::from_millis(1),
                UsageEstimate::default(),
            );
        }

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.messages.len(), 2);
        assert!(
            snapshot
                .messages
                .iter()
                .any(|message| message.model == OVERFLOW_MODEL_LABEL && message.requests_total == 3)
        );
    }

    #[test]
    fn overflow_series_do_not_merge_business_and_synthetic_traffic() {
        let metrics = Metrics {
            started_at: Instant::now(),
            inner: Mutex::new(MetricsInner::default()),
            max_message_series: 5,
        };
        for (model, traffic_class) in [
            ("model-a", "business"),
            ("model-b", "business"),
            ("model-c", "business"),
            ("model-d", "synthetic"),
        ] {
            metrics.record_message(
                MessageMetricLabels {
                    provider: "custom",
                    model,
                    traffic_class,
                    stream: false,
                },
                true,
                Duration::from_millis(1),
                UsageEstimate::default(),
            );
        }

        let snapshot = metrics.snapshot();
        assert!(snapshot.messages.len() <= 5);
        assert!(snapshot.messages.iter().any(|message| {
            message.model == OVERFLOW_MODEL_LABEL && message.traffic_class == "business"
        }));
        assert!(snapshot.messages.iter().any(|message| {
            message.model == OVERFLOW_MODEL_LABEL && message.traffic_class == "synthetic"
        }));
    }
}
