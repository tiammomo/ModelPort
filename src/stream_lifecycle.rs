use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::pricing::TokenUsageBreakdown;

#[derive(Debug, Clone)]
pub(crate) struct StreamLifecycle {
    inner: Arc<Mutex<StreamLifecycleInner>>,
    started_at: Instant,
}

#[derive(Debug, Default)]
struct StreamLifecycleInner {
    state: UpstreamStreamState,
    usage: Option<TokenUsageBreakdown>,
    first_semantic_latency: Option<Duration>,
    response: ResponseObservation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ResponseObservation {
    pub(crate) tool_call_count: usize,
    pub(crate) text_present: bool,
    pub(crate) stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum UpstreamStreamState {
    #[default]
    Pending,
    Completed,
    Failed(String),
}

impl StreamLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamLifecycleInner::default())),
            started_at: Instant::now(),
        }
    }

    pub(crate) fn mark_first_semantic_event(&self) {
        let mut inner = self.inner.lock().expect("stream lifecycle lock poisoned");
        inner
            .first_semantic_latency
            .get_or_insert_with(|| self.started_at.elapsed());
    }

    pub(crate) fn first_semantic_latency(&self) -> Option<Duration> {
        self.inner
            .lock()
            .expect("stream lifecycle lock poisoned")
            .first_semantic_latency
    }

    pub(crate) fn observe_response_fragment(
        &self,
        tool_call_count: usize,
        text_present: bool,
        stop_reason: Option<&str>,
    ) {
        if tool_call_count > 0 || text_present {
            self.mark_first_semantic_event();
        }
        self.merge_response(tool_call_count, text_present, stop_reason);
    }

    fn merge_response(
        &self,
        tool_call_count: usize,
        text_present: bool,
        stop_reason: Option<&str>,
    ) {
        let mut inner = self.inner.lock().expect("stream lifecycle lock poisoned");
        inner.response.tool_call_count = inner.response.tool_call_count.max(tool_call_count);
        inner.response.text_present |= text_present;
        if let Some(stop_reason) = stop_reason {
            inner.response.stop_reason = Some(stop_reason.to_owned());
        }
    }

    pub(crate) fn response_observation(&self) -> ResponseObservation {
        self.inner
            .lock()
            .expect("stream lifecycle lock poisoned")
            .response
            .clone()
    }

    pub(crate) fn observe_openai_response(&self, response: &Value) {
        let Some(choice) = response
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };
        let message = choice.get("message").unwrap_or(&Value::Null);
        let tool_call_count = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
            + usize::from(message.get("function_call").is_some_and(Value::is_object));
        let text_present = message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty());
        self.merge_response(
            tool_call_count,
            text_present,
            choice.get("finish_reason").and_then(Value::as_str),
        );
    }

    pub(crate) fn observe_anthropic_response(&self, response: &Value) {
        let blocks = response
            .get("content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let tool_call_count = blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            .count();
        let text_present = blocks.iter().any(|block| {
            block.get("type").and_then(Value::as_str) == Some("text")
                && block
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
        });
        self.merge_response(
            tool_call_count,
            text_present,
            response.get("stop_reason").and_then(Value::as_str),
        );
    }

    pub(crate) fn mark_completed(&self) {
        let mut inner = self.inner.lock().expect("stream lifecycle lock poisoned");
        if matches!(inner.state, UpstreamStreamState::Pending) {
            inner.state = UpstreamStreamState::Completed;
        }
    }

    pub(crate) fn mark_failed(&self, error: impl Into<String>) {
        let mut inner = self.inner.lock().expect("stream lifecycle lock poisoned");
        if matches!(inner.state, UpstreamStreamState::Pending) {
            inner.state = UpstreamStreamState::Failed(error.into());
        }
    }

    pub(crate) fn state(&self) -> UpstreamStreamState {
        self.inner
            .lock()
            .expect("stream lifecycle lock poisoned")
            .state
            .clone()
    }

    pub(crate) fn merge_usage(&self, usage: TokenUsageBreakdown) {
        let mut inner = self.inner.lock().expect("stream lifecycle lock poisoned");
        let current = inner.usage.get_or_insert_default();
        current.input_tokens = current.input_tokens.max(usage.input_tokens);
        current.output_tokens = current.output_tokens.max(usage.output_tokens);
        current.cache_write_tokens = current.cache_write_tokens.max(usage.cache_write_tokens);
        current.cache_read_tokens = current.cache_read_tokens.max(usage.cache_read_tokens);
    }

    pub(crate) fn usage(&self) -> Option<TokenUsageBreakdown> {
        self.inner
            .lock()
            .expect("stream lifecycle lock poisoned")
            .usage
    }
}

impl Default for StreamLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamTerminalOutcome {
    Completed,
    UpstreamFailed(String),
    DeliveryFailed(String),
    DownstreamCancelled { upstream_state: UpstreamStreamState },
}

