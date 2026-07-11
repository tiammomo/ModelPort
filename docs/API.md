# API Reference

ModelPort exposes an Anthropic-compatible data plane and a dashboard control
plane. The default backend origin is `http://127.0.0.1:17878`.

## Public Endpoints

| Endpoint | Authentication | Semantics |
| --- | --- | --- |
| `GET /livez` | none | Process liveness only. |
| `GET /health` | optional | Minimal public body; router authentication adds provider and storage diagnostics. |
| `GET /readyz` | router/API key | Auth/control storage readiness plus detailed diagnostics; Provider degradation does not fail it. |
| `GET /v1/models` | router/API key | Configured, visible model and alias catalog. Visibility does not prove upstream health. |
| `POST /v1/messages` | router/API key | Anthropic-compatible Messages request. |
| `GET /metrics` | router/API key | Prometheus text exposition. |

Authenticate with either header:

```http
x-api-key: <token>
Authorization: Bearer <token>
```

Dashboard-issued API keys are checked before the legacy router token. Set
`MODELPORT_REQUIRE_CONTROL_API_KEYS=1` after creating a key to enforce identity,
team, model/provider, IP, spend, quota, and per-key policy on every data-plane
request. It also rejects the legacy token for authenticated diagnostics and
metrics. The legacy token represents one unrestricted local identity.

## Messages

```bash
curl -sS http://127.0.0.1:17878/v1/messages \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
    "model": "deepseek-v4-flash",
    "max_tokens": 96,
    "messages": [{"role":"user","content":"Reply exactly: OK"}]
  }'
```

The accepted client shape is Anthropic Messages-oriented. `model`, a non-empty
`messages` array, and `max_tokens` are required. `max_tokens` must be an integer
greater than zero and no greater than `MODELPORT_MAX_OUTPUT_TOKENS` (default
`131072`); missing, zero, or oversized values return HTTP 400 before provider
routing. Each message role must be `user` or `assistant`; content may be a
string or an array of content blocks. Unknown top-level fields are preserved
where the adapter supports them, but strict fidelity mode rejects Anthropic
features that cannot be represented safely by an OpenAI-compatible provider.

