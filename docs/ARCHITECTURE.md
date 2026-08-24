# Architecture

ModelPort is a single-process Rust gateway with a separate React dashboard. Its
Anthropic Messages and scoped OpenAI Chat Completions client edges route to
Anthropic-compatible or OpenAI-compatible Providers through one governance
pipeline. It is designed for one trusted host or a small trusted network, not
public multi-tenant SaaS.

## Components

```text
Claude Code / OpenAI SDK / API client
                    |
       Anthropic Messages or OpenAI Chat
                    v
              ModelPort (Axum)
       edge parse -> typed Exchange IR
        -> auth -> validation -> model resolution
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

React dashboard -> local password or OIDC Authorization Code + PKCE
                -> /admin/* ModelPort cookie-session control plane
PostgreSQL -> request, attempt, usage, quota/spend, budget, and audit facts
JSON/PostgreSQL document -> low-frequency auth and control configuration
```

## Product Boundary: Current Gateway And Target Control Plane

The diagram above is the implemented v0.1.x data plane. ModelPort is evolving
from that governed gateway into an independent hybrid model and GPU control
plane, but documentation and APIs must not present target resources as shipped
behavior. [ADR-0007](adr/0007-independent-model-and-gpu-control-plane.md)
defines the boundary and delivery order.
The [Post-Beta AI Gateway Strategy](AI_GATEWAY_STRATEGY.md) maps gateway parity,
control/data-plane ownership, evidence gates, and cache/guardrail boundaries
onto that resource model.

The target keeps inference engines outside ModelPort:

```text
Client/Harness
      |
      v
ModelPort protocol, policy, routing, ledger, and control APIs
      |
      +---------------- Hosted Provider API
      |
      +-- Deployment -- Runtime Adapter -- external inference runtime
                             |
                       Compute Node / GPU
```

ModelPort owns desired state, observed inventory, governance, reconciliation,
and durable evidence. An external runtime owns device-specific execution and
runtime-native caches. The Dashboard calls ModelPort control APIs; it never
talks directly to a runtime or becomes another source of desired state.

### Stable resource model

| Resource | Owner and relationship |
| --- | --- |
| Client/Harness | Calls a supported ModelPort protocol edge. Claude Code, Codex CLI, Qwen Code, SDKs, and internal applications are clients, not Provider types. DeepSeek remains a Provider/model family unless a distinct caller contract is introduced and verified. |
| Provider | Governs an upstream connectivity, credential, trust, and commercial boundary. It can reference a hosted API or a Deployment-backed endpoint. |
| Model | Describes a catalog identity, capabilities, limits, compatibility, and optional pricing. Catalog presence does not imply deployment or route eligibility. |
| Runtime Adapter | Implements a versioned contract for inventory and bounded lifecycle operations against an external inference runtime. |
| Compute Node/GPU | Records host and device capacity, health, freshness, labels, and provenance without treating observations as desired state. |
| Deployment | Binds a Model, Runtime Adapter, compute allocation, endpoint, and desired/observed lifecycle. |
| Route | Selects eligible Provider/model or Deployment-backed candidates and persists the policy and decision evidence. |

Hosted and local execution share Model, Provider, Route, policy, health, and
evidence concepts, but keep distinct operational facts. Hosted APIs have
remote credentials, rate limits, regions, and Provider-reported usage. Local
Deployments have artifacts, runtime versions, GPU allocation, endpoint
lifecycle, and locally observed capacity. ModelPort remains useful without a
managed GPU.

The current local Qwen path is a reference compatibility example only.
`local-inference-stack` is not a required repository, inventory authority, or
release dependency. Core behavior must not depend on its checkout layout,
scripts, environment variables, or file formats. Existing integration helpers
remain temporary until the generic Runtime Adapter follow-up replaces them.

### Product domains and console information architecture

Backend ownership and Dashboard navigation converge on the same eight domains:

| Domain | Backend responsibility | Dashboard surface | v0.1.x status |
| --- | --- | --- | --- |
| Models | Catalog identities, capabilities, limits, compatibility, and rate cards | Models | Partial: Provider-scoped inventory and logical catalog ship today |
| Providers | Hosted/local connectivity, credentials, account pools, health, and trust policy | Providers | Implemented under Settings and model views |
| Compute | Compute Nodes, GPUs, capacity observations, labels, freshness, and provenance | Compute | Target; no first-class inventory yet |
| Deployments | Model/runtime/compute binding, endpoint, desired state, observed state, and reconciliation | Deployments | Target; local endpoints are currently configured as Providers |
| Routing | Logical models, aliases, eligibility, fallback, smart decisions, and evidence | Routing | Implemented, with some controls under Settings and Governance |
| Governance | Users, teams, keys, policies, quotas, budgets, and approvals | Governance | Implemented |
| Observability | Requests, attempts, usage, cost, latency, GPU/runtime telemetry, and retained evidence | Observability | Partial: request and Provider evidence ship; compute telemetry is target |
| Operations | Readiness, incidents, backup, retention, upgrades, diagnostics, and reconciliation | Operations | Implemented for gateway operations; deployment operations are target |

