use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct Metrics {
    started_at: Instant,
    inner: Mutex<MetricsInner>,
}

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub uptime_seconds: u64,
    pub routes: Vec<RouteMetricsSnapshot>,
    pub messages: Vec<MessageMetricsSnapshot>,
}

#[derive(Debug, Clone)]
pub struct RouteMetricsSnapshot {
    pub route: String,
    pub requests_total: u64,
    pub successes_total: u64,
    pub failures_total: u64,
    pub duration_ms_total: u64,
}

#[derive(Debug, Clone)]
pub struct MessageMetricsSnapshot {
    pub provider: String,
    pub model: String,
    pub stream: bool,
    pub requests_total: u64,
    pub successes_total: u64,
    pub failures_total: u64,
    pub duration_ms_total: u64,
}

#[derive(Debug, Default)]
struct MetricsInner {
    routes: BTreeMap<String, CounterSet>,
    messages: BTreeMap<MessageKey, CounterSet>,
}

#[derive(Debug, Default)]
struct CounterSet {
    requests_total: u64,
    successes_total: u64,
    failures_total: u64,
    duration_ms_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MessageKey {
    provider: String,
    model: String,
    stream: bool,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
            inner: Mutex::new(MetricsInner::default()),
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

    pub fn record_message(
        &self,
        provider: &str,
        model: &str,
        stream: bool,
        success: bool,
        duration: Duration,
    ) {
        let mut inner = self.inner.lock().expect("metrics lock poisoned");
        inner
            .messages
            .entry(MessageKey {
                provider: provider.to_owned(),
                model: model.to_owned(),
                stream,
            })
            .or_default()
            .record(success, duration);
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

        output.push_str("# HELP modelport_message_requests_total Total message requests by provider/model/stream.\n");
        output.push_str("# TYPE modelport_message_requests_total counter\n");
        output.push_str("# HELP modelport_message_successes_total Total successful message requests by provider/model/stream.\n");
        output.push_str("# TYPE modelport_message_successes_total counter\n");
        output.push_str("# HELP modelport_message_failures_total Total failed message requests by provider/model/stream.\n");
        output.push_str("# TYPE modelport_message_failures_total counter\n");
        output.push_str("# HELP modelport_message_duration_ms_total Total message request setup duration in milliseconds.\n");
        output.push_str("# TYPE modelport_message_duration_ms_total counter\n");
        for (key, counters) in &inner.messages {
            let labels = format!(
                "provider=\"{}\",model=\"{}\",stream=\"{}\"",
                escape_label_value(&key.provider),
                escape_label_value(&key.model),
                key.stream
            );
            push_counter_set(&mut output, "modelport_message", &labels, counters);
        }

        output
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let inner = self.inner.lock().expect("metrics lock poisoned");

        MetricsSnapshot {
            uptime_seconds: self.started_at.elapsed().as_secs(),
            routes: inner
                .routes
                .iter()
                .map(|(route, counters)| RouteMetricsSnapshot {
                    route: route.clone(),
                    requests_total: counters.requests_total,
                    successes_total: counters.successes_total,
                    failures_total: counters.failures_total,
                    duration_ms_total: counters.duration_ms_total,
                })
                .collect(),
            messages: inner
                .messages
                .iter()
                .map(|(key, counters)| MessageMetricsSnapshot {
                    provider: key.provider.clone(),
                    model: key.model.clone(),
                    stream: key.stream,
                    requests_total: counters.requests_total,
                    successes_total: counters.successes_total,
                    failures_total: counters.failures_total,
                    duration_ms_total: counters.duration_ms_total,
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
        metrics.record_message(
            "mimo",
            "mimo-v2.5-pro",
            false,
            true,
            Duration::from_millis(12),
        );

        let rendered = metrics.render_prometheus();

        assert!(rendered.contains("modelport_uptime_seconds"));
        assert!(rendered.contains(r#"modelport_route_requests_total{route="messages"} 1"#));
        assert!(rendered.contains(
            r#"modelport_message_requests_total{provider="mimo",model="mimo-v2.5-pro",stream="false"} 1"#
        ));
    }

    #[test]
    fn escapes_label_values() {
        assert_eq!(escape_label_value("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
