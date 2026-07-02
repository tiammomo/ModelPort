use std::{
    env,
    pin::Pin,
    time::{Duration, Instant},
};

use async_stream::try_stream;
use futures_util::{Stream, StreamExt};
use reqwest::{
    Client, Response,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde_json::Value;
use tracing::debug;

use crate::error::AppError;

pub type Header = (String, String);
pub type SseFrameStream = Pin<Box<dyn Stream<Item = Result<SseFrame, AppError>> + Send>>;

const MAX_ERROR_BODY_CHARS: usize = 8192;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
    max_response_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct SseFrame {
    pub event: Option<String>,
    pub id: Option<String>,
    pub retry: Option<Duration>,
    pub comments: Vec<String>,
    pub data: String,
}

impl HttpTransport {
    pub fn new() -> Result<Self, AppError> {
        let connect_timeout =
            Duration::from_secs(env_u64("MODELPORT_HTTP_CONNECT_TIMEOUT_SECS", 10));
        let request_timeout =
            Duration::from_secs(env_u64("MODELPORT_HTTP_REQUEST_TIMEOUT_SECS", 600));
        let stream_idle_timeout =
            Duration::from_secs(env_u64("MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS", 300));
        let max_response_bytes = env_usize(
            "MODELPORT_HTTP_MAX_RESPONSE_BYTES",
            DEFAULT_MAX_RESPONSE_BYTES,
        );
        let user_agent = env::var("MODELPORT_HTTP_USER_AGENT")
            .unwrap_or_else(|_| format!("model-port/{}", env!("CARGO_PKG_VERSION")));

        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(user_agent)
            .build()
            .map_err(|err| AppError::Transport(format!("failed to build HTTP client: {err}")))?;

        Ok(Self {
            client,
            request_timeout,
            stream_idle_timeout,
            max_response_bytes,
        })
    }

    pub async fn post_json(
        &self,
        url: &str,
        headers: &[Header],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, AppError> {
        let started = Instant::now();
        let response = self
            .client
            .post(url)
            .headers(header_map(headers)?)
            .json(body)
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        let body = response_body(response, self.max_response_bytes).await?;

        debug!(
            upstream_url = url,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            "upstream non-stream response"
        );

        if status.is_client_error() || status.is_server_error() {
            return Err(AppError::Upstream {
                status: status.as_u16(),
                body: sanitize_error_body(&body),
            });
        }

        serde_json::from_slice(&body).map_err(|err| {
            AppError::UpstreamProtocol(format!("upstream returned invalid JSON: {err}"))
        })
    }

    pub async fn get_json(
        &self,
        url: &str,
        headers: &[Header],
    ) -> Result<serde_json::Value, AppError> {
        let started = Instant::now();
        let response = self
            .client
            .get(url)
            .headers(header_map(headers)?)
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        let body = response_body(response, self.max_response_bytes).await?;

        debug!(
            upstream_url = url,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            "upstream get response"
        );

        if status.is_client_error() || status.is_server_error() {
            return Err(AppError::Upstream {
                status: status.as_u16(),
                body: sanitize_error_body(&body),
            });
        }

        serde_json::from_slice(&body).map_err(|err| {
            AppError::UpstreamProtocol(format!("upstream returned invalid JSON: {err}"))
        })
    }

    pub fn post_json_sse(
        &self,
        url: String,
        headers: Vec<Header>,
        body: serde_json::Value,
    ) -> SseFrameStream {
        let transport = self.clone();

        Box::pin(try_stream! {
            let started = Instant::now();
            let response = transport
                .client
                .post(&url)
                .headers(header_map(&headers)?)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&body)
                .send()
                .await
                .map_err(request_error)?;
            let status = response.status();

            if status.is_client_error() || status.is_server_error() {
                let body = response_body(response, transport.max_response_bytes).await?;
                Err(AppError::Upstream {
                    status: status.as_u16(),
                    body: sanitize_error_body(&body),
                })?;
            } else {
                debug!(
                    upstream_url = url,
                    status = status.as_u16(),
                    "upstream stream connected"
                );

                let mut chunks = response.bytes_stream();
                let mut line_buffer = Vec::new();
                let mut event: Option<String> = None;
                let mut id: Option<String> = None;
                let mut retry: Option<Duration> = None;
                let mut comments = Vec::new();
                let mut data = Vec::new();
                let mut raw_body = Vec::new();
                let mut yielded_frame = false;

                loop {
                    let chunk = tokio::time::timeout(
                        transport.stream_idle_timeout,
                        chunks.next(),
                    )
                    .await
                    .map_err(|_| {
                        AppError::Transport(format!(
                            "upstream stream idle timeout after {} seconds",
                            transport.stream_idle_timeout.as_secs()
                        ))
                    })?;

                    let Some(chunk) = chunk else {
                        break;
                    };
                    let chunk = chunk.map_err(request_error)?;
                    line_buffer.extend_from_slice(&chunk);

                    while let Some(index) = line_buffer.iter().position(|byte| *byte == b'\n') {
                        let mut line = line_buffer.drain(..=index).collect::<Vec<_>>();
                        trim_line_ending(&mut line);
                        if let Some(frame) = handle_sse_line(
                            &line,
                            &mut event,
                            &mut id,
                            &mut retry,
                            &mut comments,
                            &mut data,
                            &mut raw_body,
                        ) {
                            yielded_frame = true;
                            yield frame;
                        }
                    }
                }

                if !line_buffer.is_empty()
                    && let Some(frame) = handle_sse_line(
                        &line_buffer,
                        &mut event,
                        &mut id,
                        &mut retry,
                        &mut comments,
                        &mut data,
                        &mut raw_body,
                    )
                {
                    yielded_frame = true;
                    yield frame;
                }

                if !data.is_empty() {
                    yielded_frame = true;
                    yield SseFrame {
                        event,
                        id,
                        retry,
                        comments,
                        data: data.join("\n"),
                    };
                }

                let raw_body = sanitize_error_text(&raw_body.join("\n"));
                debug!(
                    upstream_url = url,
                    elapsed_ms = started.elapsed().as_millis(),
                    yielded_frame,
                    "upstream stream finished"
                );

                if !yielded_frame && !raw_body.is_empty() {
                    Err(AppError::UpstreamProtocol(format!(
                        "upstream returned a non-SSE response: {}",
                        raw_body
                    )))?;
                }
            }
        })
    }
}

fn handle_sse_line(
    line: &[u8],
    event: &mut Option<String>,
    id: &mut Option<String>,
    retry: &mut Option<Duration>,
    comments: &mut Vec<String>,
    data: &mut Vec<String>,
    raw_body: &mut Vec<String>,
) -> Option<SseFrame> {
    let line = String::from_utf8_lossy(line);

    if let Some(value) = line.strip_prefix("event:") {
        *event = Some(value.trim().to_owned());
        return None;
    }

    if let Some(value) = line.strip_prefix("id:") {
        *id = Some(value.trim_start().to_owned());
        return None;
    }

    if let Some(value) = line.strip_prefix("retry:") {
        if let Ok(millis) = value.trim().parse::<u64>() {
            *retry = Some(Duration::from_millis(millis));
        }
        return None;
    }

    if let Some(value) = line.strip_prefix("data:") {
        data.push(value.trim_start().to_owned());
        return None;
    }

    if let Some(value) = line.strip_prefix(':') {
        comments.push(value.trim_start().to_owned());
        return None;
    }

    if line.trim().is_empty() && !data.is_empty() {
        return Some(SseFrame {
            event: event.take(),
            id: id.take(),
            retry: retry.take(),
            comments: std::mem::take(comments),
            data: data.join("\n"),
        });
    }

    if !line.trim().is_empty() {
        raw_body.push(line.to_string());
    }

    None
}

async fn response_body(response: Response, limit: usize) -> Result<Vec<u8>, AppError> {
    let mut chunks = response.bytes_stream();
    let mut body = Vec::new();

    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(request_error)?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(AppError::UpstreamProtocol(format!(
                "upstream response exceeded MODELPORT_HTTP_MAX_RESPONSE_BYTES ({limit})"
            )));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

fn header_map(headers: &[Header]) -> Result<HeaderMap, AppError> {
    let mut map = HeaderMap::new();

    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|err| AppError::Config(format!("invalid upstream header `{name}`: {err}")))?;
        let value = HeaderValue::from_str(value).map_err(|err| {
            AppError::Config(format!("invalid value for upstream header `{name}`: {err}"))
        })?;
        map.insert(name, value);
    }

    Ok(map)
}