This mapping is an information architecture contract, not a requirement to
create eight services. The backend remains a modular monolith until a separate
ADR proves a deployment boundary, including configuration distribution,
staleness, secret delivery, failure isolation, and rollback. The supported
default remains the single-process Rust data path on one host. UI migrations
should preserve deep links and API compatibility or document an explicit
replacement.

CLIProxyAPI (CPA) can be inserted only as an internal Provider boundary:

```text
clients -> ModelPort -> cpa_codex  -> CPA -> Codex OAuth accounts
                    -> cpa_claude -> CPA -> Claude OAuth accounts
                    -> other hosted/local Providers
```

ModelPort remains the only public client endpoint and owns authentication,
policy, routing, quota, retry/fallback, health, and durable evidence. CPA owns
OAuth material and bounded account selection. Its management API is outside
ModelPort's data plane. LiteLLM is not linked or deployed; only independently
useful design patterns may be adopted. This boundary is recorded in
[ADR-0004](adr/0004-modelport-gateway-and-cpa-provider-boundary.md).

Operational logs, latency percentiles, and Dashboard ranges are filtered,
aggregated, bucketed, ordered, and paginated in PostgreSQL. Runtime routes do
not materialize complete request windows in process memory.
Client API-key authentication uses an in-memory SHA-256 hash index, so request
authentication does not scan the configured key set.

The first typed Exchange IR covers text roles, function tools, tool calls and
results, generation controls, finish reasons, normalized usage, and terminal
stream state. It is intentionally narrower than the target enterprise IR:
multimodal content, Responses items, structured output, and Provider-native
extensions still require typed additions. UI panels labelled as a request
pipeline are explanatory views, not stored raw protocol payloads.

## Technical Core

The core is a bounded protocol-and-policy pipeline, not a generic model
platform. The table below separates the shipped mechanism from the boundary an
operator must still account for.

