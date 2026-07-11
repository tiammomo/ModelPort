# Architecture

ModelPort is a single-process Rust gateway with a separate React dashboard. It
accepts an Anthropic-compatible client contract and routes requests to either an
Anthropic-compatible or OpenAI-compatible provider. It is designed for one
trusted host or a small trusted network, not public multi-tenant SaaS.

## Components

```text
Claude Code / VS Code Claude / API client
                    |
                    | Anthropic Messages API
                    v
              ModelPort (Axum)
        auth -> validation -> model resolution
        -> rate/policy/quota -> credential selection
        -> provider URL guard -> protocol adapter
                    |
          +---------+----------+
          |                    |
 Anthropic-compatible   OpenAI-compatible
          |                    |
          +---------+----------+
                    |
      response/SSE mapping, metrics, usage log

React dashboard -> /admin/* cookie-session control plane
PostgreSQL or JSON files -> auth and control documents
```

The backend has no general internal message IR. Anthropic request types and
focused conversion functions are the current boundary. UI panels labelled as a
request pipeline are explanatory views, not stored raw protocol payloads.

## Technical Core

The core is a bounded protocol-and-policy pipeline, not a generic model
platform. The table below separates the shipped mechanism from the boundary an
operator must still account for.

| Core | Implemented mechanism | Explicit boundary |
| --- | --- | --- |
| Protocol adaptation | Anthropic Messages is the client contract. Anthropic-compatible requests can pass through; OpenAI-compatible requests, responses, SSE deltas, and common Tool Use events use focused conversion code. Parsers enforce frame and stream limits and require protocol completion signals. | There is no complete provider-neutral Message/Tool IR. Configured adapters and models are not proof of real-upstream compatibility. |
| Model routing and fallback | Resolution covers `provider:model`, recursive aliases with a depth guard, exact model matches, prefixes, and the default Provider. The attempt plan skips cooling Providers while an eligible alternative exists and only retries transport/protocol failures, 429, and 5xx against a Provider that accepts the requested or resolved model. | If no non-cooling route remains, the primary is retained as a final attempt. Fallback does not promise semantic model equivalence. Once local live-stream headers are sent, a later failure cannot replay on another Provider. |
| Identity, policy, and budget | The data plane accepts a control-plane API key or the explicitly allowed legacy token. API-key model/Provider/IP policy, user quota, API-key/team rolling spend, credential availability, capability gates, and Provider/model limits are checked before an attempt is sent. Only a sent attempt is chargeable. | Quota/spend checks are preflight guards, not transactional reservations. Rate limits, stream permits, and sessions are process-local, so this is not distributed hard enforcement. |
| Credential and Provider lifecycle | Provider credentials are environment-backed. Pool selection supports manual, failover, and round-robin behavior; outcomes feed credential/Provider health and cooldown state; unusable managed pools fail closed. | Health is operational state, not an external SLA. A configured credential or successful synthetic test does not establish every model, Tool Use, or stream path as verified. |
| Control-plane state | Base environment/TOML configuration is combined with persisted routing overrides. Auth and control state use the same two logical documents in JSON-file or PostgreSQL mode. Low-frequency authorization/routing mutations restore their in-memory snapshot if persistence fails. | Writes replace whole logical documents; usage telemetry after an upstream response is best effort. PostgreSQL log filtering is still in-memory over retained rows and native database TLS is not implemented. |
| Security and observability | Browser writes require a session and CSRF token, with Origin/Referer checks when present. Trusted-proxy parsing, remote-Provider HTTPS defaults, URL guards, disabled redirects, bounded bodies/SSE, request IDs, Prometheus metrics, retained logs, and source-labelled dashboard aggregation provide operational evidence. | URL guards do not pin or revalidate DNS answers. `upstream-returned` identifies usage provenance, not invoice accuracy; `local-estimate` is heuristic, and ordinary live streams may lack final usage/success evidence. |

The detailed lifecycle and failure semantics below are normative. Provider and
Tool Use verification evidence is maintained separately in the
[Provider Matrix](PROVIDER_MATRIX.md) and
[Tool Use Compatibility](TOOL_USE_COMPATIBILITY.md).

## Backend Boundaries

- `src/main.rs`: minimal binary entry that delegates to the library.
- `src/lib.rs`: library module graph, tracing initialization, and CLI/server
  dispatch.