fn request_error(err: reqwest::Error) -> AppError {
    if err.is_timeout() {
        AppError::Transport(format!("upstream request timed out: {err}"))
    } else if err.is_connect() {
        AppError::Transport(format!("failed to connect to upstream: {err}"))
    } else {
        AppError::Transport(err.to_string())
    }
}

fn trim_line_ending(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}

fn truncate(value: String) -> String {
    if value.chars().count() <= MAX_ERROR_BODY_CHARS {
        return value;
    }

    let mut truncated = value.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>();
    truncated.push_str("... [truncated]");
    truncated
}

fn sanitize_error_body(body: &[u8]) -> String {
    sanitize_error_text(&String::from_utf8_lossy(body))
}

fn sanitize_error_text(value: &str) -> String {
    if let Ok(mut parsed) = serde_json::from_str::<Value>(value) {
        redact_json_value(&mut parsed);
        return truncate(parsed.to_string());
    }

    truncate(redact_secret_fragments(value))
}

fn redact_json_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                if sensitive_key(key) {
                    *value = Value::String("[redacted]".to_owned());
                } else {
                    redact_json_value(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_json_value(value);
            }
        }
        Value::String(value) => {
            *value = redact_secret_fragments(value);
        }
        _ => {}
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("api_key")
        || key.contains("apikey")
        || key.contains("authorization")
        || key.contains("access_token")
        || key.contains("refresh_token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("credential")
}