| Core | Implemented mechanism | Explicit boundary |
| --- | --- | --- |
| Protocol adaptation | Anthropic Messages and the scoped OpenAI Chat Completions contract parse into a typed Exchange IR and share routing/governance. Anthropic/OpenAI Provider adapters render native non-stream and SSE responses in the original client protocol. Parsers enforce frame/stream limits, require terminal signals, preserve reported usage, and reject unsupported semantics. | The IR does not yet cover Responses, multimodal content, reasoning items, or every OpenAI/Anthropic extension. Configured adapters and models are not proof of real-upstream compatibility. |
| Model routing and fallback | Resolution covers deterministic `provider:model`, recursive aliases, exact model matches, prefixes, and the default Provider. Opt-in smart aliases add capability/policy/quota/cooldown hard gates followed by explainable quality, reliability, latency, cost, and session-affinity scoring. Off, shadow, deterministic canary-control, and active decisions preserve a baseline path and store versioned evidence with the request. Retries remain limited to transport/protocol failures, 429, and 5xx against an eligible Provider. | Quality and latency configuration values are reviewed priors, not learned truth. Runtime latency and reliability signals are process-local. Fallback does not promise semantic model equivalence. Once live-stream headers are sent, a later failure cannot replay on another Provider. |
| Identity, policy, and budget | Human console sign-in supports local Argon2 credentials and an optional single-host OIDC Authorization Code + PKCE preview. A verified OIDC issuer/subject is bound to a local user, and both methods issue the same first-party console session. The data plane separately accepts a control-plane API key or the explicitly allowed shared token. API-key model/Provider/IP policy is configured in the control store. Before egress, PostgreSQL atomically admits the tenant budget and reserves user/API-key/team usage against settled plus open amounts in the attempt-creation transaction; terminal paths settle or release both forms of reservation. Only a sent attempt is chargeable. | OIDC authorization state and console sessions are process-local and do not provide multi-instance SSO continuity. OIDC does not authenticate `/v1/*` clients. PostgreSQL quota, spend, and tenant-budget reservations provide hard concurrent admission for their configured estimates. Rate limits and stream permits remain process-local. |
| Credential and Provider lifecycle | Provider credentials are environment-backed. Pool selection supports manual, failover, and round-robin behavior; outcomes feed credential/Provider health and cooldown state; unusable managed pools fail closed. | Health is operational state, not an external SLA. A configured credential or successful synthetic test does not establish every model, Tool Use, or stream path as verified. |
| Persistence and ledger | A running server requires PostgreSQL. SQLx/rustls, bounded pools, embedded migrations, composite tenant foreign keys, normalized request/attempt/routing-decision rows, hashed idempotency claims, instance leases, heartbeats, and an expired-lease reconciler form the operational ledger. The request and its routing evidence are inserted in one transaction. Terminal request rows plus open usage reservations are the source for logs, Dashboard ranges, API-key/team usage, quota/spend admission, and price snapshots. Audit events are append-only relational rows. | Low-frequency auth, API-key/team definitions, Provider overrides, and credential-pool configuration still use control documents. Routing feedback has a normalized storage foundation but is not allowed to mutate production weights online. Response replay is not implemented, and reconciled rows remain explicitly unbilled. |
| Security and observability | Browser writes require a ModelPort session and CSRF token, with Origin/Referer checks when present. The OIDC preview validates discovery metadata, signed ID-token claims, state, nonce, and PKCE before issuing that session. Trusted-proxy parsing, remote-Provider HTTPS defaults, URL and resolved-address guards, per-request DNS pinning, disabled redirects/proxies, bounded bodies/SSE, request/attempt IDs, terminal stream finalization, lease-expiry evidence, Prometheus metrics, retained logs, and source-labelled dashboard aggregation provide operational evidence. | OIDC is console authentication, not Provider or data-plane credential delegation, and its pending state is lost on restart. Private Provider URLs remain an explicit operator trust decision and outbound filtering remains defense in depth. `upstream-returned` identifies usage provenance, not invoice accuracy; `local-estimate` is heuristic, ordinary live streams may lack final Provider usage, and `unreconciled` requires external evidence before any financial adjustment. |

The detailed lifecycle and failure semantics below are normative. Provider and
Tool Use verification evidence is maintained separately in the
[Providers](PROVIDERS.md) and
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
- `src/domain.rs`: request/attempt identifiers, client protocol, and the
  explicit tenant/request context boundary.
- `src/database.rs`: SQLx PostgreSQL URL/TLS policy, pool bounds, acquisition
  timeout, and credential-safe location rendering.
- `src/enterprise_ledger.rs`: mandatory-tenant request/attempt lifecycle,
  operational log, usage-policy aggregation, budget, and append-only audit
  repository. PostgreSQL is required at runtime; memory is test-only.
- `src/exchange.rs`: typed client-protocol parsing, capability/fidelity checks,
  Provider rendering, and cross-protocol response mapping.
- `src/stream_lifecycle.rs`: shared upstream terminal state and normalized
  streaming usage evidence.
- `src/routes.rs`: application-state types, shared HTTP policy helpers, legacy
  handlers that have not yet moved, domain-router composition, and global
  middleware applied exactly once.
- `src/routes/`: domain-owned route registration plus public client,
  operations, identity, Provider, governance, control, and evidence handlers
  and views.
- `src/providers/`: Anthropic pass-through and OpenAI-compatible request,
  response, and SSE conversion.
- `src/http.rs`: the upstream HTTP client, bounded response reading, SSE frame
  parsing, timeouts, redirect policy, and upstream error redaction.
- `src/auth.rs`: dashboard users, Argon2 password hashes, per-username login
  lockout, OIDC issuer/subject bindings and local-user resolution,
  timing-mitigation work, in-memory sessions, and session cookies.
- `src/control.rs`: API keys, teams, policy/quota definitions, Provider
  overrides, credential pools, health, and tests. It does not store request
  usage or audit history.
- `src/storage.rs`: compatibility JSON-file or SQLx/PostgreSQL persistence for
  the two auth/control documents.
- `src/metrics.rs`: process-local Prometheus counters.
- `dashboard/`: the browser control plane. It consumes `/admin/*`; it is not a
  second source of routing truth.

### HTTP route ownership

The single-process server is composed from ten explicit HTTP domains: system,
client API, internal operations, admin authentication, governance, admin
operations, Providers, control, evidence, and identity. Each domain module owns
its Axum method/path registration. `routes::router` merges those routers and
then applies request IDs, tracing, concurrency, response headers, and the global
body limit once; domain-local middleware such as login body and cache policy
remains next to the owned route.