- `src/cli.rs`: command parsing, configuration validation, and complete backup
  export/validate/restore.
- `src/server.rs`: runtime state construction, listener, and graceful shutdown.
- `src/config.rs`: base provider configuration, environment/TOML loading,
  validation, aliases, and model resolution.
- `src/routes.rs` and `src/routes/`: router assembly, security helpers, public
  client routes, operations routes, and control-plane endpoints.
- `src/providers/`: Anthropic pass-through and OpenAI-compatible request,
  response, and SSE conversion.
- `src/http.rs`: the upstream HTTP client, bounded response reading, SSE frame
  parsing, timeouts, redirect policy, and upstream error redaction.
- `src/auth.rs`: dashboard users, Argon2 password hashes, per-username login
  lockout, timing-mitigation work, in-memory sessions, and session cookies.
- `src/control.rs`: API keys, teams, policies, quotas, usage logs, provider
  overrides, credential pools, health, tests, and audit events.
- `src/storage.rs`: JSON-file or PostgreSQL persistence for the two state
  documents.
- `src/metrics.rs`: process-local Prometheus counters.
- `dashboard/`: the browser control plane. It consumes `/admin/*`; it is not a
  second source of routing truth.

## Request Lifecycle

For `POST /v1/messages`, the current order is:

1. Axum applies the global body-size and concurrency layers and assigns an
   `x-request-id` when one was not supplied. Both built-in protocol adapters
   forward that opaque value upstream; it is correlation metadata, not a
   trace/span parent.
2. The client is authenticated with a control-plane API key or, when allowed,
   the legacy router token.
3. The Anthropic request and Tool Use structure are validated. `max_tokens` is
   mandatory, must be positive, and cannot exceed the configured output cap.
4. The model is resolved from `provider:model`, an alias, an exact model, a
   prefix, or the default provider.
5. Process-local rate limits run for global, identity, IP, provider, and model
   dimensions.
6. A streaming request acquires a process-local stream permit or returns 429
   before an upstream attempt. Then a route-attempt list is built;
   cooling-down providers are skipped while an eligible alternative exists; if
   every eligible route is cooling, the primary remains as the final attempt.
7. For each attempt, ModelPort selects a provider credential, checks API-key
   policy and quota, validates the provider URL and capability gate, then calls
   the protocol adapter. `failover` and `round_robin` pools with no usable
   credential fail closed for that Provider; only `manual` can retain an
   explicitly selected non-disabled credential.
8. Non-stream responses are mapped before returning. Usage, provider outcome,
   metrics, and the request log are recorded. Quota and spend state changes only
   after an upstream attempt was actually sent; an attempt-level preflight
   rejection that reaches this recorder is logged with zero usage and no charge.
9. Stream responses are mapped as Anthropic-style SSE.

Automatic cross-provider fallback is limited to transport failures, upstream
protocol failures, HTTP 429, and HTTP 5xx, and only to a configured provider
that can accept the requested or resolved model. It is not a semantic guarantee
that the fallback model behaves identically.

Completed paths that expose Provider usage are labelled
`billingMode="upstream-returned"`; paths that use the request heuristic are
`billingMode="local-estimate"`. This is provenance, not an invoice guarantee:
cost still comes from the local pricing table, and ordinary live streams may
not expose their final Provider usage.

## Configuration And Runtime Overrides

Base configuration comes from environment defaults or a TOML file. The control
plane can overlay provider records, model inventory, aliases, default provider,
and provider order. See [Configuration](CONFIGURATION.md) for the exact source
and reload rules.

Dashboard changes to control-plane records are persisted. They do not rewrite
`.env` or `config.toml`.

Provider update serialization distinguishes “unchanged” from “clear”. Omitting
`apiKeyEnv` preserves the current value, while `clearApiKeyEnv=true` clears it;
combining the clear flag with a non-empty value is invalid. This explicit flag
avoids treating an empty browser field as an ambiguous partial update.

## State And Persistence

There are two logical JSON documents:

| Namespace | Contents |
| --- | --- |
| `auth` | Users and password hashes. Sessions and failed-login counters are process-local. |
| `control` | Teams, API-key hashes, policy, quota, usage, audit, routing overrides, credentials metadata, and provider health. |

