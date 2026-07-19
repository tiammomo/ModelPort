# API Reference

ModelPort exposes Anthropic Messages and a scoped OpenAI Chat Completions data
plane plus a dashboard control plane. The default backend origin is
`http://127.0.0.1:38082`.

## Public Endpoints

| Endpoint | Authentication | Semantics |
| --- | --- | --- |
| `GET /livez` | none | Process liveness only. |
| `GET /health` | optional | Minimal public body; router authentication adds provider and storage diagnostics. |
| `GET /admin/auth/methods` | none | Advertises password availability and the optional OIDC console sign-in label/start path. It does not establish a session. |
| `GET /admin/auth/oidc/start` | none | Creates single-use OIDC state, nonce, PKCE values, and an HttpOnly browser-flow cookie, then redirects to the configured identity provider. |
| `GET /admin/auth/oidc/callback` | none | Requires the matching browser-flow cookie, validates and consumes the OIDC callback, resolves the local user, issues the ModelPort console cookie, and redirects to a local return path. |
| `GET /readyz` | router/API key | Auth/control and normalized-ledger readiness plus detailed diagnostics; Provider degradation does not fail it. |
| `GET /v1/models` | router/API key | Configured, visible model and alias catalog. Visibility does not prove upstream health. |
| `POST /v1/messages` | router/API key | Anthropic-compatible Messages request. |
| `POST /v1/messages/count_tokens` | router/API key | Exact Provider tokenizer count when the selected Provider enables the capability. |
| `POST /v1/chat/completions` | router/API key | Scoped OpenAI-compatible Chat Completions request. |
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
curl -sS http://127.0.0.1:38082/v1/messages \
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

## Count Tokens

```bash
curl -sS http://127.0.0.1:38082/v1/messages/count_tokens \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H 'anthropic-version: 2023-06-01' \
  -H 'content-type: application/json' \
  -d '{
    "model": "qwen3.5-code",
    "messages": [{"role":"user","content":"你好，world"}]
  }'
```

The response is `{"input_tokens":N}`. ModelPort validates the same Anthropic
input and Tool Use guardrails as Messages, resolves aliases, applies API-key and
team model/Provider/IP policy with zero estimated usage, selects the active
Provider credential, and forwards the body only when that Provider explicitly
configures `token_counting.mode="anthropic"`. The count comes from the selected
upstream tokenizer and chat template; ModelPort does not use its characters/4
usage estimate here. There is deliberately no cross-provider fallback because
a count from another tokenizer would be misleading.

## Chat Completions

```bash
curl -sS http://127.0.0.1:38082/v1/chat/completions \
  -H "Authorization: Bearer $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
    "model": "deepseek-v4-flash",
    "messages": [{"role":"user","content":"Reply exactly: OK"}]
  }'
```

This compatibility surface currently accepts text-only `system`, `developer`,
`user`, `assistant`, and `tool` messages; OpenAI function tools and tool calls;
`temperature`, `top_p`, penalties, `seed`, `stop`, `tool_choice`,
`parallel_tool_calls`, text `response_format`, `stream_options.include_usage`,
and `n=1`. `max_completion_tokens` or legacy `max_tokens` is optional; when
neither is supplied, ModelPort uses 4096 only for local estimation and for an
Anthropic Provider that requires an explicit output limit.

The endpoint deliberately rejects fields outside this documented slice,
including current image/audio content, rather than silently dropping semantics.
Routing to an Anthropic Provider also rejects OpenAI-only penalties/seed,
non-text response formats, strict function schemas, or system/developer
messages interspersed after conversation start because those cannot be
preserved faithfully. A successful response uses the OpenAI `chat.completion`
shape whether the selected Provider is OpenAI-compatible or
Anthropic-compatible.

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

Configured provider models are advertised in both forms, with the stable
provider-qualified ID first (for example
`local_qwen:qwen3.5-9b-q5km`) and the legacy unqualified model ID second (for
example `qwen3.5-9b-q5km`). Long-lived clients should persist the qualified ID
because it remains unambiguous when two providers expose the same upstream
model name. Aliases remain separate catalog entries and resolve through the
same deterministic routing rules.