Tests maintain one complete method/path/domain inventory for the 68 current
route registrations. They reject duplicate method ownership and probe the
composed application so a missing path or method fails independently of handler
authorization or resource lookup results. A new Compute, Deployment, protocol,
or admin capability must extend its domain router and this inventory rather
than adding another registration to the root composition function.

This is an internal modular-monolith boundary. It does not create another
process, public discovery endpoint, authorization source, or API version, and
reverting the composition requires no data or protocol migration.

## Request Lifecycle

For `POST /v1/messages` and `POST /v1/chat/completions`, the current order is:

1. Axum applies the global body-size and concurrency layers and assigns an
   `x-request-id` when one was not supplied. Both built-in protocol adapters
   forward that opaque value upstream; it is correlation metadata, not a
   trace/span parent.
2. The client is authenticated with a control-plane API key or, when allowed,
   the legacy router token.
3. The edge validates its protocol contract and parses a typed Exchange
   request. Anthropic `max_tokens` is mandatory; OpenAI output-token limits are
   optional. Supplied limits must be positive and within the configured cap.
4. The model is resolved from `provider:model`, an alias, an exact model, a
   prefix, or the default provider.
5. Process-local rate limits run for global, identity, IP, provider, and model
   dimensions.
6. A streaming request acquires a process-local stream permit or returns 429
   before an upstream attempt. Then a route-attempt list is built;
   cooling-down providers are skipped while an eligible alternative exists; if
   every eligible route is cooling, the primary remains as the final attempt.
7. ModelPort atomically creates the tenant-scoped request row. When an
   `Idempotency-Key` is present, its hash is uniquely claimed alongside the
   protocol/body fingerprint; a duplicate returns 409 before Provider egress.
   A per-instance lease starts and remains owned through the response body.
8. For each attempt, ModelPort selects a provider credential, checks API-key
   policy and quota, validates the provider URL and capability gate, then calls
   the protocol adapter. Immediately before the call it inserts a leased,
   tenant-scoped Provider-attempt row. `failover` and `round_robin` pools with no usable
   credential fail closed for that Provider; only `manual` can retain an
   explicitly selected non-disabled credential.
9. Non-stream responses are mapped back to the originating client protocol
   before returning. Usage, provider outcome,
   metrics, and the request log are recorded. Quota and spend state changes only
   after an upstream attempt was actually sent; an attempt-level preflight
   rejection that reaches this recorder is logged with zero usage and no charge.
10. Stream responses are rendered as Anthropic events or OpenAI
   `chat.completion.chunk` events. A single body-lifecycle finalizer classifies
   protocol completion, upstream failure/timeout, delivery failure, or
   downstream cancellation, reconciles any streamed usage, and records terminal
   metrics, request evidence, and the known Provider outcome.

The lease heartbeat runs every one-third TTL for both handler and response-body
lifetimes. Startup and a periodic worker claim no ownership; they only
terminalize rows whose recorded lease is already expired. Those rows use
`lease_expired_unreconciled`, zero usage, and `chargeable=false`, avoiding both
permanent `started` records and fabricated billing evidence.

Automatic cross-provider fallback is limited to transport failures, upstream
protocol failures, HTTP 429, and HTTP 5xx, and only to a configured provider
that can accept the requested or resolved model. It is not a semantic guarantee
that the fallback model behaves identically.

`POST /v1/messages/count_tokens` is a smaller authenticated data-plane path:
it reuses Anthropic input guardrails, model resolution, Provider credential
selection, API-key/team model/Provider/IP policy, URL policy, and rate limits,
then calls only the resolved Provider's explicit token-counting capability. It
has no fallback, inference ledger row, or usage charge because tokenizer
identity must remain exact and no generation occurs.

Completed paths that expose Provider usage are labelled
`billingMode="upstream-returned"`; paths that use the request heuristic are
`billingMode="local-estimate"`. This token provenance is separate from monetary
reconciliation. `costEstimate` always remains operational; `actualCost` and
`billableCost` require either a trusted Provider amount or an exact-model,
versioned rate card applied to Provider usage. A request can be partially
billable when only some retry/fallback attempts have evidence; each attempt row
retains its own evidence. Request totals include every sent attempt. Mixed
Provider-reported and locally estimated attempts use
`billingMode="mixed-attempts"`, while fully estimated retries use
`billingMode="local-estimate+retry"`.