With `MODELPORT_DATABASE_URL`, the documents are stored as two `jsonb` rows in
`modelport_state`. Without it, they are JSON files. Writes are currently
synchronous and replace the complete logical document; file mode uses a
temporary file plus rename to avoid exposing a partially written JSON file.
The PostgreSQL client currently uses `NoTls`; the default Compose private bridge
is the deployment assumption, not encrypted transport support. Remote state
storage must stay on an independently protected trusted path until native TLS
is implemented.
This keeps the design simple, but write amplification and request latency grow
with retained state. It is a known scaling limit, not a transactional
event-store design. Request-log filters, summaries, and pagination are also
computed in memory over retained rows; PostgreSQL mode does not issue an
indexed row-level log query.

Low-frequency identity, policy, quota, routing, Provider, and credential
mutations snapshot the in-memory document before writing. A failed write
restores that snapshot, returns an error, and makes readiness fail closed until
a later complete write succeeds; an HTTP 5xx therefore cannot leave a routing
or authorization change active only in the current process. Request usage and
other post-upstream telemetry remain best effort so a persistence failure does
not replace a response already paid for and received from an upstream.

CLI backup load validates both document schemas and critical auth invariants
before restore. Restore saves the previous values but replaces auth and control
sequentially; there is no atomic transaction spanning the two logical
documents.

## Identity And Budget Boundaries

An API key must be created for a real active auth user, and the server stores
the canonical username rather than trusting request metadata. Every data-plane
authentication checks that the owner still exists and is active. Disabling or
suspending a user revokes that user's keys and removes the user's quota rows.

Console roles intentionally differ: administrators manage all key policy and
lifecycle fields; normal users can read owned keys and change only their name
and group, revoke them, or delete them; viewers are read-only. A user cannot
create or restore keys or edit team/model/provider/IP/expiry/spend policy.

User quota records use UTC calendar periods: a day begins at 00:00 UTC, a week
at Monday 00:00 UTC, and a month on its first day at 00:00 UTC. API-key and team
spend policy is separate and uses rolling 5-hour, 24-hour, 7-day, and 30-day
windows. The persisted `rateLimited` name is retained for compatibility, but it
enables periodic spend limits rather than request-rate limiting.

Rolling spend is kept in hourly buckets. The oldest bucket that intersects a
window is included in full, so the check is intentionally conservative near the
boundary and can include almost one extra hour. User quota checks and spend
checks are preflight guards rather than transactional reservations; concurrent
requests can still overshoot a tight cap.

A team cannot be deleted while any API key references it. This dependency
check prevents deletion from silently broadening access by removing team
policy; operators must reassign or delete referencing keys first.

## Dashboard Aggregation

Dashboard trend queries are aggregated on the server over the complete retained
usage set in the requested window, not over the current paginated logs page.
The response includes request/error and token buckets, model usage, and a range
summary. Ranges are bounded to 90 days.

The backend marks the source as `persisted-usage`,
`process-metrics-estimate`, or `empty`, and separately exposes whether the
result is estimated and whether the usage store has reached
`MODELPORT_USAGE_LOG_LIMIT`. Reaching the retention limit means older rows may
have been evicted; “full window” therefore means the full retained set, not
unbounded historical storage.

## Streaming Boundary

The SSE adapter handles split frames, Anthropic events, OpenAI deltas, Tool Use
arguments, and configured replay deduplication. For OpenAI-compatible Tool Use,
`streaming_arguments="delta"` preserves incremental fragments, while
`cumulative` and `best_effort` enable argument replay deduplication and recovery
of the best complete JSON object available at stream completion. Text replay is
separate: `fidelity_mode="stability"` alone does not rewrite output, so
`deduplicate_stream_text` or `buffer_stream_text` must be enabled explicitly.
On the normal live-stream path, an upstream failure after local response headers
can only be represented as an SSE `event: error`.

ModelPort now establishes the upstream connection and checks its initial HTTP
status before returning the local SSE response, so connect and pre-header HTTP
failures can participate in normal fallback. Completing the stream remains a
separate phase: a stream is recorded as accepted before its final success,
token usage, and duration are known. Later stream failures do not currently
participate in cross-provider fallback. Operators must inspect the SSE body and
provider logs, not only the initial HTTP status or success counter. A transport
timeout before local response creation is persisted as `status=timeout`; an
idle timeout after live-stream headers cannot rewrite the accepted log row.