Request-size and Tool Use limits are documented in
[Configuration](CONFIGURATION.md#rate-limits-and-request-guardrails).

## Model Resolution

Resolution is deterministic:

1. `provider:model` selects an enabled provider explicitly.
2. An exact alias is resolved, with a maximum alias depth.
3. An exact configured model is selected in provider order.
4. A model prefix is selected in provider order.
5. The default provider receives its default model, or the unknown model when
   that provider enables arbitrary passthrough.

An entry in `/v1/models` means it is configured and passes the local credential
visibility check. Keyless local/custom providers can appear even when their
runtime is offline; use provider testing or a real request for health evidence.

## Streaming

Set `"stream": true` to receive Anthropic-style server-sent events. Common
events are:

```text
message_start
content_block_start
content_block_delta
content_block_stop
message_delta
message_stop
error
```

OpenAI-compatible text deltas and tool-call deltas are converted into
Anthropic content blocks. A failure after response headers have been sent is
reported as `event: error` while the HTTP status remains 200. Consumers must
inspect the event stream rather than treating the initial status as completion.

The transport establishes the upstream response and checks its initial status
before sending the local stream, allowing pre-header connect/HTTP failures to
fallback. It also enforces per-line, per-event, total-stream and idle limits.
The request log still records live-stream acceptance before final completion;
see [Architecture](ARCHITECTURE.md#streaming-boundary).

`MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` covers a complete non-stream request, but
for SSE it covers only `send()` through the upstream response-header handshake.
After a live stream is established there is no fixed total-duration timeout.
Each received chunk resets `MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS`; line,
event, and total raw-stream byte limits remain in force. A continuously active,
bounded stream may legitimately outlive the request-timeout value.

Streaming requests also require a process-local permit. The cap is
`MODELPORT_MAX_CONCURRENT_STREAMS`, or the effective general request cap when
unset. Exhaustion returns HTTP 429 with `Retry-After: 1` before an upstream
attempt. A permit remains held until the downstream body finishes or is
dropped, not merely until the handler returns.

The SSE handshake requires a 2xx status other than 204 and a
`Content-Type` whose media type is `text/event-stream`. A missing or non-SSE
content type such as `application/json` is rejected even when the status is
otherwise 2xx; parameters such as `charset` are accepted. For non-2xx and
wrong-content-type responses, the error body is subject to the response byte
limit, a total request timeout, and the resettable stream-idle timeout before it
is redacted and returned. These failures occur before downstream headers and
can participate in fallback.

A native Anthropic stream is complete only after `message_stop`. An
OpenAI-compatible stream must provide `[DONE]` or a `finish_reason`; ModelPort
then emits the Anthropic `message_stop`. EOF without the required termination
signal is an upstream-protocol failure. If local HTTP 200 headers have already
been sent, that failure is delivered as an SSE `event: error` and cannot trigger
cross-provider fallback.

For an OpenAI-compatible provider with `buffer_stream_text=true`, ModelPort
instead waits for a complete non-stream upstream response and converts it before
returning locally chunked SSE. Upstream HTTP or conversion failures are normal
HTTP errors rather than post-200 SSE errors, and reported upstream usage is
available to request accounting. This mode delays the first downstream byte
until generation completes and does not preserve live-stream cancellation
semantics.

## Tool Use

The data plane accepts Anthropic `tools`, `tool_choice`, assistant `tool_use`,
and user `tool_result`. OpenAI-compatible providers use function tools and are
mapped back to Anthropic blocks. Validation and known provider limits are in
[Tool Use Compatibility](TOOL_USE_COMPATIBILITY.md).

## Request IDs

ModelPort accepts `x-request-id`; otherwise middleware assigns one. The response
propagates it and completed usage records retain it for dashboard correlation.
Both built-in protocol adapters forward it to the upstream request, although an
upstream may ignore, replace, or omit it from its own logs. Treat it as an opaque
diagnostic identifier, not an authorization token or a complete distributed
trace; ModelPort does not attach trace/span-parent semantics.

## Errors

Non-stream failures use a stable envelope:

```json
{
  "error": {
    "type": "invalid_request_error",
    "code": "invalid_request",
    "status": 400,
    "message": "invalid request: model is required",
    "hint": "..."
  }
}
```

Important categories include `authentication_error`, `forbidden_error`,
`quota_exceeded`, `rate_limit_error`, `invalid_request_error`,
`upstream_error`, and `server_error`. A failed `/readyz` storage check returns
HTTP 503 with code `not_ready`. Local rate-limit responses include `Retry-After`.
Quota/spend-limit failures also use HTTP 429 but are not the same sliding-window
condition.

Every upstream GET, POST, and SSE handshake requires a 2xx status. Redirects are
not followed. Upstream 4xx/5xx errors retain their status through the existing
error mapping where applicable; informational 1xx, redirect 3xx, and invalid
statuses become a client-facing 502 so a gateway failure is never mistaken for
a redirect. Upstream error bodies are bounded and common credential patterns
are redacted, but clients should not publish full error payloads without
reviewing them.

## Dashboard Control Plane

Dashboard login is `POST /admin/auth/login`; it is the only normal admin route
that does not already require a session. The server sets an HttpOnly,
SameSite=Lax cookie. Use `MODELPORT_ADMIN_COOKIE_SECURE=1` behind HTTPS.

Route groups include:

- `/admin/auth/*`: login, logout, current session.
- `/admin/dashboard`, `/admin/logs`, `/admin/logs/{id}`, `/admin/latency`:
  operational views.
- `/admin/users`, `/admin/api-keys`, `/admin/teams`, `/admin/quotas`: identity
  and policy.
- `/admin/providers`, `/admin/aliases`: provider lifecycle, credentials, model
  inventory, and routes.
- `/admin/settings`, `/admin/settings/reload-config`: runtime view, default
  provider/order updates, and base-config reload.
- `GET /admin/audit`, `POST /admin/backup`: audit events and a redacted,
  non-restorable diagnostic snapshot.

### Identity, API Keys, Teams, And Quotas

Control-plane permissions are role-aware:

| Role | API-key visibility and writes |
| --- | --- |
| `admin` | Read all keys; create, edit policy/status/team/expiry/spend settings, revoke, restore, and delete. |
| `user` | Read only owned keys; edit only `name` and `group`, revoke or delete an owned key; cannot create, restore, or change policy. |
| `viewer` | Read-only; no API-key, team, quota, user, or Provider writes. |

Creating an API key is admin-only and `userId` must identify an existing
`active` user. The server ignores a caller-supplied username and stores the
canonical username from the auth store. Data-plane authentication rechecks
that the owner still exists and remains active. Disabling or suspending a user
revokes that user's keys and removes the user's quotas; an orphaned or inactive
owner cannot continue using an old key.

When supplied, API-key `expiresAt` is a decimal Unix epoch millisecond string.
Malformed values and timestamps at or before the current time return HTTP 400;
they are never silently converted into a non-expiring key.

User quotas (`daily`, `weekly`, `monthly`) reset on UTC calendar boundaries:
00:00 UTC each day, Monday 00:00 UTC each week, and 00:00 UTC on the first day
of each month. A quota create/update must reference a real user; the server
again derives the canonical username.

API-key and team spend limits are a different contract. The fields
`fiveHourLimitUsd`, `dailyLimitUsd`, `weeklyLimitUsd`, and `monthlyLimitUsd`
mean rolling 5-hour, 24-hour, 7-day, and 30-day windows. They are enforced for
an API key only when the persisted compatibility field `rateLimited` is true;
the dashboard labels this switch “periodic spend limits”, not request-rate
limiting. Spend uses hourly buckets, so the oldest bucket overlapping a rolling
window is included in full. The boundary can therefore conservatively
over-count by up to almost one hour.

Only a request that actually starts an upstream attempt increments a user quota
or the API-key/team spend ledger. Attempt-level credential, policy, quota,
capability, URL, or Provider-rate rejection before `send()` can produce a
post-routing log row with zero usage and no charge. Earlier ingress failures
such as authentication, request-shape validation, model resolution, global
rate limiting, or stream-permit exhaustion may return before a persisted usage
row is created; they still never consume quota/spend.

Deleting a team is rejected with HTTP 400 while any API key—active or
revoked—still references it. Reassign or delete every referencing key first;
team deletion never silently removes policy by unbinding keys.

Provider update bodies use camelCase. Omitting `apiKeyEnv` preserves the
current environment-variable name. Send `clearApiKeyEnv: true` to clear it;
do not combine that flag with a non-empty `apiKeyEnv`, which returns HTTP 400.
An empty dashboard field is serialized as the explicit clear flag.

Non-local/non-custom Provider URLs must use HTTPS unless the process starts
with `MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1`. The override is intended only
for a trusted internal upstream because HTTP exposes the referenced Provider
API key and request/response content in plaintext. Local/custom runtime classes
retain HTTP support for controlled local integration.

Credential pool modes are also fail-safe. `manual` preserves the operator's
selected non-disabled credential. `failover` and `round_robin` select only
usable credentials; if a configured pool has none, that Provider attempt fails
closed and routing may continue to another Provider candidate. It does not fall
back to a disabled, cooling-down, or missing-environment credential.

`POST /admin/backup` is intentionally not a safe GET: it creates an audit event
and requires an admin session plus `X-ModelPort-CSRF`. Likewise,
`POST /admin/providers/{provider_id}/models` performs upstream model discovery
and persists provider-test/audit state. Its former side-effecting GET alias is
not supported. Provider model create/update/delete operations share the path
with their documented mutating methods. `POST /admin/users` is also a mutating
operation and has no CSRF exemption.

### Dashboard Ranges

`GET /admin/dashboard` accepts `range=1d|3d|7d|custom`. A custom range requires
inclusive Unix-millisecond `from` and `to` values with `from < to`; no dashboard
range may exceed 90 days. The backend aggregates all retained usage rows in the
selected window before returning request/error series, token series, model
usage, and `rangeSummary`. These values are not derived from the current logs
page.

`rangeDataSource` reports `persisted-usage`, `process-metrics-estimate`, or
`empty`; `rangeDataEstimated` explicitly marks the process-metrics fallback.
`rangeDataAtRetentionLimit=true` means the retained log has reached
`MODELPORT_USAGE_LOG_LIMIT`, so the requested window may be incomplete even
though aggregation covers the full retained set. The default retention is
5,000 rows.

### Request Logs And Latency

`GET /admin/logs` performs filtering and pagination on the server. Query names
are camelCase:

| Parameter | Semantics |
| --- | --- |
| `page` | Default `1`; `0` is normalized to `1`. |
| `pageSize` | Default `20`; clamped to `1..500`. |
| `status` | Exact `success`, `error`, or `timeout`. |
| `provider`, `userId`, `apiKeyId` | Exact match. |
| `model` | Case-insensitive substring across requested and resolved model. |
| `dateFrom`, `dateTo` | Inclusive Unix epoch milliseconds. |
| `search` | Case-insensitive substring across log/request IDs, provider/channel, model, user/key/group/team labels, error/detail, and request path. |
| `username`, `group` | Case-insensitive substring; `group` checks both group fields. |
| `stream` | Exact `stream` or `non-stream`. |

Each textual filter other than `search` is limited to 256 characters;
`search` is limited to 512. Oversized values return `invalid_request`.

Results are ordered by timestamp descending. The response is
`{ logs, total, summary }`: `total` is the filtered count before pagination,
and `summary` is calculated over that complete filtered set rather than the
current page. Summary fields are `totalRequests`, `successRequests`, the four
token classes, `totalTokens`, `totalCostEstimate`, `rpm`, and `tpm`.

Each row's `billingMode` records usage provenance. `upstream-returned` means a
completed adapter path exposed Provider-reported token usage; `local-estimate`
means ModelPort used its request-based estimate. Both remain operational cost
estimates because ModelPort applies its local pricing table. When an
attempt-level preflight rejection produces a row, it has `local-estimate`
provenance but zero tokens/cost and is not charged to quota or the spend ledger.
Normal live streams commonly remain
`local-estimate`; non-stream and buffered-stream paths can be
`upstream-returned` when the Provider supplies usage.

Persisted `status` is `success` for a successful pre-response attempt,
`timeout` when a transport error before the downstream response is classified
as timed out, and `error` for other failures. A live-stream idle timeout after
HTTP/SSE headers still cannot revise the already accepted log row, so it is not
guaranteed to appear as `timeout` in this endpoint.

`statusCode` is the gateway's client-facing HTTP mapping for that pre-response
outcome, not a separately captured raw-upstream field. Upstream statuses are
retained when the error mapping permits (for example 401), while a transport
failure maps to 502 rather than an undifferentiated 500. Provider attempt
outcome tracking uses the same mapped status.

Malformed/negative numeric queries return HTTP 400. Unsupported `status` or
`stream`, and `dateFrom > dateTo`, return the normal `invalid_request` envelope.
`GET /admin/logs/{id}` returns one log object directly; an unknown ID returns
the standard HTTP 404 envelope with `error.code="not_found"`.

When no persisted usage rows exist, the list can contain aggregate fallback
rows reconstructed from process metrics. They are not individual request
records and have no persisted request ID. `/admin/latency` calculates
nearest-rank percentiles from retained usage rows when available and marks
`percentilesEstimated=false`; without samples it returns an aggregate
metrics-derived estimate with `sampleCount=0` and
`percentilesEstimated=true`.

Write requests require `X-ModelPort-CSRF: 1` plus a valid session. When the
browser supplies Origin or Referer, it must be same-origin or listed in
`MODELPORT_ALLOWED_ORIGINS`.

The backend does not expose a general CORS policy. Serve the dashboard and API
through one origin (the Docker Nginx image does this). Merely setting
`VITE_API_BASE_URL` to a different browser origin is insufficient without a
separate trusted CORS/reverse-proxy design.