## Configuration And Runtime Overrides

Base configuration comes from environment defaults or a TOML file. The control
plane can overlay provider records, model inventory, aliases, default provider,
and provider order. See [Configuration](CONFIGURATION.md) for the exact source
and reload rules.

The TOML-only Runtime Adapter registry is a separate trusted control-plane
boundary. Its adapter identities, discovery origins, credentials, and
collection/freshness policy do not participate in inference Provider routing
or inherit development-harness metadata. Enabled entries are validated and
their environment-backed credentials resolved at configuration load; polling
and inventory presentation are separate lifecycle slices.

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
| `auth` | Users, password hashes, and OIDC issuer/subject bindings. Sessions, pending OIDC authorization state, and failed-login counters are process-local. |
| `control` | Teams, API-key hashes, policy and quota definitions, routing overrides, credentials metadata, and provider health. |

`MODELPORT_DATABASE_URL` is mandatory. These low-frequency documents are stored
as two `jsonb` rows in `modelport_state`; there is no runtime file fallback or
automatic JSON import. Each row carries a monotonic `revision`; complete-document
writes use compare-and-swap and return a stable HTTP 409 conflict instead of
overwriting a newer revision. Readiness also fails closed when an instance
detects that its in-memory revision is stale. Logical backup restore replaces
the auth and control rows in one PostgreSQL transaction. This is an interim
lost-update guard, not a substitute for the planned tenant-scoped relational
repositories and cross-domain transactions. The synchronous store boundary
uses a dedicated SQLx/Tokio worker with rustls and a one-connection pool.

The async normalized ledger uses `MODELPORT_ENTERPRISE_DATABASE_URL` or falls
back to `MODELPORT_DATABASE_URL`. Embedded migrations create explicit
organization, project, and environment parents plus gateway-request and
Provider-attempt children, budget accounts, per-attempt reservations, and an
append-only evidence event stream. Composite keys make the tenant part of every parent
relationship, and repository writes repeat that tenant scope in the query.
PostgreSQL is the only runtime ledger implementation.

The relational slice removes request/attempt write amplification, makes
incomplete work discoverable, and serializes competing budget reservations in
PostgreSQL. Attempt creation plus reservation, terminal settlement, and
expired-lease release each commit atomically. Dashboard ranges, request logs,
quota/spend checks, management statistics, audit history, and Enterprise
Operations all read indexed relational rows—including identity, client path,
traffic class, Tool Use outcome, pricing provenance, latency/TTFT, repair,
retry, fallback, ordered Provider attempts, and recent budget evidence.

Low-frequency identity, policy, quota, routing, Provider, and credential
mutations snapshot the in-memory document before writing. A failed or stale
write restores that snapshot, returns an error, and makes readiness fail closed
until a later complete write succeeds; neither a persistence 5xx nor a state
conflict 409 can leave a routing or authorization change active only in the
current process. Request
finalization after response headers is asynchronous so a persistence failure
cannot replace a response already paid for and received from an upstream;
readiness and ledger diagnostics expose such failures.

CLI backup load validates both document schemas and critical auth invariants
before restore. Restore saves the previous values, verifies both observed
revisions, and replaces auth and control together in one PostgreSQL transaction.

## Identity And Budget Boundaries

Human console sign-in can use a local password or the optional
[OIDC preview](OIDC.md). OIDC identity is bound by the verified issuer/subject
pair to a local ModelPort user and produces the same HttpOnly console session;
automatic provisioning, when enabled, creates only an ordinary `user`. The
identity-provider token and the ModelPort session cookie are never accepted as
data-plane credentials. Pending OIDC state and active console sessions are
process-local, so a restart invalidates both and this slice does not provide
multi-instance enterprise IAM.

An API key must be created for a real active auth user, and the server stores
the canonical username rather than trusting request metadata. Every data-plane
authentication checks that the owner still exists and is active. Disabling or
suspending a user revokes that user's keys and removes the user's quota rows.

Console roles intentionally differ: administrators manage all key policy and
lifecycle fields. Normal users can create up to five active personal keys with
a maximum 30-day lifetime, narrow model/Provider scope during creation, rotate
their user keys, and rename, group, revoke, or delete owned keys. They cannot
create service-account/team keys, restore keys, or edit administrator-controlled
team/model/provider/IP/expiry/spend policy after creation. Viewers are read-only.