## Streaming

For `/v1/messages`, set `"stream": true` to receive Anthropic-style
server-sent events. Common events are:

```text
message_start
content_block_start
content_block_delta
content_block_stop
message_delta
message_stop
error
```

For `/v1/chat/completions`, `"stream": true` returns OpenAI
`chat.completion.chunk` data events followed by `data: [DONE]`. When
`stream_options.include_usage=true`, ModelPort preserves an OpenAI-compatible
final empty-`choices` usage chunk or synthesizes it from Anthropic stream usage.
If the stream is interrupted, that final usage chunk may be unavailable; the
terminal request log then retains the best evidence available.

OpenAI-compatible text deltas and tool-call deltas are converted into
Anthropic content blocks for `/v1/messages`. A failure after response headers
have been sent is reported as Anthropic `event: error` or an OpenAI data event
with an `error` object while the HTTP status remains 200. Consumers must inspect
the event stream rather than treating the initial status as completion.

The transport establishes the upstream response and checks its initial status
before sending the local stream, allowing pre-header connect/HTTP failures to
fallback. It also enforces per-line, per-event, total-stream and idle limits.
The request log, message metrics, and Provider health are finalized only when
the body completes, fails, times out, or is dropped. Client cancellation is
recorded with status code `499`; see
[Architecture](ARCHITECTURE.md#streaming-boundary).

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
then renders the terminal signal required by the originating client protocol.
EOF without a required termination signal is an upstream-protocol failure. If
local HTTP 200 headers have already been sent, that failure is delivered inside
SSE and cannot trigger cross-provider fallback.

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

## Idempotent Retry Claims

Both inference endpoints accept an optional `Idempotency-Key` header containing
1–200 visible ASCII characters without whitespace. The claim is scoped by
organization, project, and environment and is written atomically before any
Provider attempt. ModelPort stores only SHA-256 of the key plus a protocol/body
fingerprint; do not put credentials or personal data in the key.

Reusing a key has these outcomes:

- the original request is still running: HTTP 409 `idempotency_conflict`;
- the original request is terminal with the same fingerprint: HTTP 409 because
  response replay is not implemented in this release;
- the body or client protocol differs: HTTP 409 `idempotency_conflict`.

This provides an at-most-once Provider-call boundary, not transparent response
replay. A client that receives an uncertain network result should query its own
request records or retain the 409 as evidence; using a new key authorizes a new
Provider call. `x-request-id` remains correlation-only and does not claim
idempotency. The claim is durable and multi-instance-safe with the PostgreSQL
ledger; database-free development mode keeps it only in process memory.

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
`quota_exceeded`, `rate_limit_error`, `invalid_request_error` (including the
HTTP 409 code `idempotency_conflict`),
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

Password login is `POST /admin/auth/login`. The three public OIDC/capability
entry points are `GET /admin/auth/methods`, `GET /admin/auth/oidc/start`, and
`GET /admin/auth/oidc/callback`, as listed above. All successful human sign-in
methods issue the same HttpOnly, SameSite=Lax ModelPort console cookie. Use
`MODELPORT_ADMIN_COOKIE_SECURE=1` behind HTTPS; other normal administrator
routes require that session.

This console identity boundary is separate from the data plane. Neither an
OIDC token nor the console cookie authenticates `/v1/messages` or
`/v1/chat/completions`; clients use a ModelPort API key (or the explicitly
enabled legacy router token), while upstream Provider credentials remain on
the ModelPort server. See [OIDC Console Sign-In](OIDC.md).

Route groups include:

- `/admin/auth/*`: capability discovery, password/OIDC sign-in, logout, and
  current session.
- `/admin/dashboard`, `/admin/logs`, `/admin/logs/{id}`, `/admin/latency`:
  operational views.
- `/admin/enterprise/overview`, `/admin/enterprise/requests`,
  `/admin/enterprise/requests/{ledger_id}`, and `/admin/enterprise/budget*`:
  administrator-only normalized request/attempt and transactional-budget
  evidence views.
- `/admin/users`, `/admin/api-keys`, `/admin/teams`, `/admin/quotas`: identity
  and policy.
- `/admin/providers`, `/admin/aliases`: provider lifecycle, credentials, model
  inventory, routes, and the administrator-only live DeepSeek balance check.
- `/admin/settings`, `/admin/settings/reload-config`: runtime view, default
  provider/order updates, and base-config reload.
- `GET /admin/audit`, `POST /admin/backup`: audit events and a redacted,
  non-restorable diagnostic snapshot.

### Enterprise Ledger Views

`GET /admin/enterprise/overview` reports the active ledger backend, its
credential-redacted location, lease/reconciliation intervals, lifecycle
counts, idempotent-request count, active/expired leases, unreconciled rows,
cost microunits, and organization/project/environment cardinality.

`GET /admin/enterprise/requests` accepts `page` (default 1), `pageSize`
(default 25, maximum 100), and optional exact `state`, `protocol`,
`organizationId`, `projectId`, and `environmentId` filters. `search` performs a
bounded correlation search over ledger/request IDs, principal, model, tenant,
terminal reason, and error. The response contains `requests`, `total`, `page`,
and `pageSize`. `GET /admin/enterprise/requests/{ledger_id}` returns one request
plus its ordered Provider attempts or a standard 404.

These endpoints require an administrator dashboard session. They expose only
whether a request claimed an idempotency key; the raw key, stored hash, request
fingerprint, prompts, and Provider bodies are never returned. Memory mode uses
the same response contract for local development, but its rows are neither
durable nor multi-instance safe.

### Enterprise Transactional Budget

`GET /admin/enterprise/budget` reads one USD budget account and its 50 most
recent evidence events. Supply `organizationId`, `projectId`, and
`environmentId` together; omitting all three selects
`org_local/prj_default/env_default`. Partial scope is rejected.

`PUT /admin/enterprise/budget` sets the hard limit. It requires an administrator
session and `X-ModelPort-CSRF` like every dashboard write:

```json
{
  "organizationId": "org_local",
  "projectId": "prj_default",
  "environmentId": "env_default",
  "limitMicrounits": 100000000,
  "unlimited": false
}
```

Set `unlimited=true` and omit `limitMicrounits` to remove the limit. USD money
is transported as integer microunits; the dashboard accepts up to six decimal
places and converts without floating-point arithmetic.

`POST /admin/enterprise/budget/adjustments` records a signed settlement
adjustment. `deltaMicrounits` cannot be zero or make settled spend negative;
`reason` and `evidenceReference` are mandatory and limited to 500 characters:

```json
{
  "organizationId": "org_local",
  "projectId": "prj_default",
  "environmentId": "env_default",
  "deltaMicrounits": -2500000,
  "reason": "Provider invoice credit",
  "evidenceReference": "invoice://2026-07/credit-42"
}
```

Before Provider egress, every attempt atomically reserves its maximum local
estimate against `settled + reserved`. The terminal path moves that reservation
to actual/provider-reported settlement; an expired lease releases it as
unreconciled. PostgreSQL performs each transition in one transaction and the
event table rejects updates and deletes. Memory mode keeps the same contract
for a single process, but it is not durable or distributed.

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

`POST /admin/providers/deepseek/balance` performs an administrator-only,
CSRF-protected live read against DeepSeek's official `GET /user/balance`
endpoint using the active server-side provider credential. The response
contains `isAvailable`, CNY/USD `balanceInfos`, `checkedAt`,
`managementScope=read-monitor-alert`, and
`billingAuthority=deepseek-console`. It never returns the upstream API key.
ModelPort may display and alert on the balance, but recharge, refunds, invoices,
and authoritative settlement remain in the DeepSeek console. Local token-cost
and transactional-budget ledgers remain independent estimates/evidence and do
not overwrite the provider invoice.

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
| `search` | Case-insensitive substring across log/request/attempt IDs, provider/channel, model, user/key/group/team labels, terminal reason, error/detail, and request path. |
| `username`, `group` | Case-insensitive substring; `group` checks both group fields. |
| `stream` | Exact `stream` or `non-stream`. |
| `toolUse` | Exact `requested` or `not-requested`; classifies the workflow without retaining tool arguments. |

Each textual filter other than `search` is limited to 256 characters;
`search` is limited to 512. Oversized values return `invalid_request`.

Results are ordered by timestamp descending. The response is
`{ logs, total, summary }`: `total` is the filtered count before pagination,
and `summary` is calculated over that complete filtered set rather than the
current page. Summary fields are `totalRequests`, `successRequests`, the four
token classes, `totalTokens`, `totalCostEstimate`, `rpm`, and `tpm`.
`toolUseRequests` and `toolUseSuccessRequests` provide the corresponding
workflow counts. Rows recorded before this field was introduced deserialize as
`toolUseRequested=false`; use a deployment timestamp when calculating coverage.

Each row's `billingMode` records usage provenance. `upstream-returned` means a
completed adapter path exposed Provider-reported token usage; `local-estimate`
means ModelPort used its request-based estimate. Both remain operational cost
estimates because ModelPort applies its local pricing table. When an
attempt-level preflight rejection produces a row, it has `local-estimate`
provenance but zero tokens/cost and is not charged to quota or the spend ledger.
Live streams become `upstream-returned` when their Anthropic/OpenAI events
contain recognized usage; otherwise they remain `local-estimate`. Non-stream
and buffered-stream paths can also be `upstream-returned` when the Provider
supplies usage. `clientProtocol` records `anthropic-messages` or
`openai-chat-completions` independently from the selected Provider `protocol`,
and `requestPath` records the public edge used.

Authenticated inference clients may set `x-modelport-traffic-class` to one of
`business`, `synthetic`, or `diagnostic`; omission defaults to `business` and
any other value returns 400. The bounded value is exposed as `trafficClass` so
deployment dashboards can exclude acceptance traffic without provider-name
heuristics. This is a caller assertion, not an authorization boundary.

`toolRepairAttempted` and `toolRepairRecovered` are aggregate booleans. They
record the opt-in, one-attempt strict-schema repair lifecycle without retaining
tool names, arguments, paths, results, or Provider bodies. A recovered request
uses `billingMode=upstream-returned+tool-repair`; its request-level token and
cost totals include both Provider responses, while each attempt remains visible
independently in the enterprise ledger.

`errorMessage` is durable audit telemetry, not a copy of the client error
response. It retains only a coarse category (for example timeout, rate limit,
authentication, Tool protocol, or generic failure) plus an explicit redaction
marker. Request values, validation paths, Provider bodies, URLs, and storage
diagnostics are removed before usage, ledger, and Provider-health persistence.

`toolUseRequested` is true when the request declares or selects tools, or
continues an existing tool call/result exchange. It does not prove that the
model emitted a new call, and ModelPort still does not retain tool names,
arguments, results, messages, or Provider bodies in the usage log.

`toolOutcome` is an aggregate-only workflow classification:
`unknown_legacy`, `not_requested`, `completed`, `client_cancelled`, `timeout`,
`protocol_error`, or `upstream_or_delivery_error`. It is derived from the protocol terminal state
and sanitized error category; it never contains tool names, arguments, results,
or raw Provider content. Older rows deserialize as `unknown_legacy` so they do
not falsely imply that the historical request omitted tools.

Persisted `status` is `success` only after a non-stream response succeeds or a
stream reaches its protocol terminal signal and downstream body EOF. `timeout`
is used for classified pre-response or live-stream upstream timeouts, and
`error` for other failures and downstream cancellation. `requestId` identifies
the client request; `attemptId` identifies the last upstream attempt represented
by the row.

`terminalReason` distinguishes `completed`, pre-response failure/timeout,
`upstream_error`, `upstream_timeout`, `delivery_error`, and downstream
cancellation. `statusCode` is the gateway's effective outcome mapping, not a
separately captured raw-upstream field. Upstream statuses are retained when the
error mapping permits (for example 401); stream protocol/delivery failures map
to 502, upstream timeouts to 504, and downstream cancellation to 499. Provider
health uses the upstream outcome, so a downstream cancellation after known
upstream completion does not incorrectly penalize the Provider.

Malformed/negative numeric queries return HTTP 400. Unsupported `status`,
`stream`, or `toolUse`, and `dateFrom > dateTo`, return the normal
`invalid_request` envelope.
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