impl StreamTerminalOutcome {
    pub(crate) fn after_eof(lifecycle: &StreamLifecycle) -> Self {
        match lifecycle.state() {
            UpstreamStreamState::Completed => Self::Completed,
            UpstreamStreamState::Failed(error) => Self::UpstreamFailed(error),
            UpstreamStreamState::Pending => Self::UpstreamFailed(
                "stream body ended without a protocol terminal signal".to_owned(),
            ),
        }
    }

    pub(crate) fn after_body_error(lifecycle: &StreamLifecycle, error: String) -> Self {
        match lifecycle.state() {
            UpstreamStreamState::Failed(upstream_error) => Self::UpstreamFailed(upstream_error),
            UpstreamStreamState::Completed | UpstreamStreamState::Pending => {
                Self::DeliveryFailed(error)
            }
        }
    }

    pub(crate) fn after_drop(lifecycle: &StreamLifecycle) -> Self {
        Self::DownstreamCancelled {
            upstream_state: lifecycle.state(),
        }
    }

    pub(crate) fn success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    pub(crate) fn timed_out(&self) -> bool {
        self.error_message()
            .is_some_and(|message| message.to_ascii_lowercase().contains("timed out"))
    }

    pub(crate) fn status_code(&self) -> u16 {
        match self {
            Self::Completed => 200,
            Self::UpstreamFailed(_) if self.timed_out() => 504,
            Self::UpstreamFailed(_) | Self::DeliveryFailed(_) => 502,
            Self::DownstreamCancelled { .. } => 499,
        }
    }

    pub(crate) fn terminal_reason(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::UpstreamFailed(_) if self.timed_out() => "upstream_timeout",
            Self::UpstreamFailed(_) => "upstream_error",
            Self::DeliveryFailed(_) => "delivery_error",
            Self::DownstreamCancelled {
                upstream_state: UpstreamStreamState::Completed,
            } => "downstream_cancelled_after_upstream_complete",
            Self::DownstreamCancelled { .. } => "downstream_cancelled",
        }
    }

    pub(crate) fn error_message(&self) -> Option<&str> {
        match self {
            Self::Completed => None,
            Self::UpstreamFailed(error) | Self::DeliveryFailed(error) => Some(error),
            Self::DownstreamCancelled {
                upstream_state: UpstreamStreamState::Failed(error),
            } => Some(error),
            Self::DownstreamCancelled { .. } => {
                Some("downstream cancelled before stream delivery completed")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_lifecycle_finishes_successfully_at_eof() {
        let lifecycle = StreamLifecycle::new();
        lifecycle.mark_completed();
        let outcome = StreamTerminalOutcome::after_eof(&lifecycle);
        assert!(outcome.success());
        assert_eq!(outcome.terminal_reason(), "completed");
    }

    #[test]
    fn missing_terminal_signal_is_an_upstream_failure() {
        let outcome = StreamTerminalOutcome::after_eof(&StreamLifecycle::new());
        assert!(!outcome.success());
        assert_eq!(outcome.status_code(), 502);
        assert_eq!(outcome.terminal_reason(), "upstream_error");
    }

    #[test]
    fn downstream_drop_preserves_known_upstream_completion() {
        let lifecycle = StreamLifecycle::new();
        lifecycle.mark_completed();
        let outcome = StreamTerminalOutcome::after_drop(&lifecycle);
        assert_eq!(
            outcome.terminal_reason(),
            "downstream_cancelled_after_upstream_complete"
        );
        assert_eq!(outcome.status_code(), 499);
    }

    #[test]
    fn first_upstream_terminal_state_wins() {
        let lifecycle = StreamLifecycle::new();
        lifecycle.mark_completed();
        lifecycle.mark_failed("late failure");
        assert_eq!(lifecycle.state(), UpstreamStreamState::Completed);
    }

    #[test]
    fn usage_fragments_merge_by_token_dimension() {
        let lifecycle = StreamLifecycle::new();
        lifecycle.merge_usage(TokenUsageBreakdown {
            input_tokens: 10,
            output_tokens: 1,
            ..TokenUsageBreakdown::default()
        });
        lifecycle.merge_usage(TokenUsageBreakdown {
            input_tokens: 0,
            output_tokens: 7,
            cache_read_tokens: 3,
            ..TokenUsageBreakdown::default()
        });

        assert_eq!(
            lifecycle.usage(),
            Some(TokenUsageBreakdown {
                input_tokens: 10,
                output_tokens: 7,
                cache_read_tokens: 3,
                ..TokenUsageBreakdown::default()
            })
        );
    }

    #[test]
    fn response_observation_merges_stream_fragments_and_captures_ttft_once() {
        let lifecycle = StreamLifecycle::new();
        lifecycle.observe_response_fragment(0, true, None);
        let first = lifecycle.first_semantic_latency().unwrap();
        lifecycle.observe_response_fragment(2, false, Some("tool_calls"));

        assert_eq!(lifecycle.first_semantic_latency(), Some(first));
        assert_eq!(
            lifecycle.response_observation(),
            ResponseObservation {
                tool_call_count: 2,
                text_present: true,
                stop_reason: Some("tool_calls".to_owned()),
            }
        );
    }
}