Handshake validation requires a 2xx response other than 204 and a
`text/event-stream` media type before local headers. Missing and explicit
non-SSE content types are rejected; media-type parameters such as `charset` are
valid. Non-2xx and wrong-content-type error bodies are constrained by the
response byte limit, the total request timeout, and the stream-idle timeout,
then redacted before they become an error eligible for fallback.

Native Anthropic streams must reach `message_stop`. OpenAI-compatible streams
must reach `[DONE]` or a `finish_reason`, after which ModelPort emits
`message_stop`. EOF without the protocol's termination signal is an upstream
protocol error rather than a successful partial response. Once local HTTP 200
headers exist, this is represented by SSE `event: error` and cannot restart on
another Provider.

The general request timeout covers the entire non-stream exchange but only the
SSE `send()`/response-header handshake. Once the live response body starts,
there is no fixed total-duration timer. The parser instead applies a resettable
per-chunk idle timeout plus line, event, and total raw-stream byte ceilings. An
active stream can therefore remain open beyond the request-timeout duration.

The stream permit count comes from `MODELPORT_MAX_CONCURRENT_STREAMS`, defaulting
to the effective general request-concurrency limit. Unlike the normal handler
future, the permit is moved into the returned body and survives until that body
finishes or is dropped. This makes downstream slow readers visible to capacity
control; an exhausted semaphore returns HTTP 429 with `Retry-After: 1` and no
quota/spend charge.

`buffer_stream_text=true` is a distinct compatibility path. ModelPort sends a
non-stream OpenAI-compatible request, awaits and validates the complete
response, converts it to an Anthropic message, and only then creates locally
chunked SSE. Upstream HTTP/protocol failures therefore remain normal HTTP errors
and can fallback before local headers. When the upstream reports usage, the
adapter attaches it to the internal response so metrics, quota spend, and the
request log use those token values instead of the request estimate. The tradeoff
is full-generation time to first byte; client cancellation after local SSE
starts cannot cancel an upstream generation that already finished, and local
delivery completion is still not observed.

## Security Boundaries

- Data-plane credentials and dashboard sessions are separate.
- Admin Argon2 work runs on blocking workers outside the auth-state mutex. A
  process-local four-hash gate returns 429 after a five-second queue wait;
  unknown/disabled-user attempts remain in the expensive hash class, and the
  five-attempt/15-minute username lockout remains process-local.
- Dashboard writes require a session, `X-ModelPort-CSRF`, and an allowed
  Origin/Referer when the browser sends one.
- The backend does not currently provide general cross-origin CORS headers.
  Deploy the dashboard and API behind one origin.
- Forwarded client IP headers are accepted only from configured trusted peers.
  ModelPort walks `X-Forwarded-For` from the connected peer right-to-left,
  discards only explicitly trusted proxy hops, and selects the first untrusted
  address. It never trusts an attacker-supplied leftmost value merely because
  the nearest peer is a proxy.
- Provider URLs reject userinfo, query strings, fragments, disallowed schemes,
  and literal private/link-local/metadata IPs by default. Credentials are sent
  from environment-backed header configuration rather than embedded in the
  URL. Hostname DNS resolution is not pinned or revalidated against private
  addresses, so this is not a complete DNS-rebinding defense.
- Non-local/non-custom Providers require HTTPS by default. The explicit
  `MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` escape hatch is only for a trusted
  internal network because HTTP exposes Provider API keys and prompt/response
  content in plaintext. Local/custom runtime classes retain HTTP support for
  loopback and controlled local integration.
- Upstream redirects are disabled and response/SSE byte counts are bounded.
- Upstream error redaction covers common secret fields and token patterns; it is
  defense in depth, not a reason to log raw secrets or payloads.

See [Security Policy](../SECURITY.md) and [Operations](OPERATIONS.md).

## Deliberate Non-Goals

- Model inference inside the gateway.
- A chat client or prompt-history product.
- Enterprise IAM, OIDC/SSO, public multi-tenancy, or exact billing.
- Distributed rate limiting or multi-instance coordination.
- A complete provider-neutral Tool/Message IR.
- Image and Responses APIs in the current text gateway.
