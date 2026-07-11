use std::{
    env,
    pin::Pin,
    time::{Duration, Instant},
};

use async_stream::try_stream;
use futures_util::{Stream, StreamExt};
use reqwest::{
    Client, Response,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use tracing::debug;

use crate::error::AppError;

pub type Header = (String, String);
pub type SseFrameStream = Pin<Box<dyn Stream<Item = Result<SseFrame, AppError>> + Send>>;

const MAX_ERROR_BODY_CHARS: usize = 8192;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_SSE_LINE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_SSE_EVENT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_SSE_STREAM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
    max_response_bytes: usize,
    max_sse_line_bytes: usize,
    max_sse_event_bytes: usize,
    max_sse_stream_bytes: usize,
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
        let max_sse_line_bytes = env_usize(
            "MODELPORT_HTTP_SSE_MAX_LINE_BYTES",
            DEFAULT_MAX_SSE_LINE_BYTES,
        );
        let max_sse_event_bytes = env_usize(
            "MODELPORT_HTTP_SSE_MAX_EVENT_BYTES",
            DEFAULT_MAX_SSE_EVENT_BYTES,
        );
        let max_sse_stream_bytes = env_usize(
            "MODELPORT_HTTP_SSE_MAX_STREAM_BYTES",
            DEFAULT_MAX_SSE_STREAM_BYTES,
        );
        let user_agent = env::var("MODELPORT_HTTP_USER_AGENT")
            .unwrap_or_else(|_| format!("model-port/{}", env!("CARGO_PKG_VERSION")));

        let client = Client::builder()
            .connect_timeout(connect_timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .redirect(Policy::none())
            .user_agent(user_agent)
            .build()
            .map_err(|err| AppError::Transport(format!("failed to build HTTP client: {err}")))?;

        Ok(Self {
            client,
            request_timeout,
            stream_idle_timeout,
            max_response_bytes,
            max_sse_line_bytes,
            max_sse_event_bytes,
            max_sse_stream_bytes,
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

        if !status.is_success() {
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

        if !status.is_success() {
            return Err(AppError::Upstream {
                status: status.as_u16(),
                body: sanitize_error_body(&body),
            });
        }

        serde_json::from_slice(&body).map_err(|err| {
            AppError::UpstreamProtocol(format!("upstream returned invalid JSON: {err}"))
        })
    }

    pub async fn post_json_sse(
        &self,
        url: String,
        headers: Vec<Header>,
        body: serde_json::Value,
    ) -> Result<SseFrameStream, AppError> {
        let transport = self.clone();
        let started = Instant::now();
        let response = tokio::time::timeout(
            transport.request_timeout,
            transport
                .client
                .post(&url)
                .headers(header_map(&headers)?)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| {
            AppError::Transport(format!(
                "upstream SSE handshake timed out after {} seconds",
                transport.request_timeout.as_secs()
            ))
        })?
        .map_err(request_error)?;
        let status = response.status();

        if !status.is_success() {
            let body = response_body_with_timeouts(
                response,
                transport.max_response_bytes,
                transport.request_timeout,
                transport.stream_idle_timeout,
            )
            .await?;
            return Err(AppError::Upstream {
                status: status.as_u16(),
                body: sanitize_error_body(&body),
            });
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Err(AppError::UpstreamProtocol(
                "upstream SSE endpoint returned HTTP 204 with no event stream".to_owned(),
            ));
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<missing>")
            .to_owned();
        let has_sse_content_type = content_type
            .split(';')
            .next()
            .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("text/event-stream"));
        if !has_sse_content_type {
            let body = response_body_with_timeouts(
                response,
                transport.max_response_bytes,
                transport.request_timeout,
                transport.stream_idle_timeout,
            )
            .await?;
            return Err(AppError::UpstreamProtocol(format!(
                "upstream SSE endpoint returned `{content_type}` instead of text/event-stream: {}",
                sanitize_error_body(&body)
            )));
        }

        debug!(
            upstream_url = url,
            status = status.as_u16(),
            "upstream stream connected"
        );

        Ok(Box::pin(try_stream! {
                let mut chunks = response.bytes_stream();
                let mut line_buffer = Vec::new();
                let mut event: Option<String> = None;
                let mut id: Option<String> = None;
                let mut retry: Option<Duration> = None;
                let mut comments = Vec::new();
                let mut data = Vec::new();
                let mut raw_body = Vec::new();
                let mut yielded_frame = false;
                let mut event_received_bytes = 0usize;
                let mut stream_received_bytes = 0usize;

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
                    stream_received_bytes = checked_sse_bytes(
                        stream_received_bytes,
                        chunk.len(),
                        transport.max_sse_stream_bytes,
                        "stream",
                        "MODELPORT_HTTP_SSE_MAX_STREAM_BYTES",
                    )?;
                    line_buffer.extend_from_slice(&chunk);

                    while let Some(index) = line_buffer.iter().position(|byte| *byte == b'\n') {
                        let mut line = line_buffer.drain(..=index).collect::<Vec<_>>();
                        trim_line_ending(&mut line);
                        ensure_sse_line_limit(&line, transport.max_sse_line_bytes)?;
                        event_received_bytes = checked_sse_bytes(
                            event_received_bytes,
                            line.len(),
                            transport.max_sse_event_bytes,
                            "event",
                            "MODELPORT_HTTP_SSE_MAX_EVENT_BYTES",
                        )?;
                        if let Some(frame) = handle_sse_line(
                            &line,
                            &mut event,
                            &mut id,
                            &mut retry,
                            &mut comments,
                            &mut data,
                            &mut raw_body,
                        ) {
                            event_received_bytes = 0;
                            yielded_frame = true;
                            yield frame;
                        }
                    }

                    ensure_pending_sse_line_limit(
                        &line_buffer,
                        transport.max_sse_line_bytes,
                    )?;
                }

                if !line_buffer.is_empty() {
                    ensure_sse_line_limit(&line_buffer, transport.max_sse_line_bytes)?;
                    let _ = checked_sse_bytes(
                        event_received_bytes,
                        line_buffer.len(),
                        transport.max_sse_event_bytes,
                        "event",
                        "MODELPORT_HTTP_SSE_MAX_EVENT_BYTES",
                    )?;
                    if let Some(frame) = handle_sse_line(
                        &line_buffer,
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

            if !yielded_frame {
                let message = if raw_body.is_empty() {
                    "upstream SSE response ended before any data event".to_owned()
                } else {
                    format!("upstream returned a non-SSE response: {raw_body}")
                };
                Err(AppError::UpstreamProtocol(message))?;
            }
        }))
    }
}

fn ensure_sse_line_limit(line: &[u8], limit: usize) -> Result<(), AppError> {
    if line.len() > limit {
        return Err(sse_limit_error(
            "line",
            "MODELPORT_HTTP_SSE_MAX_LINE_BYTES",
            limit,
        ));
    }

    Ok(())
}

fn ensure_pending_sse_line_limit(line: &[u8], limit: usize) -> Result<(), AppError> {
    let line_len = if line.last() == Some(&b'\r') {
        line.len().saturating_sub(1)
    } else {
        line.len()
    };

    if line_len > limit {
        return Err(sse_limit_error(
            "line",
            "MODELPORT_HTTP_SSE_MAX_LINE_BYTES",
            limit,
        ));
    }

    Ok(())
}

fn checked_sse_bytes(
    current: usize,
    additional: usize,
    limit: usize,
    kind: &str,
    setting: &str,
) -> Result<usize, AppError> {
    let total = current
        .checked_add(additional)
        .ok_or_else(|| sse_limit_error(kind, setting, limit))?;
    if total > limit {
        return Err(sse_limit_error(kind, setting, limit));
    }

    Ok(total)
}

fn sse_limit_error(kind: &str, setting: &str, limit: usize) -> AppError {
    AppError::UpstreamProtocol(format!(
        "upstream SSE {kind} exceeded {setting} ({limit} bytes)"
    ))
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
            data: std::mem::take(data).join("\n"),
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

async fn response_body_with_timeouts(
    response: Response,
    limit: usize,
    total_timeout: Duration,
    idle_timeout: Duration,
) -> Result<Vec<u8>, AppError> {
    tokio::time::timeout(total_timeout, async move {
        let mut chunks = response.bytes_stream();
        let mut body = Vec::new();

        loop {
            let next = tokio::time::timeout(idle_timeout, chunks.next())
                .await
                .map_err(|_| {
                    AppError::Transport(format!(
                        "upstream error body idle timeout after {} seconds",
                        idle_timeout.as_secs()
                    ))
                })?;
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk.map_err(request_error)?;
            if body.len().saturating_add(chunk.len()) > limit {
                return Err(AppError::UpstreamProtocol(format!(
                    "upstream response exceeded MODELPORT_HTTP_MAX_RESPONSE_BYTES ({limit})"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    })
    .await
    .map_err(|_| {
        AppError::Transport(format!(
            "upstream error body timed out after {} seconds",
            total_timeout.as_secs()
        ))
    })?
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
    use axum::{
        Router,
        http::{StatusCode, header::CONTENT_TYPE, header::LOCATION},
        routing::{get, post},
    };
    use tokio::net::TcpListener;

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

    #[test]
    fn rejects_sse_limits_as_protocol_errors() {
        assert!(ensure_sse_line_limit(b"1234", 4).is_ok());
        assert!(matches!(
            ensure_sse_line_limit(b"12345", 4),
            Err(AppError::UpstreamProtocol(message))
                if message.contains("MODELPORT_HTTP_SSE_MAX_LINE_BYTES")
        ));
        assert_eq!(
            checked_sse_bytes(4, 4, 8, "event", "MODELPORT_HTTP_SSE_MAX_EVENT_BYTES").unwrap(),
            8
        );
        assert!(matches!(
            checked_sse_bytes(4, 5, 8, "event", "MODELPORT_HTTP_SSE_MAX_EVENT_BYTES"),
            Err(AppError::UpstreamProtocol(message))
                if message.contains("MODELPORT_HTTP_SSE_MAX_EVENT_BYTES")
        ));
    }

    #[tokio::test]
    async fn http_client_does_not_follow_redirects() {
        let app = Router::new()
            .route(
                "/redirect",
                get(|| async { (StatusCode::FOUND, [(LOCATION, "/target")]) }),
            )
            .route("/target", get(|| async { "redirected" }));
        let base_url = spawn_upstream(app).await;
        let transport = HttpTransport::new().unwrap();

        let response = transport
            .client
            .get(format!("{base_url}/redirect"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers().get(LOCATION).unwrap(), "/target");
    }

    #[tokio::test]
    async fn sse_handshake_applies_request_timeout() {
        let app = Router::new().route(
            "/stream",
            post(|| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                ([(CONTENT_TYPE, "text/event-stream")], "data: ok\n\n")
            }),
        );
        let base_url = spawn_upstream(app).await;
        let transport = test_transport(Duration::from_millis(25), 1024, 4096, 8192);
        let error = match transport
            .post_json_sse(
                format!("{base_url}/stream"),
                Vec::new(),
                serde_json::json!({}),
            )
            .await
        {
            Ok(_) => panic!("SSE handshake should have timed out"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Transport(message) if message.contains("timed out")
        ));
    }

    #[tokio::test]
    async fn sse_handshake_returns_upstream_status_before_streaming() {
        let app = Router::new().route(
            "/stream",
            post(|| async { (StatusCode::TOO_MANY_REQUESTS, "provider busy") }),
        );
        let base_url = spawn_upstream(app).await;
        let transport = test_transport(Duration::from_secs(1), 1024, 4096, 8192);

        let error = match transport
            .post_json_sse(
                format!("{base_url}/stream"),
                Vec::new(),
                serde_json::json!({}),
            )
            .await
        {
            Ok(_) => panic!("non-successful handshake should fail before returning a stream"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Upstream { status: 429, body } if body.contains("provider busy")
        ));
    }

    #[tokio::test]
    async fn sse_error_body_has_a_total_timeout() {
        let app = Router::new().route(
            "/stream",
            post(|| async {
                let chunks = futures_util::stream::unfold((), |_| async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Some((
                        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from_static(b"x")),
                        (),
                    ))
                });
                axum::response::Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .body(axum::body::Body::from_stream(chunks))
                    .unwrap()
            }),
        );
        let base_url = spawn_upstream(app).await;
        let transport = test_transport(Duration::from_millis(25), 1024, 4096, 8192);

        let error = match transport
            .post_json_sse(
                format!("{base_url}/stream"),
                Vec::new(),
                serde_json::json!({}),
            )
            .await
        {
            Ok(_) => panic!("slow upstream error body should time out"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Transport(message) if message.contains("error body timed out")
        ));
    }

    #[tokio::test]
    async fn sse_handshake_rejects_json_success_responses() {
        let app = Router::new().route(
            "/stream",
            post(|| async {
                (
                    [(CONTENT_TYPE, "application/json")],
                    r#"{"error":"not an event stream"}"#,
                )
            }),
        );
        let base_url = spawn_upstream(app).await;
        let transport = HttpTransport::new().unwrap();

        let error = match transport
            .post_json_sse(
                format!("{base_url}/stream"),
                Vec::new(),
                serde_json::json!({}),
            )
            .await
        {
            Ok(_) => panic!("JSON success response should fail the SSE handshake"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::UpstreamProtocol(message)
                if message.contains("application/json") && message.contains("not an event stream")
        ));
    }

    #[tokio::test]
    async fn sse_handshake_requires_event_stream_content_type() {
        let app = Router::new().route(
            "/stream",
            post(|| async {
                axum::response::Response::new(axum::body::Body::from("data: ok\n\n"))
            }),
        );
        let base_url = spawn_upstream(app).await;
        let transport = test_transport(Duration::from_secs(1), 1024, 4096, 8192);

        let error = match transport
            .post_json_sse(
                format!("{base_url}/stream"),
                Vec::new(),
                serde_json::json!({}),
            )
            .await
        {
            Ok(_) => panic!("missing SSE content type should fail the handshake"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::UpstreamProtocol(message) if message.contains("<missing>")
        ));
    }

    #[tokio::test]
    async fn empty_sse_body_is_a_protocol_error() {
        let url = spawn_sse_upstream("").await;
        let transport = test_transport(Duration::from_secs(1), 1024, 4096, 8192);
        let mut stream = transport
            .post_json_sse(url, Vec::new(), serde_json::json!({}))
            .await
            .unwrap();

        assert!(matches!(
            stream.next().await,
            Some(Err(AppError::UpstreamProtocol(message)))
                if message.contains("ended before any data event")
        ));
    }

    #[tokio::test]
    async fn sse_stream_enforces_line_event_and_total_limits() {
        let line_url = spawn_sse_upstream("data: 12345\n\n").await;
        let line_transport = test_transport(Duration::from_secs(1), 8, 1024, 1024);
        assert_sse_limit(
            line_transport,
            line_url,
            "MODELPORT_HTTP_SSE_MAX_LINE_BYTES",
        )
        .await;

        let event_url =
            spawn_sse_upstream(": note\nevent: delta\nid: 7\nretry: 5\ndata: one\ndata: two\n\n")
                .await;
        let event_transport = test_transport(Duration::from_secs(1), 64, 48, 1024);
        assert_sse_limit(
            event_transport,
            event_url,
            "MODELPORT_HTTP_SSE_MAX_EVENT_BYTES",
        )
        .await;

        let stream_url = spawn_sse_upstream("abcdef").await;
        let stream_transport = test_transport(Duration::from_secs(1), 64, 64, 5);
        assert_sse_limit(
            stream_transport,
            stream_url,
            "MODELPORT_HTTP_SSE_MAX_STREAM_BYTES",
        )
        .await;
    }

    #[tokio::test]
    async fn sse_event_limit_resets_after_each_frame() {
        let url = spawn_sse_upstream("event: x\ndata: 1\n\nevent: y\ndata: 2\n\n").await;
        let transport = test_transport(Duration::from_secs(1), 8, 15, 1024);
        let mut stream = transport
            .post_json_sse(url, Vec::new(), serde_json::json!({}))
            .await
            .unwrap();

        let first = stream.next().await.unwrap().unwrap();
        let second = stream.next().await.unwrap().unwrap();

        assert_eq!(first.event.as_deref(), Some("x"));
        assert_eq!(first.data, "1");
        assert_eq!(second.event.as_deref(), Some("y"));
        assert_eq!(second.data, "2");
        assert!(stream.next().await.is_none());
    }

    fn test_transport(
        request_timeout: Duration,
        max_sse_line_bytes: usize,
        max_sse_event_bytes: usize,
        max_sse_stream_bytes: usize,
    ) -> HttpTransport {
        HttpTransport {
            client: Client::builder().redirect(Policy::none()).build().unwrap(),
            request_timeout,
            stream_idle_timeout: Duration::from_secs(1),
            max_response_bytes: 1024,
            max_sse_line_bytes,
            max_sse_event_bytes,
            max_sse_stream_bytes,
        }
    }

    async fn assert_sse_limit(transport: HttpTransport, url: String, setting: &str) {
        let mut stream = transport
            .post_json_sse(url, Vec::new(), serde_json::json!({}))
            .await
            .unwrap();
        let error = stream.next().await.unwrap().unwrap_err();

        assert!(matches!(
            error,
            AppError::UpstreamProtocol(message) if message.contains(setting)
        ));
    }

    async fn spawn_sse_upstream(body: &'static str) -> String {
        let app = Router::new().route(
            "/stream",
            post(move || async move { ([(CONTENT_TYPE, "text/event-stream")], body) }),
        );
        let base_url = spawn_upstream(app).await;
        format!("{base_url}/stream")
    }

    async fn spawn_upstream(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{address}")
    }
}