fn redact_secret_fragments(value: &str) -> String {
    let mut output = redact_after_marker(value, "Bearer ");
    output = redact_after_marker(&output, "sk-");
    output = redact_after_marker(&output, "sk_m");
    output
}

fn redact_after_marker(value: &str, marker: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(index) = rest.find(marker) {
        let (before, after_before) = rest.split_at(index);
        output.push_str(before);
        output.push_str(marker);

        let after_marker = &after_before[marker.len()..];
        let secret_len = after_marker
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            .map(char::len_utf8)
            .sum::<usize>();

        if secret_len >= 8 {
            output.push_str("[redacted]");
            rest = &after_marker[secret_len..];
        } else {
            rest = after_marker;
        }
    }

    output.push_str(rest);
    output
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_frame() {
        let mut event = None;
        let mut id = None;
        let mut retry = None;
        let mut comments = Vec::new();
        let mut data = Vec::new();
        let mut raw_body = Vec::new();

        assert!(
            handle_sse_line(
                b"event: content_block_delta",
                &mut event,
                &mut id,
                &mut retry,
                &mut comments,
                &mut data,
                &mut raw_body
            )
            .is_none()
        );
        assert!(
            handle_sse_line(
                b"data: {\"ok\":true}",
                &mut event,
                &mut id,
                &mut retry,
                &mut comments,
                &mut data,
                &mut raw_body
            )
            .is_none()
        );
        let frame = handle_sse_line(
            b"",
            &mut event,
            &mut id,
            &mut retry,
            &mut comments,
            &mut data,
            &mut raw_body,
        )
        .unwrap();

        assert_eq!(frame.event.as_deref(), Some("content_block_delta"));
        assert_eq!(frame.data, r#"{"ok":true}"#);
        assert!(raw_body.is_empty());
    }

    #[test]
    fn parses_sse_metadata_fields() {
        let mut event = None;
        let mut id = None;
        let mut retry = None;
        let mut comments = Vec::new();
        let mut data = Vec::new();
        let mut raw_body = Vec::new();

        assert!(
            handle_sse_line(
                b": upstream keepalive",
                &mut event,
                &mut id,
                &mut retry,
                &mut comments,
                &mut data,
                &mut raw_body,
            )
            .is_none()
        );
        assert!(
            handle_sse_line(
                b"id: evt_123",
                &mut event,
                &mut id,
                &mut retry,
                &mut comments,
                &mut data,
                &mut raw_body,
            )
            .is_none()
        );
        assert!(
            handle_sse_line(
                b"retry: 2500",
                &mut event,
                &mut id,
                &mut retry,
                &mut comments,
                &mut data,
                &mut raw_body,
            )
            .is_none()
        );
        assert!(
            handle_sse_line(
                b"data: {\"ok\":true}",
                &mut event,
                &mut id,
                &mut retry,
                &mut comments,
                &mut data,
                &mut raw_body,
            )
            .is_none()
        );
        let frame = handle_sse_line(
            b"",
            &mut event,
            &mut id,
            &mut retry,
            &mut comments,
            &mut data,
            &mut raw_body,
        )
        .unwrap();

        assert_eq!(frame.id.as_deref(), Some("evt_123"));
        assert_eq!(frame.retry, Some(Duration::from_millis(2500)));
        assert_eq!(frame.comments, vec!["upstream keepalive"]);
    }

    #[test]
    fn captures_non_sse_body_lines() {
        let mut event = None;
        let mut id = None;
        let mut retry = None;
        let mut comments = Vec::new();
        let mut data = Vec::new();
        let mut raw_body = Vec::new();

        assert!(
            handle_sse_line(
                b"{\"error\":\"bad key\"}",
                &mut event,
                &mut id,
                &mut retry,
                &mut comments,
                &mut data,
                &mut raw_body
            )
            .is_none()
        );

        assert_eq!(raw_body, vec![r#"{"error":"bad key"}"#]);
    }

    #[test]
    fn sanitizes_json_error_secrets() {
        let sanitized = sanitize_error_text(
            r#"{"error":{"message":"bad key sk-test-secret-value","api_key":"sk-live-secret-value"}}"#,
        );

        assert!(sanitized.contains("[redacted]"));
        assert!(!sanitized.contains("sk-test-secret-value"));
        assert!(!sanitized.contains("sk-live-secret-value"));
    }

    #[test]
    fn sanitizes_plain_text_bearer_tokens() {
        let sanitized = sanitize_error_text("upstream rejected Bearer sk-test-secret-value");

        assert!(sanitized.contains("Bearer [redacted]"));
        assert!(!sanitized.contains("sk-test-secret-value"));
    }
}