User quota records use UTC calendar periods: a day begins at 00:00 UTC, a week
at Monday 00:00 UTC, and a month on its first day at 00:00 UTC. API-key and team
spend policy is separate and uses rolling 5-hour, 24-hour, 7-day, and 30-day
windows. The `rateLimited` name enables periodic spend limits rather than
request-rate limiting.

Rolling-spend and user-quota admission sums settled relational usage plus open
usage reservations. PostgreSQL takes stable scope locks and creates or extends
the usage reservation in the same transaction as the Provider attempt. One
logical request reserves its request unit only once; retries add token/cost
estimates. Terminal completion settles actual usage, while non-chargeable or
expired work releases the reservation.

A team cannot be deleted while any API key references it. This dependency
check prevents deletion from silently broadening access by removing team
policy; operators must reassign or delete referencing keys first.

## Dashboard Aggregation

Dashboard trend queries are aggregated on the server over the complete retained
usage set in the requested window, not over the current paginated logs page.
The response includes request/error and token buckets, model usage, and a range
summary. Ranges are bounded to 90 days.

The backend marks the source as `relational-ledger` or `empty`. It never
substitutes process-lifetime counters for historical charts. The query reads
only the selected trend window plus the current UTC day.

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
separate phase: the request log, message metrics, and Provider health are not
finalized at handshake. A response-body guard records protocol completion,
upstream failure/timeout, delivery failure, or downstream cancellation exactly
once. Later stream failures still cannot participate in cross-provider
fallback, and final token usage commonly remains a request estimate. Operators
must inspect the SSE body as well as the terminal log. A live-stream timeout can
therefore be persisted as `status=timeout` with a 504 terminal mapping even
though the already-sent HTTP status remains 200.

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

The general request timeout covers the entire non-stream exchange and the total
upstream SSE lifecycle. After response headers, each body read is bounded by
both the remaining total time and a resettable per-chunk idle timeout. Line,
event, and total raw-stream byte ceilings apply independently.

The stream permit count comes from `MODELPORT_MAX_CONCURRENT_STREAMS`, defaulting
to the effective general request-concurrency limit. Unlike the normal handler
future, the permit is moved into the returned body and survives until that body
finishes or is dropped. This makes downstream slow readers visible to capacity
control; an exhausted semaphore returns HTTP 429 with `Retry-After: 1` and no
quota/spend charge. A dropped body records a 499 downstream-cancellation
outcome. When upstream completion is already known, Provider health remains a
success even though downstream delivery did not complete.

`buffer_stream_text=true` is a distinct compatibility path. ModelPort sends a
non-stream OpenAI-compatible request, awaits and validates the complete
response, converts it to an Anthropic message, and only then creates locally
chunked SSE. Upstream HTTP/protocol failures therefore remain normal HTTP errors
and can fallback before local headers. When the upstream reports usage, the
adapter attaches it to the internal response so metrics, quota spend, and the
request log use those token values instead of the request estimate. The tradeoff
is full-generation time to first byte; client cancellation after local SSE
starts cannot cancel an upstream generation that already finished. Local
delivery cancellation is observed and logged separately from that successful
upstream outcome.

## Security Boundaries

- Data-plane credentials and dashboard sessions are separate.
- Optional OIDC uses Authorization Code flow with PKCE, state, nonce, and a
  short-lived HttpOnly browser-flow binding for human console authentication.
  Verified identities resolve to local users and receive the normal ModelPort
  session; identity-provider tokens are not forwarded to Providers or accepted
  by `/v1/*`.
- Pending OIDC authorization state and console sessions are process-local. A
  restart invalidates in-progress sign-ins and active sessions, so the preview
  is intended for the current single-host deployment profile.
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
  and private/link-local/metadata destinations by default. Immediately before
  each request, hostname answers are validated and pinned into the HTTP client
  while the original hostname remains available for Host and TLS SNI.
  Redirects and environment proxies are disabled to prevent a second resolver
  from changing the destination. Credentials are sent from environment-backed
  header configuration rather than embedded in the URL. Explicitly allowing a
  private Provider remains an operator trust decision and should be paired with
  outbound network policy.
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
- Treating one local runtime repository, engine, or Harness as the ModelPort
  product model.
- A chat client or prompt-history product.
- Complete enterprise IAM (SCIM, service accounts, organization lifecycle,
  resource-level RBAC, and distributed SSO/session coordination), public
  multi-tenancy, or exact billing. The shipped OIDC slice is a single-host
  console sign-in preview, not that broader identity plane.
- Distributed rate limiting or multi-instance coordination.
- A complete provider-neutral Tool/Message IR.
- Image and Responses APIs in the current text gateway.
