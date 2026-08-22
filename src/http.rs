use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
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

#[derive(Debug, Clone, Copy, Default)]
pub struct SseTimeouts {
    pub request: Option<Duration>,
    pub stream_idle: Option<Duration>,
}

impl SseTimeouts {
    pub fn new(request: Option<Duration>, stream_idle: Option<Duration>) -> Self {
        Self {
            request,
            stream_idle,
        }
    }
}

const MAX_ERROR_BODY_CHARS: usize = 8192;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_SSE_LINE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_SSE_EVENT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_SSE_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_PINNED_CLIENTS: usize = 128;
const MAX_UPSTREAM_RETRY_AFTER_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
    connect_timeout: Duration,
    user_agent: String,
    pinned_clients: Arc<Mutex<HashMap<PinnedClientKey, Client>>>,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
    max_response_bytes: usize,
    max_sse_line_bytes: usize,
    max_sse_event_bytes: usize,
    max_sse_stream_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PinnedClientKey {
    dns_name: String,
    addresses: Vec<SocketAddr>,
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

        let client = build_client(connect_timeout, &user_agent, None)?;

        Ok(Self {
            client,
            connect_timeout,
            user_agent,
            pinned_clients: Arc::new(Mutex::new(HashMap::new())),
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
        provider_id: &str,
        allow_private_provider_urls: bool,
        url: &str,
        headers: &[Header],
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, AppError> {
        self.post_json_with_timeout(
            provider_id,
            allow_private_provider_urls,
            url,
            headers,
            body,
            None,
        )
        .await
    }

    pub async fn post_json_with_timeout(
        &self,
        provider_id: &str,
        allow_private_provider_urls: bool,
        url: &str,
        headers: &[Header],
        body: &serde_json::Value,
        request_timeout: Option<Duration>,
    ) -> Result<serde_json::Value, AppError> {
        let started = Instant::now();
        let request_timeout = request_timeout.unwrap_or(self.request_timeout);
        let client = self
            .client_for_provider(provider_id, url, allow_private_provider_urls)
            .await?;
        let response = client
            .post(url)
            .headers(header_map(headers)?)
            .json(body)
            .timeout(request_timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        let retry_after_secs = upstream_retry_after_secs(response.headers());
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
                retry_after_secs,
            });
        }

        serde_json::from_slice(&body).map_err(|err| {
            AppError::UpstreamProtocol(format!("upstream returned invalid JSON: {err}"))
        })
    }

    pub async fn get_json(
        &self,
        provider_id: &str,
        allow_private_provider_urls: bool,
        url: &str,
        headers: &[Header],
    ) -> Result<serde_json::Value, AppError> {
        let started = Instant::now();
        let client = self
            .client_for_provider(provider_id, url, allow_private_provider_urls)
            .await?;
        let response = client
            .get(url)
            .headers(header_map(headers)?)
            .timeout(self.request_timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        let retry_after_secs = upstream_retry_after_secs(response.headers());
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
                retry_after_secs,
            });
        }

        serde_json::from_slice(&body).map_err(|err| {
            AppError::UpstreamProtocol(format!("upstream returned invalid JSON: {err}"))
        })
    }

    #[cfg(test)]
    pub async fn post_json_sse(
        &self,
        provider_id: &str,
        allow_private_provider_urls: bool,
        url: String,
        headers: Vec<Header>,
        body: serde_json::Value,
    ) -> Result<SseFrameStream, AppError> {
        self.post_json_sse_with_timeouts(
            provider_id,
            allow_private_provider_urls,
            url,
            headers,
            body,
            SseTimeouts::default(),
        )
        .await
    }

    pub async fn post_json_sse_with_timeouts(
        &self,
        provider_id: &str,
        allow_private_provider_urls: bool,
        url: String,
        headers: Vec<Header>,
        body: serde_json::Value,
        timeouts: SseTimeouts,
    ) -> Result<SseFrameStream, AppError> {
        let mut transport = self.clone();
        transport.request_timeout = timeouts.request.unwrap_or(transport.request_timeout);
        transport.stream_idle_timeout = timeouts
            .stream_idle
            .unwrap_or(transport.stream_idle_timeout);
        let started = Instant::now();
        let client = transport
            .client_for_provider(provider_id, &url, allow_private_provider_urls)
            .await?;
        let response = tokio::time::timeout(
            transport.request_timeout,
            client
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
        let retry_after_secs = upstream_retry_after_secs(response.headers());

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
                retry_after_secs,
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
                    let total_remaining = transport
                        .request_timeout
                        .checked_sub(started.elapsed())
                        .ok_or_else(|| AppError::Transport(format!(
                            "upstream stream total timeout after {} seconds",
                            transport.request_timeout.as_secs()
                        )))?;
                    let read_timeout = transport.stream_idle_timeout.min(total_remaining);
                    let chunk = tokio::time::timeout(
                        read_timeout,
                        chunks.next(),
                    )
                    .await
                    .map_err(|_| {
                        if started.elapsed() >= transport.request_timeout {
                            AppError::Transport(format!(
                                "upstream stream total timeout after {} seconds",
                                transport.request_timeout.as_secs()
                            ))
                        } else {
                            AppError::Transport(format!(
                                "upstream stream idle timeout after {} seconds",
                                transport.stream_idle_timeout.as_secs()
                            ))
                        }
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

    async fn client_for_provider(
        &self,
        provider_id: &str,
        url: &str,
        allow_private_provider_urls: bool,
    ) -> Result<Client, AppError> {
        let pin = crate::config::resolve_provider_base_url_for_connection(
            provider_id,
            url,
            allow_private_provider_urls,
        )
        .await?;
        let Some(dns_name) = pin.dns_name else {
            // Literal IP URLs do not perform a second DNS lookup. Their IP was
            // checked by the same policy immediately above.
            return Ok(self.client.clone());
        };
        let key = PinnedClientKey {
            dns_name,
            addresses: pin.addresses,
        };
        if let Some(client) = self
            .pinned_clients
            .lock()
            .expect("pinned HTTP client cache lock poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(client);
        }

        let client = build_client(self.connect_timeout, &self.user_agent, Some(&key))?;
        let mut clients = self
            .pinned_clients
            .lock()
            .expect("pinned HTTP client cache lock poisoned");
        if clients.len() >= MAX_PINNED_CLIENTS {
            clients.clear();
        }
        Ok(clients.entry(key).or_insert_with(|| client.clone()).clone())
    }
}

fn build_client(
    connect_timeout: Duration,
    user_agent: &str,
    pin: Option<&PinnedClientKey>,
) -> Result<Client, AppError> {
    let mut builder = Client::builder()
        .connect_timeout(connect_timeout)
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(Policy::none())
        // Environment proxies can resolve the destination independently and
        // would invalidate connection-level DNS pinning.
        .no_proxy()
        .user_agent(user_agent);
    if let Some(pin) = pin {
        builder = builder.resolve_to_addrs(&pin.dns_name, &pin.addresses);
    }
    builder
        .build()
        .map_err(|err| AppError::Transport(format!("failed to build HTTP client: {err}")))
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

fn upstream_retry_after_secs(headers: &HeaderMap) -> Option<u64> {
    let value = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();
    let seconds = value.parse::<u64>().ok().or_else(|| {
        let deadline = httpdate::parse_http_date(value).ok()?;
        deadline
            .duration_since(std::time::SystemTime::now())
            .ok()
            .map(|duration| duration.as_secs())
    })?;
    Some(seconds.min(MAX_UPSTREAM_RETRY_AFTER_SECS))
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
    fn retry_after_supports_seconds_and_caps_http_dates() {
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, HeaderValue::from_static("7"));
        assert_eq!(upstream_retry_after_secs(&headers), Some(7));

        let future = std::time::SystemTime::now() + Duration::from_secs(600);
        headers.insert(
            reqwest::header::RETRY_AFTER,
            HeaderValue::from_str(&httpdate::fmt_http_date(future)).unwrap(),
        );
        assert_eq!(
            upstream_retry_after_secs(&headers),
            Some(MAX_UPSTREAM_RETRY_AFTER_SECS)
        );
    }

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
    async fn pinned_client_connects_only_to_validated_addresses_and_preserves_host() {
        let app = Router::new().route(
            "/host",
            get(|headers: axum::http::HeaderMap| async move {
                headers
                    .get(axum::http::header::HOST)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("missing")
                    .to_owned()
            }),
        );
        let base_url = spawn_upstream(app).await;
        let parsed = reqwest::Url::parse(&base_url).unwrap();
        let address = SocketAddr::new(
            parsed.host_str().unwrap().parse().unwrap(),
            parsed.port().unwrap(),
        );
        let pin = PinnedClientKey {
            dns_name: "provider.invalid".to_owned(),
            addresses: vec![address],
        };
        let client = build_client(
            Duration::from_secs(1),
            "model-port-pinning-test",
            Some(&pin),
        )
        .unwrap();

        let response = client
            .get(format!("http://provider.invalid:{}/host", address.port()))
            .send()
            .await
            .unwrap();
        let host = response.text().await.unwrap();

        assert_eq!(host, format!("provider.invalid:{}", address.port()));
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
                "local_test",
                true,
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
                "local_test",
                true,
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
            AppError::Upstream { status: 429, body, .. } if body.contains("provider busy")
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
        // Keep enough headroom for the local server to schedule under a fully
        // parallel test run. The body is infinite, so this still exercises the
        // total body timeout rather than the SSE handshake timeout.
        let transport = test_transport(Duration::from_millis(250), 1024, 4096, 8192);

        let error = match transport
            .post_json_sse(
                "local_test",
                true,
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
    async fn successful_sse_stream_has_a_total_timeout() {
        let app = Router::new().route(
            "/stream",
            post(|| async {
                let chunks = futures_util::stream::unfold(0u64, |index| async move {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Some((
                        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(format!(
                            "data: {index}\n\n"
                        ))),
                        index.saturating_add(1),
                    ))
                });
                axum::response::Response::builder()
                    .header(CONTENT_TYPE, "text/event-stream")
                    .body(axum::body::Body::from_stream(chunks))
                    .unwrap()
            }),
        );
        let base_url = spawn_upstream(app).await;
        let transport = test_transport(Duration::from_millis(120), 1024, 4096, 8192);
        let mut stream = transport
            .post_json_sse(
                "local_test",
                true,
                format!("{base_url}/stream"),
                Vec::new(),
                serde_json::json!({}),
            )
            .await
            .unwrap();

        let error = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(Err(error)) = stream.next().await {
                    break error;
                }
            }
        })
        .await
        .expect("stream should stop at its total timeout");

        assert!(matches!(
            error,
            AppError::Transport(message) if message.contains("stream total timeout")
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
                "local_test",
                true,
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
                "local_test",
                true,
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
            .post_json_sse("local_test", true, url, Vec::new(), serde_json::json!({}))
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
            .post_json_sse("local_test", true, url, Vec::new(), serde_json::json!({}))
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
            client: Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .build()
                .unwrap(),
            connect_timeout: Duration::from_secs(1),
            user_agent: "model-port-test".to_owned(),
            pinned_clients: Arc::new(Mutex::new(HashMap::new())),
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
            .post_json_sse("local_test", true, url, Vec::new(), serde_json::json!({}))
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
