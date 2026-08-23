# Configuration

This document is the maintained reference for ModelPort configuration. Start
from [`.env.example`](../.env.example) for local development or
[`deploy/docker/modelport.env.example`](../deploy/docker/modelport.env.example)
for Docker Compose.

## Sources

ModelPort supports two base-configuration modes:

1. **Environment defaults:** used when no TOML configuration file exists.
   Built-in provider templates are enabled by credentials, provider-specific
   values, or an explicit `MODELPORT_ENABLE_*` flag.
2. **TOML:** set `MODELPORT_CONFIG`, or place a file at
   `~/.config/modelport/config.toml`. TOML defines provider records, order,
   aliases, server defaults, and the router-token environment variable.

An environment file is read from `MODELPORT_ENV_FILE`, or from `.env` in the
current working directory when present. Local scripts source `.env` into the
process. Docker Compose both supplies it as `env_file` and mounts it read-only
at `/config/.env`.

The process environment takes precedence over `MODELPORT_ENV_FILE`/`.env` for
the same key. Avoid defining conflicting values in both places. In Docker,
remember that Compose copies `env_file` values into the process when the
container is created.

Control-plane overrides are applied after the base configuration for provider
records, model inventory, aliases, default provider, and provider order.

## Runtime Adapter Registry

TOML may declare up to 64 trusted Runtime Adapter origins. Runtime Adapters are
provider-neutral control-plane discovery implementations: they do not execute
inference requests and their IDs are independent from Provider IDs, Codex or
Claude development harnesses, and external implementations such as the Qwen
reference adapter.

```toml
[runtime_adapters.gpu_fleet]
enabled = true
base_url = "https://runtime-adapter.internal.example"
bearer_token_env = "MODELPORT_RUNTIME_ADAPTER_GPU_FLEET_TOKEN"
poll_interval_seconds = 30
stale_after_seconds = 90
```

Each enabled entry requires a v1alpha1 adapter ID, an HTTPS origin (or plain
HTTP on a literal loopback address), and the name of an environment variable
containing an RFC 6750 Bearer token. Inline credentials and unknown fields are
rejected. The environment-variable name must contain only ASCII letters,
digits, and underscores and must not begin with a digit. ModelPort resolves and
validates the token at startup; debug output and errors redact it, and the token
is never part of the TOML document or a serializable configuration type.

`poll_interval_seconds` defaults to 30 and is bounded from 5 through 3,600.
`stale_after_seconds` defaults to 90, is bounded from 5 through 86,400, and
must cover at least one polling interval. Disabled declarations are inert:
their endpoint and credential are neither required nor resolved. Duplicate
TOML adapter tables, invalid enabled declarations, missing credentials, and a
registry over 64 entries fail configuration loading closed. This registry does
not start polling; background collection and admin inventory APIs remain
separate reviewed work.

## Required Minimum: DeepSeek-Only Example

```env
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-local-token
MODELPORT_ADMIN_USERNAME=admin
MODELPORT_ADMIN_PASSWORD=replace-with-a-long-random-admin-password
MODELPORT_DEFAULT_PROVIDER=deepseek

DEEPSEEK_ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
DEEPSEEK_ANTHROPIC_AUTH_TOKEN=replace-with-a-real-provider-key
DEEPSEEK_MODEL=deepseek-v4-flash
```

The client must send the effective router token. `ANTHROPIC_AUTH_TOKEN` is also
accepted as the router-token fallback when `MODELPORT_AUTH_TOKEN` is absent,
but deployments should set one unambiguous server token and make the client
match it.

This minimum is one supported topology, not a requirement that every ModelPort
deployment install DeepSeek. At least one enabled Provider and a valid
`MODELPORT_DEFAULT_PROVIDER` are required; a Qwen-only deployment can omit all
DeepSeek values.

Validate before startup:

```bash
scripts/config-validate.sh
```

The CLI and normal server startup both load `AppConfig`, evaluate its
`validation_issues()` boundary, and run the same deployment-environment
preflight. Any `Error` makes `config validate` exit non-zero and makes the
server refuse to bind; `Warning` entries are printed by the CLI and logged by
the server while startup continues. Base-config reload also rejects a candidate
with application-configuration errors, but environment-only deployment changes
still require a restart.

Placeholder secrets, an invalid/missing default provider, broken aliases,
unsafe provider definitions, and malformed guardrail values therefore do not
silently enter service. Numeric environment variables are checked as unsigned
integers and, where zero has no safe meaning, as greater than zero. In
particular `MODELPORT_MAX_REQUEST_BODY_BYTES`,
`MODELPORT_MAX_CONCURRENT_REQUESTS`, and the documented request-size, session,
HTTP timeout/body, and SSE byte guardrails must be non-zero. Rate limiting has
its separate explicit disable switch.

The shared deployment preflight requires `MODELPORT_DATABASE_URL` and validates
PostgreSQL URL syntax without echoing credentials, database TLS policy, pool
min/max and acquisition-timeout bounds, enterprise lease timing, trusted-proxy
IP/CIDR entries, and allowed-origin syntax. Enterprise mode additionally
permits only `verify-full`. This is a local syntax and policy check: it does not
connect to PostgreSQL, run migrations, or verify the live certificate chain.
Startup and authenticated `/readyz` provide those runtime checks.

## Provider Topology Recipes

Provider topology is defined by TOML records plus the environment values those
records reference. Keep runtime endpoints and secrets in `.env`/the process;
keep provider names, protocol, model inventory, aliases, and order in TOML.

### Smart routing groups

Smart routing is opt-in and applies only to aliases declared under
`[routing.groups.*]`. Existing aliases and explicit `provider:model` requests
remain deterministic. A route first removes disabled, capability-incompatible,
policy/quota-ineligible, and cooling candidates. It then ranks the remaining
candidates with bounded operator quality/latency priors, current Provider
reliability, recent process-local latency, estimated price, and an optional
stable session affinity signal. Prompt or message content is not inspected.

```toml
[routing]
mode = "shadow"               # off, shadow, or active
default_profile = "balanced"  # quality, balanced, economy, or latency
policy_version = "builtin-v1"
activation_percent = 0        # 0-100; used only by active mode

[routing.groups.general]
aliases = ["modelport-auto"]
default_profile = "balanced"

[[routing.groups.general.candidates]]
provider = "deepseek"
model = "deepseek-v4-pro"
quality = 0.95
latency_hint_ms = 1800

[[routing.groups.general.candidates]]
provider = "openai"
model = "gpt-5.5"
quality = 0.98
latency_hint_ms = 2200
```

`off` keeps a configured smart alias on its declared baseline order when it is
called directly, but does not advertise that alias from `/v1/models`. `shadow`
advertises it and computes/stores a recommendation without changing the
baseline first candidate. `active` applies the scored order only to the stable
session bucket (when the session header exists) or principal-scoped
request-ID bucket selected by `activation_percent`; without a reused request ID,
assignment is per request. The other requests remain the canary control group.
An active configuration with zero percent is valid but emits a warning.

Clients may override the group/default profile with
`x-modelport-routing-profile`. `x-modelport-session-id` adds a small,
deterministic affinity tie-breaker; its raw value is neither logged nor
persisted. Both headers are optional. Invalid profiles fail before Provider
egress.

The following environment variables are emergency/runtime overrides for the
corresponding TOML values:

```env
MODELPORT_SMART_ROUTING_MODE=shadow
MODELPORT_SMART_ROUTING_PROFILE=balanced
MODELPORT_SMART_ROUTING_ACTIVATION_PERCENT=0
```

Candidate quality is currently a versioned operator prior, not a self-modifying
online-learning weight. Change it through reviewed configuration, increment
`policy_version`, and compare shadow evidence before increasing active traffic.
Configuration is bounded to 128 groups, 64 aliases per group, 256 candidates
per group, and 1,024 aliases and candidates in total.

### Local Qwen only

Environment:

```env
MODELPORT_CONFIG=config.toml
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-router-token
MODELPORT_DEFAULT_PROVIDER=local_qwen
QWEN_LOCAL_BASE_URL=http://qwen-runtime:8080/v1
```

Omit `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`, `DEEPSEEK_API_KEY`, and every other
unused upstream credential. For a host process, replace the Docker DNS address
with the Qwen runtime's reachable loopback URL.

The complete contract-aligned example is
[`deploy/local-inference/modelport.local-qwen.toml`](../deploy/local-inference/modelport.local-qwen.toml).
It is validated by the repository checks and should be copied or merged rather
than edited in place.

```toml
default_provider = "local_qwen"
provider_order = ["local_qwen"]

[auth]
token_env = "MODELPORT_AUTH_TOKEN"

[providers.local_qwen]
display_name = "Qwen3.5-9B Q5_K_M (local)"
protocol = "openai-compat"
base_url_env = "QWEN_LOCAL_BASE_URL"
base_url = "http://qwen-runtime:8080/v1"
api_key_required = false
default_model = "qwen3.5-9b-q5km"
models = ["qwen3.5-9b-q5km"]
passthrough_unknown_models = false
max_tokens_field = "max_tokens"
fidelity_mode = "best_effort"

[providers.local_qwen.tool_use]
supported = true
tool_choice = true
parallel_tool_calls = true
streaming_arguments = "best_effort"
response_validation = "strict"
repair_invalid_arguments = true

[providers.local_qwen.token_counting]
mode = "anthropic"
context_tokens = 131072
recommended_reasoning_input_tokens = 94208
model_recommended_input_tokens = { "qwen3.5-fast" = 24576, "qwen3.5-code" = 57344, "qwen3.5-deep" = 94208 }
max_output_tokens = 32768
model_max_output_tokens = { "qwen3.5-fast" = 4096, "qwen3.5-code" = 16384, "qwen3.5-deep" = 32768 }

[aliases]
"qwen3.5-fast" = "local_qwen:qwen3.5-9b-q5km"
"qwen3.5-code" = "local_qwen:qwen3.5-9b-q5km"
"qwen3.5-deep" = "local_qwen:qwen3.5-9b-q5km"
```

Use an environment-backed API key field if the local runtime itself requires
authentication; do not reuse ModelPort's client/router token as an upstream key
unless the runtime was deliberately configured that way.

### DeepSeek official Anthropic only

Use the required-minimum environment above and the shipped
[`config.example.toml`](../config.example.toml). The Provider protocol must be
`anthropic`, its Base URL must be `https://api.deepseek.com/anthropic`, and its
server-side secret is `DEEPSEEK_ANTHROPIC_AUTH_TOKEN`.

The dashboard's administrator-only balance action calls the official balance
endpoint from the server with that credential. It can display availability and
CNY/USD balances; it cannot recharge, refund, invoice, or replace the DeepSeek
console's authoritative billing. ModelPort usage/cost records are local
governance evidence and must not be presented as the upstream invoice.

### Local Qwen plus DeepSeek

Combine the two Provider records and make the default explicit:

```toml
default_provider = "local_qwen"
provider_order = ["local_qwen", "deepseek"]
```

Keep both endpoint values in the ModelPort environment:

```env
QWEN_LOCAL_BASE_URL=http://qwen-runtime:8080/v1
DEEPSEEK_ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
DEEPSEEK_ANTHROPIC_AUTH_TOKEN=replace-with-a-real-provider-key
```

Unqualified Qwen aliases can remain the default, while clients can select
DeepSeek deterministically with `deepseek:deepseek-v4-flash`. Provider fallback
does not mean arbitrary model substitution: the requested model must be
eligible for the fallback Provider and the failure must be retryable.

### CPA as an internal Provider

CPA is an optional CLIProxyAPI account adapter behind ModelPort. Do not point
clients directly to CPA and do not configure CPA as ModelPort's control plane.
In environment-default mode, enable its Codex and Claude channels
independently:

```env
MODELPORT_ENABLE_CPA_CODEX=1
CPA_CODEX_BASE_URL=http://127.0.0.1:8317/v1
CPA_CODEX_API_KEY=replace-with-cpa-client-api-key
CPA_CODEX_MODEL=gpt-5.3-codex
CPA_CODEX_MODELS=gpt-5.3-codex

MODELPORT_ENABLE_CPA_CLAUDE=1
CPA_CLAUDE_BASE_URL=http://127.0.0.1:8317
CPA_CLAUDE_API_KEY=replace-with-cpa-client-api-key
CPA_CLAUDE_MODEL=claude-sonnet-4-6
CPA_CLAUDE_MODELS=claude-sonnet-4-6
```

Setting either key or an explicit `MODELPORT_ENABLE_CPA_*` flag enables only
that Provider. The two key variables may contain the same CPA client key; they
stay separate so enabling one protocol never silently enables the other.

The maintained Docker path uses TOML. TOML mode never invents Provider records
from environment variables, so add the channels you intend to use and include
them in `provider_order`:

```toml
provider_order = ["deepseek", "cpa_codex", "cpa_claude"]

[providers.cpa_codex]
display_name = "CPA · OpenAI Codex"
protocol = "openai-compat"
base_url_env = "CPA_CODEX_BASE_URL"
api_key_env = "CPA_CODEX_API_KEY"
api_key_required = true
default_model = "gpt-5.3-codex"
models = ["gpt-5.3-codex"]
model_prefixes = []
passthrough_unknown_models = false
max_tokens_field = "max_completion_tokens"

[providers.cpa_claude]
display_name = "CPA · Claude Code"
protocol = "anthropic"
base_url_env = "CPA_CLAUDE_BASE_URL"
api_key_env = "CPA_CLAUDE_API_KEY"
api_key_required = true
default_model = "claude-sonnet-4-6"
models = ["claude-sonnet-4-6"]
model_prefixes = []
passthrough_unknown_models = false
max_tokens_field = "max_tokens"
```

CPA Providers require explicit model allowlists and
`passthrough_unknown_models=false`. They cannot claim a model prefix. Use
`cpa_codex:model` or `cpa_claude:model` for acceptance and deterministic
routing. `cpa_codex` Base URL must end in `/v1`; `cpa_claude` must omit `/v1`
because the Anthropic adapter appends `/v1/messages`.

Docker deployments should use `http://cpa:8317/v1` and
`http://cpa:8317` only after attaching CPA under the single-label `cpa` DNS
name to ModelPort's private network. Keep CPA unexposed. A private literal IP
still requires `MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS=1`; a public hostname
requires HTTPS.

ModelPort owns request-level retries and cross-Provider fallback. Set CPA's
`request-retry: 0` and bound `max-retry-credentials` initially so one
ModelPort attempt cannot fan out across an unbounded CPA account pool. See the
[CPA Provider contract](PROVIDERS.md#cpa-codex-and-claude-account-adapter).

### QuantPilot client boundary

For QuantPilot, issue a dashboard API key scoped only to the providers/models it
needs, commonly:

- `local_qwen:qwen3.5-9b-q5km`
- `deepseek:deepseek-v4-flash`
- `GET /v1/models` and `POST /v1/chat/completions`

Store that client key in QuantPilot as `MODELPORT_API_KEY`. Never copy
`DEEPSEEK_ANTHROPIC_AUTH_TOKEN`, a Qwen upstream key, the complete ModelPort
`.env`, or provider credential-pool material into QuantPilot. A Qwen-only client
key may omit every DeepSeek scope; ModelPort itself may also run Qwen-only.

QuantPilot's official-direct `deepseek-v4-flash` profile bypasses ModelPort and
uses its own `DEEPSEEK_API_KEY`; it is a separate path from the namespaced
`deepseek:deepseek-v4-flash` ModelPort model. ModelPort is not involved in the
direct path and cannot govern its usage or balance.

## Server, Authentication, And State

| Variable | Default | Meaning |
| --- | --- | --- |
| `MODELPORT_BIND` | `127.0.0.1:17878` | Backend listen address. |
| `MODELPORT_MAX_REQUEST_BODY_BYTES` | `33554432` | Axum request-body limit for all routes; must be greater than zero. |
| `MODELPORT_MAX_CONCURRENT_REQUESTS` | `64` | Process-wide concurrency layer; must be greater than zero. |
| `MODELPORT_MAX_CONCURRENT_STREAMS` | inherits `MODELPORT_MAX_CONCURRENT_REQUESTS` | Maximum concurrent streaming response bodies. Exhaustion returns HTTP 429 with `Retry-After: 1`; the permit is held until the body completes or is dropped. |
| `MODELPORT_AUTH_TOKEN` | required | Legacy router token. |
| `MODELPORT_ALLOW_NO_AUTH` | off | Dangerous isolated-test override. Never use on a shared network. |
| `MODELPORT_REQUIRE_CONTROL_API_KEYS` | off | Reject the legacy token wherever data-plane authentication is evaluated (`/v1/*`, `/metrics`, `/readyz`, and detailed health); require dashboard-issued keys. |
| `MODELPORT_ADMIN_USERNAME` | `admin` | First-admin bootstrap username. Used only when the auth store is empty. |
| `MODELPORT_ADMIN_PASSWORD` | effective router token fallback | First-admin password; set it explicitly. It and the fallback must pass strong-password checks. |
| `MODELPORT_ADMIN_EMAIL` | `admin@modelport.local` | First-admin bootstrap email. |
| `MODELPORT_ADMIN_SESSION_TTL_SECONDS` | `43200` | Dashboard session lifetime. |
| `MODELPORT_ADMIN_COOKIE_SECURE` | off | Add `Secure` to the dashboard cookie. Set to `1` behind HTTPS. |
| `MODELPORT_REQUIRE_DUAL_APPROVAL` | off; always on in enterprise mode | Require an approved change request from two distinct administrators before high-risk identity, Provider, model, or hard-budget writes. Small-Team mode otherwise relies on the administrator session, CSRF protection, and audit trail so a one-admin first install remains operable. |
| `MODELPORT_OIDC_ISSUER` | unset | OIDC issuer discovery URL. OIDC console sign-in stays disabled when no OIDC values are configured. |
| `MODELPORT_OIDC_CLIENT_ID` | unset | OIDC client identifier; required with issuer and redirect URI when OIDC is enabled. |
| `MODELPORT_OIDC_CLIENT_SECRET` | unset | Optional confidential-client secret. Leave unset only when the identity provider accepts the supported public-client code exchange. |
| `MODELPORT_OIDC_REDIRECT_URI` | unset | Exact external callback URL; its path must be `/admin/auth/oidc/callback` with no query or fragment. |
| `MODELPORT_OIDC_LABEL` | `Single sign-on` | Login-button label. |
| `MODELPORT_OIDC_AUTO_PROVISION` | off | Create missing ordinary users after a valid OIDC login. Keep off initially and pre-create users; it never grants administrator access. |
| `MODELPORT_OIDC_USERNAME_CLAIM` | `preferred_username` | ID-token claim used as the ModelPort username. |
| `MODELPORT_OIDC_EMAIL_CLAIM` | `email` | ID-token claim read as the ModelPort email. Initial linking/JIT requires the standard `email` claim plus `email_verified=true`; verification is not inherited by a custom claim name. |
| `MODELPORT_OIDC_ALLOW_INSECURE_HTTP` | off | Allow HTTP only for loopback OIDC development URLs. Never enable it for a remote or production identity provider. |
| `MODELPORT_STATE_DIR` | `.modelport` | Working directory for explicit backup output. Runtime state is not stored here. |
| `MODELPORT_DATABASE_URL` | required at runtime | PostgreSQL target for the operational ledger and low-frequency auth/control documents. Compose constructs an internal default unless explicitly overridden. |
| `MODELPORT_ENTERPRISE_DATABASE_URL` | inherits `MODELPORT_DATABASE_URL` | Optional separate PostgreSQL target for the operational ledger and embedded migrations. `MODELPORT_DATABASE_URL` is still required. |
| `MODELPORT_DATABASE_TLS_MODE` | `prefer`; `verify-full` in enterprise mode | SQLx PostgreSQL TLS mode: `disable`, `allow`, `prefer`, `require`, `verify-ca`, or `verify-full`. Enterprise mode rejects every value except `verify-full`. Certificate options such as `sslrootcert` can be supplied in the PostgreSQL URL. |
| `MODELPORT_DATABASE_MAX_CONNECTIONS` | `16` | Maximum connections in the normalized ledger pool. Each auth/control document worker independently caps its pool at one connection. |
| `MODELPORT_DATABASE_MIN_CONNECTIONS` | `0` | Minimum eagerly maintained PostgreSQL connections, capped at the pool maximum. |
| `MODELPORT_DATABASE_ACQUIRE_TIMEOUT_SECS` | `10` | Maximum wait to acquire a PostgreSQL connection. |
| `MODELPORT_LEDGER_LEASE_TTL_SECS` | `300` | Lifetime of a request/attempt ownership lease; minimum 30 seconds. Active requests renew at one-third of this interval. |
| `MODELPORT_LEDGER_RECONCILE_INTERVAL_SECS` | `60` | Interval for reclaiming expired `started` rows; minimum 5 seconds and strictly smaller than the lease TTL. |
| `MODELPORT_FINALIZATION_DRAIN_TIMEOUT_SECONDS` | `30` | Graceful-shutdown deadline for tracked streaming ledger finalizers; valid range `1..300`. |
| `MODELPORT_REQUEST_DETAIL_RETENTION_DAYS` | `30` | Age after which an explicit admin retention apply redacts request identity/network/error/idempotency details, redacts mutable Provider-attempt details, and removes routing-decision evidence. |
| `MODELPORT_USER_USAGE_RETENTION_DAYS` | `90` | Age after which an explicit retention apply de-identifies user-level usage rows while preserving required aggregate/budget evidence. Must not be shorter than request-detail retention. |
| `MODELPORT_AUDIT_RETENTION_DAYS` | `395` | Age after which an explicit retention apply removes ordinary governance audit events. Must not be shorter than user-usage retention; immutable budget events remain. |
| `MODELPORT_RETENTION_LEGAL_HOLD` | off | When `1`, retention dry-run still previews but an apply returns `applied=false` and changes no retained data. Restart required. |
| `MODELPORT_ENTERPRISE_MODE` | off | Fail-closed production profile. Requires database TLS `verify-full`, secure admin cookies, dashboard-issued control API keys, HTTPS-only allowed origins, explicit trusted proxies, and enabled CSRF protection. |

Bootstrap variables do not overwrite existing users. Dashboard sessions are
process-local and are invalidated by restart.

OIDC is an optional console-login method and is disabled by default. Once any
required OIDC value is configured, set `MODELPORT_OIDC_ISSUER`,
`MODELPORT_OIDC_CLIENT_ID`, and `MODELPORT_OIDC_REDIRECT_URI` together. Register
the exact external callback ending in `/admin/auth/oidc/callback`; this path is
fixed. Automatic provisioning remains disabled unless explicitly enabled.

```env
MODELPORT_OIDC_ISSUER=https://identity.example.com/realms/modelport
MODELPORT_OIDC_CLIENT_ID=modelport
MODELPORT_OIDC_REDIRECT_URI=https://modelport.example.com/admin/auth/oidc/callback
# Optional for a confidential client:
MODELPORT_OIDC_CLIENT_SECRET=replace-with-client-secret
MODELPORT_OIDC_LABEL=Company SSO
MODELPORT_OIDC_AUTO_PROVISION=0
MODELPORT_OIDC_USERNAME_CLAIM=preferred_username
MODELPORT_OIDC_EMAIL_CLAIM=email
# Loopback development only:
# MODELPORT_OIDC_ALLOW_INSECURE_HTTP=1
```

For production OIDC, serve one HTTPS origin and also set:

```env
MODELPORT_ADMIN_COOKIE_SECURE=1
MODELPORT_ALLOWED_ORIGINS=https://modelport.example.com
MODELPORT_REQUIRE_CONTROL_API_KEYS=1
```

OIDC authenticates dashboard users only. Requiring control-plane API keys keeps
the data plane separate from the browser session and legacy shared router token.

The stream limit is a process-local semaphore separate from the normal request
service-future limit. The general concurrency layer can release when an Axum
handler returns a response; the stream permit is deliberately wrapped around
the response body so a slow or abandoned client continues to occupy capacity
until completion or body drop. Clients receiving its 429 should honor
`Retry-After`.

PostgreSQL access uses SQLx, Tokio, rustls, bounded pools, and embedded
versioned migrations. Development mode defaults to `prefer` so the private
Compose database remains usable. For any remote or production database, use
`verify-full` and provide a trusted root through the PostgreSQL URL. Merely
using `require` encrypts transport but does not enforce the enterprise hostname
and certificate policy.

At startup, ModelPort migrates the normalized organization/project/environment,
gateway-request, Provider-attempt, budget, and audit schema. Terminal request
rows are the usage source for logs, Dashboard ranges, quota/spend checks, and
management statistics. Auth and low-frequency control definitions may still
use files, but a running server has no memory fallback for the operational
ledger and requires `MODELPORT_DATABASE_URL`. `/readyz` verifies all stores.

`0005_current_operational_schema.sql` preserves existing normalized request and
attempt rows. It backfills conservative values for dimensions absent from the
older schema and derives final Provider, model, protocol, retry count, and
fallback snapshots from existing attempts before adding current constraints.
Historical rows without Provider attempts retain a null Provider snapshot.
Always back up PostgreSQL and exercise the migration against a restored copy
before upgrading production.

Each PostgreSQL request and Provider-attempt row carries an instance lease.
ModelPort renews it throughout non-stream and streaming lifecycles. Startup and
the periodic reconciler terminalize only expired `started` rows as
`lease_expired_unreconciled`; because Provider evidence is unknown after a
crash, those rows retain zero usage and `chargeable=false` pending future
manual evidence or adjustment.

Compose's default URL directly interpolates `MODELPORT_POSTGRES_PASSWORD`
without percent-encoding. Prefer a long URL-safe password containing letters,
digits, `_`, and `-`. If the raw PostgreSQL password contains reserved URL
characters such as `@`, `:`, `/`, `%`, or `#`, set an explicitly percent-encoded
complete `MODELPORT_DATABASE_URL`; keep `MODELPORT_POSTGRES_PASSWORD` as the raw
password used to initialize PostgreSQL.

## Security And Network

| Variable | Default | Meaning |
| --- | --- | --- |
| `MODELPORT_TRUSTED_PROXIES` | loopback | Comma-separated proxy IPs/CIDRs allowed to supply forwarded client IP headers. |
| `MODELPORT_ALLOWED_ORIGINS` | unset | Extra comma-separated absolute HTTP(S) origins accepted for dashboard write checks. Entries are scheme + host + optional port only; userinfo, path, query, and fragment are rejected. This does not enable CORS. |
| `MODELPORT_DISABLE_CSRF` | off | Emergency local-debug bypass for dashboard write protection. |
| `MODELPORT_EXPOSE_DETAILED_HEALTH` | off | Expose detailed `/health` without authentication. |
| `MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS` | off | Allow literal private provider addresses. IPv4-mapped IPv6 literals are normalized before this check. Use only on trusted networks. |
| `MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP` | off | Permit plain HTTP for non-local/non-custom Providers. Emergency trusted-network override; HTTPS is the safe default. |
| `MODELPORT_INCLUDE_UNAVAILABLE_PROVIDERS` | off | Keep file-config providers that lack required keys; useful for diagnostics, not normal routing. |

Forwarded headers are considered only when the connected peer matches
`MODELPORT_TRUSTED_PROXIES`. ModelPort appends that peer to the received
`X-Forwarded-For` chain, walks from right to left, removes explicitly trusted
proxy hops, and uses the first untrusted address. Do not trust an entire client
network just to make forwarding work. A single-hop proxy should overwrite XFF
with its observed `$remote_addr` instead of preserving an untrusted incoming
chain.

Before every outbound Provider request, ModelPort resolves the hostname, rejects
an answer set containing a private, link-local, metadata, or unspecified address
unless that Provider is explicitly allowed to use private networking, and pins
the original hostname to the validated addresses for the connection. Redirects
and environment HTTP proxies are disabled so they cannot trigger an independent
destination lookup. Keep an outbound firewall or allowlist as defense in depth,
especially when administrators may approve private Provider URLs.

Provider base URLs must be clean HTTP(S) API bases. Validation rejects userinfo,
query strings, and fragments; the transport also refuses redirects. Do not put
API keys or other credentials in a URL query or authority. Configure the
documented key environment variable so the adapter sends credentials in the
protocol header.

Remote Provider records must use `https://` by default. Plain `http://` sends
the Provider API key, request content, and response content without transport
encryption; any host or network device on the path can read or alter them. Set
`MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` only for an explicitly trusted
internal upstream whose network boundary you control, and prefer TLS even
there. Providers classified as local/custom (`custom`, `ollama`, and
`local_*`) may still use HTTP for loopback or local-runtime integration. The
override does not weaken private/metadata-IP checks; those remain controlled
separately by `MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS`.

## HTTP Transport

| Variable | Default | Meaning |
| --- | --- | --- |
| `MODELPORT_HTTP_CONNECT_TIMEOUT_SECS` | `10` | Upstream connect timeout. |
| `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` | `600` | Complete non-stream timeout and total upstream SSE lifecycle timeout, including connection, response headers, and event-body reads. |
| `MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS` | `300` | Maximum silence between upstream stream chunks after the handshake. |
| `MODELPORT_HTTP_MAX_RESPONSE_BYTES` | `33554432` | Maximum non-stream/error body accepted from upstream. |
| `MODELPORT_HTTP_SSE_MAX_LINE_BYTES` | `1048576` | Maximum buffered SSE line. |
| `MODELPORT_HTTP_SSE_MAX_EVENT_BYTES` | `8388608` | Maximum bytes accumulated for one SSE event. |
| `MODELPORT_HTTP_SSE_MAX_STREAM_BYTES` | `67108864` | Maximum raw bytes accepted for one upstream stream. |
| `MODELPORT_HTTP_USER_AGENT` | `model-port/<version>` | Upstream User-Agent override. |

Upstream redirects are disabled. A live SSE stream is bounded by the total
request timeout measured from the outbound request start. Each event-body read
uses the smaller of the remaining total time and the resettable per-chunk idle
timeout. Line, event, and total-stream byte limits apply independently.
Increasing byte or timeout limits increases memory/connection exposure; change
them deliberately.

An SSE handshake requires a 2xx status other than 204 and the
`text/event-stream` media type. The request timeout covers connection through
response headers. Non-2xx and wrong-content-type error bodies are then bounded
by `MODELPORT_HTTP_MAX_RESPONSE_BYTES` and by both a total body-read timeout
using the request-timeout value and the resettable stream-idle timeout, so a
slow-drip error cannot hold the connection indefinitely. Established event
streams use the total request timeout, idle timeout, and SSE byte limits and
must still end with the protocol's required termination event.

## Rate Limits And Request Guardrails

| Variable | Default | Meaning |
| --- | --- | --- |
| `MODELPORT_RATE_LIMIT_DISABLED` | off | Disable every process-local rate dimension. |
| `MODELPORT_RATE_LIMIT_WINDOW_SECONDS` | `60` | Sliding-window duration. |
| `MODELPORT_RATE_LIMIT_GLOBAL_PER_MINUTE` | `6000` | Global request count per configured window. |
| `MODELPORT_RATE_LIMIT_API_KEY_PER_MINUTE` | `600` | Identity count per window. |
| `MODELPORT_RATE_LIMIT_IP_PER_MINUTE` | `1200` | Client-IP count per window. |
| `MODELPORT_RATE_LIMIT_PROVIDER_PER_MINUTE` | `3000` | Resolved-provider count per window. |
| `MODELPORT_RATE_LIMIT_MODEL_PER_MINUTE` | `1200` | Resolved-model count per window. |
| `MODELPORT_MAX_MODEL_NAME_CHARS` | `240` | Maximum model-name characters. |
| `MODELPORT_MAX_MESSAGES` | `200` | Maximum messages per request. |
| `MODELPORT_MAX_MESSAGES_JSON_CHARS` | `2097152` | Maximum serialized messages characters. |
| `MODELPORT_MAX_SYSTEM_JSON_CHARS` | `262144` | Maximum serialized system characters. |
| `MODELPORT_MAX_TOOLS` | `256` | Maximum Tool Use definitions. |
| `MODELPORT_MAX_TOOLS_JSON_CHARS` | `1048576` | Maximum serialized tools characters. |
| `MODELPORT_MAX_OUTPUT_TOKENS` | `131072` | Maximum accepted Anthropic `max_tokens` or OpenAI `max_completion_tokens`/`max_tokens`. |

Every `POST /v1/messages` request must include integer `max_tokens > 0` and the
value must be at most `MODELPORT_MAX_OUTPUT_TOKENS`. This is validated locally
before Provider routing. The Provider's `max_tokens_field` only selects the
outbound OpenAI-compatible field name; it does not make the client field
optional or change the global cap.

`POST /v1/chat/completions` accepts optional `max_completion_tokens` or legacy
`max_tokens`; if both are present they must agree, and any supplied value must
be positive and within the same global cap. An omitted value uses a bounded
4096-token local estimate and is rendered explicitly only when the selected
Provider contract requires it.

A per-window rate value of `0` disables that dimension. Rate limits are
process-local and reset on restart. User quotas and API-key/team spend limits
are persisted. Before Provider egress, PostgreSQL serializes competing scopes
and atomically admits both the tenant budget and the request's usage
reservation in the same transaction as attempt creation. Admission counts
settled usage plus open reservations; terminal requests settle actual usage or
release a non-chargeable/expired reservation.

## Provider Environment Pattern

The complete built-in catalog and current defaults are in
[Providers](PROVIDERS.md). Most providers use:

```text
<PROVIDER>_API_KEY
<PROVIDER>_BASE_URL
<PROVIDER>_MODEL
<PROVIDER>_MODELS=model-a,model-b
MODELPORT_ENABLE_<PROVIDER>=1
```

Names that intentionally differ include:

| Provider | Credential | Base URL | Model |
| --- | --- | --- | --- |
| `cpa_codex` | `CPA_CODEX_API_KEY` | `CPA_CODEX_BASE_URL` | `CPA_CODEX_MODEL` |
| `cpa_claude` | `CPA_CLAUDE_API_KEY` | `CPA_CLAUDE_BASE_URL` | `CPA_CLAUDE_MODEL` |
| `deepseek` | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN` (fallback `DEEPSEEK_API_KEY`) | `DEEPSEEK_ANTHROPIC_BASE_URL` | `DEEPSEEK_MODEL` |
| `deepseek_openai` | `DEEPSEEK_OPENAI_API_KEY` (fallback `DEEPSEEK_API_KEY`) | `DEEPSEEK_OPENAI_BASE_URL` | `DEEPSEEK_OPENAI_MODEL` |
| `mimo` | `MIMO_OPENAI_API_KEY` | `MIMO_OPENAI_BASE_URL` (fallback `BASE_URL`) | `MIMO_MODEL` |
| `anthropic` | `ANTHROPIC_API_KEY` | `ANTHROPIC_UPSTREAM_BASE_URL` | `ANTHROPIC_UPSTREAM_MODEL` |
| `openai` | `MODELPORT_OPENAI_API_KEY` (legacy fallback `OPENAI_API_KEY`) | `MODELPORT_OPENAI_BASE_URL` (legacy fallback `OPENAI_BASE_URL`) | `MODELPORT_OPENAI_MODEL` (legacy fallback `OPENAI_MODEL`) |
| `gemini` | `GEMINI_API_KEY` (fallback `GOOGLE_API_KEY`) | `GEMINI_OPENAI_BASE_URL` | `GEMINI_MODEL` |
| `dashscope` | `DASHSCOPE_API_KEY` (fallback `QWEN_API_KEY`) | `DASHSCOPE_BASE_URL` | `DASHSCOPE_MODEL` |
| `kimi` | `MOONSHOT_API_KEY` (fallback `KIMI_API_KEY`) | `KIMI_BASE_URL` | `KIMI_MODEL` |
| `ark` | `ARK_API_KEY` (fallback `VOLCENGINE_API_KEY`) | `ARK_BASE_URL` | `ARK_MODEL` |

Catalog variables are:

```text
CPA_CODEX_MODELS
CPA_CLAUDE_MODELS
DEEPSEEK_MODELS
DEEPSEEK_OPENAI_MODELS
MIMO_MODELS
ANTHROPIC_UPSTREAM_MODELS
MODELPORT_OPENAI_MODELS
OPENROUTER_MODELS
GEMINI_MODELS
XAI_MODELS
GROQ_MODELS
DASHSCOPE_MODELS
KIMI_MODELS
ZHIPU_MODELS
MISTRAL_MODELS
ARK_MODELS
OLLAMA_MODELS
CUSTOM_OPENAI_MODELS
SGLANG_MODELS
VLLM_MODELS
LLAMACPP_MODELS
```

The `MODELPORT_OPENAI_*` namespace is deliberately server-specific. Standard
`OPENAI_BASE_URL`, `OPENAI_API_KEY`, `OPENAI_MODEL`, and `OPENAI_MODELS` remain
fallbacks for compatibility, but using one without its `MODELPORT_OPENAI_*`
counterpart produces a configuration warning. New deployments should reserve
standard `OPENAI_*` variables for SDK/client processes. A configured `openai`
Provider whose `/v1` base URL points to the same local listener as
`MODELPORT_BIND` is rejected as a self-referential routing loop.

Local runtimes use `SGLANG_*`, `VLLM_*`, `LLAMACPP_*`, or `OLLAMA_*` and are
enabled with the corresponding `MODELPORT_ENABLE_*` flag. `custom` is enabled
by a custom URL, model, key, or `MODELPORT_ENABLE_CUSTOM=1`.
Optional runtime credentials are `SGLANG_API_KEY`, `VLLM_API_KEY`,
`LLAMACPP_API_KEY`, and `OLLAMA_API_KEY`; set `api_key_required=true` in TOML
when the runtime must reject unauthenticated calls.

### Provider Activation

- `deepseek` is always inserted in environment-default mode because it is the
  configured sample/default path; validation fails when it is the default and
  its required credential is absent.
- Other built-ins activate when an enable flag, base URL, model, or credential
  (including a fallback name) is present.
- `BASE_URL` can activate `mimo`; avoid exporting a generic value unintentionally.
- In TOML mode, providers that require a missing key are filtered unless they
  are the configured default or `MODELPORT_INCLUDE_UNAVAILABLE_PROVIDERS=1`.
- TOML providers with `api_key_required=false` remain visible even if their
  local runtime is offline. Catalog visibility is not a health check.
- `MODELPORT_<PROVIDER_ID>_BUFFER_STREAM_TEXT=1` enables the built-in buffered
  generation path for that provider ID. It awaits and converts a complete
  non-stream upstream response before creating local SSE, so pre-header errors
  can fallback and reported usage can be accounted. This is a compatibility
  escape hatch with full-generation time to first byte, not a normal
  performance setting.

Dashboard credential profiles store an environment-variable name, never its
plaintext value. A newly added variable must exist in the process environment;
editing a mounted `.env` file alone does not add a new process variable. Recreate
or restart the service after adding credential-profile variables.

Dashboard/API partial Provider updates distinguish omission from clearing:
omitting camelCase `apiKeyEnv` preserves the current name, while
`clearApiKeyEnv: true` removes it. A non-empty `apiKeyEnv` and the clear flag
cannot be sent together. This control-plane flag is not a TOML field; TOML uses
the declarative `api_key_env` value shown below.

Credential pool modes are `manual`, `failover`, and `round_robin`. `manual`
retains the explicitly selected non-disabled record. The two automatic modes
consider only active credentials whose environment value exists and whose
cooldown has expired. If a configured automatic pool has no usable credential,
the Provider fails closed and routing can try another Provider candidate; it
does not silently reuse an unusable account.

## TOML Provider Fields

```toml
[providers.example]
display_name = "Example"
protocol = "openai-compat" # or "anthropic"
base_url = "https://provider.example/v1"
api_key_env = "EXAMPLE_API_KEY"
api_key_required = true
default_model = "example-model"
models = ["example-model"]
model_prefixes = ["example-"]
passthrough_unknown_models = false
max_tokens_field = "max_completion_tokens" # max_tokens | both
deduplicate_stream_text = false
buffer_stream_text = false
fidelity_mode = "best_effort" # strict | best_effort | stability
request_timeout_ms = 600000       # optional; non-stream and SSE handshake/total timeout
stream_idle_timeout_ms = 300000   # optional; resets after each SSE data chunk

# Only non-sensitive, static attribution headers are allowed. Authentication,
# cookies, Host/framing, forwarding, request-ID, trace, and User-Agent headers
# are reserved and rejected during validation.
[providers.example.static_headers]
HTTP-Referer = "https://modelport.example"
X-Title = "ModelPort"

[providers.example.retry]
max_attempts = 2       # includes the first request; 1 disables same-Provider retry
initial_delay_ms = 250
max_delay_ms = 5000    # also caps an upstream Retry-After; absolute maximum 60000
jitter_ratio = 0.1

[providers.example.tool_use]
supported = true
tool_choice = true
parallel_tool_calls = true
streaming_arguments = "delta"
response_validation = "best_effort"
repair_invalid_arguments = false

# Optional llama.cpp request-level thinking mapping. This is valid only for an
# OpenAI-compatible provider; omit it for providers without this extension.
[providers.example.reasoning]
mode = "llama_cpp"
default_enabled = false
model_enabled = { "example-fast" = false, "example-code" = true, "example-deep" = true }
default_effort = "high"
model_effort = { "example-fast" = "off", "example-deep" = "max" }
default_budget_tokens = 4096
model_budget_tokens = { "example-fast" = 512, "example-deep" = 16384 }

# Optional per-Provider and exact-model adaptation metadata. `models` remains
# the route allowlist and order; profiles do not authorize a model by themselves.
[providers.example.model_profile_defaults]
input_modalities = ["text"]
tool_use = "supported"          # supported | unsupported | unknown
tool_choice = "supported"
parallel_tool_calls = "supported"
strict_tool_schema = "unknown"
reasoning = "unknown"
reasoning_dialect = "openai"    # see the reasoning dialect list below
reasoning_replay = "same_protocol"

[providers.example.model_profiles."example-model"]
display_name = "Example Model"
family = "example"
context_window = 131072
max_output_tokens = 32768
reasoning = "supported"
reasoning_efforts = ["off", "low", "medium", "high", "max"]
default_reasoning_effort = "medium"

[providers.example.sampling]
mode = "llama_cpp"

[providers.example.sampling.profiles."example-code"]
temperature = 0.6
top_p = 0.95
top_k = 20
min_p = 0.0
presence_penalty = 0.0
repeat_penalty = 1.0

# Optional exact Anthropic Count Tokens forwarding. The upstream must expose
# the corresponding endpoint; unsupported providers should leave this absent.
[providers.example.token_counting]
mode = "anthropic"
context_tokens = 131072
recommended_reasoning_input_tokens = 94208
model_recommended_input_tokens = { "example-fast" = 24576, "example-deep" = 94208 }
max_output_tokens = 32768
model_max_output_tokens = { "example-fast" = 4096, "example-deep" = 32768 }

[providers.example.pricing]
input_per_million = 1.0
output_per_million = 4.0
cache_write_per_million = 1.0
cache_read_per_million = 0.1

[providers.example.model_pricing."example-model"]
input_per_million = 1.0
output_per_million = 4.0
cache_write_per_million = 1.0
cache_read_per_million = 0.1
version = "provider-public-2026-08-01-v1"
effective_at = "2026-08-01T00:00:00Z"
currency = "USD"
source = "provider_published"
service_tier = "standard"
evidence = "https://provider.example/pricing#example-model"
```

`pricing` is the legacy provider-wide USD estimate. It overrides the built-in
model-family estimate for admission, routing, and capacity reservation, but it
is never settled or presented as an invoice-grade charge.

`model_pricing.<exact-model>` is the governed rate-card surface. The key must
appear verbatim in the Provider's `models` list. Each card records its version,
RFC3339 UTC effective timestamp, USD currency, source, standard service tier, and a durable
evidence reference. Supported sources are `provider_published`,
`provider_contract`, and `internal_chargeback`; `legacy_estimate` is rejected.
Until request-tier matching exists, non-standard service tiers are rejected
instead of silently applying the wrong rate. Add a new card version when rates
change; completed ledger rows retain their applied price and evidence snapshot.

`input_per_million` and `output_per_million` apply to ordinary prompt and
generated tokens. `cache_write_per_million` and `cache_read_per_million` apply
only when the upstream reports those token classes. Set all four values to zero
for an intentionally free but still governed internal chargeback.

For an OpenAI-compatible aggregator that returns an authoritative USD
`usage.cost`, set `trust_upstream_cost = true` on that Provider. This is enabled
by default only for the built-in OpenRouter profile. Do not enable it for an
unreviewed proxy: a trusted Provider-reported amount takes precedence over a
local exact-model card.

`fidelity_mode="stability"` is a label for a provider configured with stream
rewriting; it does not enable deduplication by itself. Set
`deduplicate_stream_text` or `buffer_stream_text` explicitly.

The embedded, versioned adaptation catalog is the first layer for known
Provider/model combinations. Effective model metadata is merged in this order:
catalog, Provider `model_profile_defaults`, TOML `model_profiles`, then an
administrator control-plane override. `/models` discovery may add a reviewed
candidate to inventory but never authorizes routing and never changes a
capability to verified. Capability values are tri-state. Advanced behavior such
as explicit reasoning, strict tool schemas, or parallel tool calls is used only
when the effective model profile says `supported`; `unknown` fails closed for
that behavior while ordinary text requests remain compatible.

Reasoning effort values are `off`, `minimal`, `low`, `medium`, `high`, `xhigh`,
and `max`. An explicit client control wins over logical-model defaults, exact
model defaults, and Provider defaults. OpenAI clients send `reasoning_effort`;
Anthropic clients keep native `thinking`. Known OpenAI-compatible dialects are
`openai`, `deepseek`, `openrouter`, `qwen`, `zai`, `string_thinking`, and
`llama_cpp`; native Anthropic providers use `native_anthropic`. ModelPort does
not silently choose a nearby effort when the requested level is absent.
`reasoning_effort_map` can map a portable effort to an exact upstream string.
Budget and effort remain separate: a `thinking.budget_tokens` value is rejected
when the selected dialect cannot represent it.

Reasoning replay is protocol state. Native Anthropic thinking signatures and
OpenAI-compatible `reasoning_content` are preserved only on a lossless path.
They are never flattened into ordinary assistant text or fabricated. A
cross-protocol tool continuation is rejected before egress when its effective
profile requires `same_protocol` replay; use the Provider's native protocol for
that agent workflow.

The Provider retry loop owns each billable attempt and ledger row. Only
transport failures, HTTP 429, and HTTP 5xx are retried. Authentication, quota,
ordinary invalid requests, and protocol/schema failures are not automatically
retried. A numeric or HTTP-date `Retry-After` is honored within
`retry.max_delay_ms` and the global 60-second bound. Once a streaming response
has crossed the downstream header boundary, ModelPort never starts a fallback.

`tool_use.streaming_arguments` is a runtime Tool Use argument strategy. For an
OpenAI-compatible provider, `delta` preserves incremental argument fragments,
while `cumulative` and `best_effort` enable replay deduplication and recovery of
the best complete JSON object available at stream completion. `native` is the
normal Anthropic pass-through mode. These settings cannot prove that an
upstream implements the advertised behavior; certify each provider/model with
real acceptance calls.

`tool_use.response_validation` defaults to `best_effort`. Set it to `strict`
for a trusted local or certified OpenAI-compatible runtime: ModelPort then
rejects missing or undeclared function names, non-object or invalid JSON
arguments, duplicate call IDs, `tool_choice`/parallel-count violations, and
inconsistent tool-call finish reasons. In a live stream, a violation is
reported as an Anthropic `error` event after the SSE handshake.

`tool_use.repair_invalid_arguments` defaults to `false` and is valid only for
an OpenAI-compatible provider with `response_validation="strict"`. For a
non-stream Anthropic Messages request, ModelPort may make exactly one additional
attempt against the same provider when the first tool call fails its declared
JSON Schema. The retry prompt contains neither arguments nor validation paths,
the failed candidate is never delivered, both attempts enter the ledger and
token accounting, and the repair reserves 256 context tokens during admission.
It never retries streams, transport/auth/time-out failures, tool execution, or
arbitrary protocol failures.

[`config.example.toml`](../config.example.toml) is intentionally minimal and
self-contained around DeepSeek. When extending it, keep aliases limited to
enabled providers: an alias targeting a provider filtered out for a missing key
is a validation error.

`reasoning.mode="llama_cpp"` translates Anthropic Messages `thinking` into the
llama.cpp OpenAI-compatible extensions. `thinking.type="disabled"` sends
`chat_template_kwargs.enable_thinking=false`; `enabled` or `adaptive` enables
thinking and sends `thinking_budget_tokens`. Budget precedence is the explicit
request value, then the requested ModelPort alias in `model_budget_tokens`, then
`default_budget_tokens`. Optional `default_enabled` is the Provider fallback
when the client protocol has no portable thinking control, especially OpenAI
Chat Completions. `model_enabled` overrides that fallback by requested logical
model, so one loaded runtime can keep `fast` non-thinking while enabling
`code`/`deep` by default. An explicit Anthropic `thinking` value always wins.
Omit both enable settings to preserve the runtime default. The resolved upstream
model ID is unchanged, so these aliases do not add model memory. Providers
without this explicitly configured mode retain their existing native behavior.

`sampling.mode="llama_cpp"` applies a profile selected by the originally
requested ModelPort model or alias. Supported profile defaults are
`temperature`, `top_p`, `top_k`, `min_p`, `presence_penalty`, and
`repeat_penalty`. Explicit client values already present in the converted
request take precedence; unlisted models are unchanged. Profiles are valid only
for OpenAI-compatible providers because `min_p` and `repeat_penalty` are
llama.cpp extensions. Validation rejects empty profile names, non-finite values,
and unsafe ranges before reload.

`token_counting.mode="anthropic"` enables authenticated
`POST /v1/messages/count_tokens` for that Provider. ModelPort rewrites aliases
to the resolved upstream model and forwards the Anthropic Count Tokens body to
the Provider's native endpoint. It returns only the Provider-reported integer
`input_tokens`; it never substitutes the local characters/4 usage heuristic.
The mode is opt-in because many OpenAI-compatible runtimes do not implement
this endpoint. Token counting is rate-limited but does not create an inference
ledger/usage charge and does not fall back to a different tokenizer.

When `context_tokens` is set, Anthropic Messages inference performs an exact
upstream count before generation and rejects `input_tokens + max_tokens` above
that limit with an actionable error; input is never silently truncated.
`recommended_reasoning_input_tokens` adds a stricter input ceiling while
thinking is enabled so the model retains room for reasoning and final text.
`model_recommended_input_tokens` applies a more conservative ceiling to the
originally requested logical model or alias, falling back to the Provider-wide
recommendation. `max_output_tokens` and `model_max_output_tokens` similarly
reject oversized output requests before exact counting or Provider traffic;
the logical-model value takes precedence over the Provider-wide value.
Explicit `thinking.type="disabled"` bypasses only the recommendation, never the
hard context or output limit. OpenAI Chat Completions cannot use exact input
admission because converting it to an Anthropic count body would not be
lossless, but its configured output limits are still enforced.

## Reload Versus Restart

| Change | Reload | Restart/recreate |
| --- | --- | --- |
| Base provider URL/key/model list/pricing | Yes for TOML or an env-file value not shadowed by the process | Recreate when changing an existing process variable |
| TOML aliases and provider order | Yes | — |
| Dashboard provider/model/alias/default/order overrides | Applied immediately | — |
| New credential-profile environment variable | No | Yes |
| Bind address, body limit, request/stream concurrency layers | No | Yes |
| HTTP client timeouts, response limit, User-Agent | No | Yes |
| Rate-limit values/window | No | Yes |
| Trusted proxies, health exposure, private/insecure-URL policy, CSRF/origin policy | No | Yes |
| Admin bootstrap, session TTL, secure-cookie flag | No | Yes |
| Storage backend or state paths | No | Yes |

Reload from the dashboard Operations tab or restart the service. A successful
reload validates the new base snapshot but does not mutate `.env` or TOML.
Because process values win, editing a key that Compose already loaded from
`env_file` has no effect until the container is recreated.

## API Key tenant binding

Every Control API Key has one request-ledger scope:
`organizationId/projectId/environmentId`. New and pre-upgrade keys start at
`org_local/prj_default/env_default`; an administrator binds a dedicated key
before a consuming application declares another scope:

```http
PUT /admin/api-keys/{key_id}/scope
Content-Type: application/json

{
  "organizationId": "org_local",
  "projectId": "prj_quantpilot",
  "environmentId": "env_development"
}
```

The endpoint is admin-only, validates a complete bounded tuple, persists it in
the Control Store, and emits an audit activity. The first PostgreSQL request
auto-provisions the normalized catalog rows for that trusted binding.

Clients may send all three assertion headers:

```text
X-ModelPort-Organization-Id: org_local
X-ModelPort-Project-Id: prj_quantpilot
X-ModelPort-Environment-Id: env_development
```

Omitting them uses the key binding. A partial tuple is 400; a different tuple is
403. Headers never create authority. Give every consuming application its own
key and ModelPort project; do not share QuantPilot's key with future products.

## Client, Compose, Script, And Dashboard Variables

These names are consumed outside the backend configuration loader:

| Variable | Consumer | Meaning |
| --- | --- | --- |
| `ANTHROPIC_BASE_URL` | Claude client | ModelPort API origin. |
| `ANTHROPIC_AUTH_TOKEN` | Claude client; server fallback | Client token; also the server token fallback when `MODELPORT_AUTH_TOKEN` is absent. |
| `ANTHROPIC_MODEL`, `ANTHROPIC_DEFAULT_*_MODEL`, `ANTHROPIC_SMALL_FAST_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL` | Claude client | Client-side selected model names. |
| `MODELPORT_API_PUBLISH`, `MODELPORT_DASHBOARD_PUBLISH` | Compose | Host publish address/port. |
| `MODELPORT_POSTGRES_DB`, `MODELPORT_POSTGRES_USER`, `MODELPORT_POSTGRES_PASSWORD` | Compose/PostgreSQL | Internal database bootstrap. |
| `MODELPORT_IMAGE`, `MODELPORT_DASHBOARD_IMAGE` | Compose | Release images; use version tags for evaluation and immutable digests for shared/production use. |
| `MODELPORT_PULL_POLICY` | Compose | Image pull policy; root source-build Compose defaults to `never`, release Compose to `missing`. Production upgrades set `always`. |
| `MODELPORT_COMPOSE_FILE` | `scripts/compose-up.sh`, `scripts/doctor.sh` | Selected Compose manifest; defaults to the root source-build profile. |
| `MODELPORT_LOCAL_BUILD` | `scripts/compose-up.sh` | Set to `1` after `scripts/build-container.sh` to select the two `:local` images and disable pulls. |
| `MODELPORT_HEALTHCHECK_API_KEY` | Compose healthcheck | Dedicated scoped key for authenticated `/readyz`; local Compose falls back to the legacy router token when omitted. |
| `MODELPORT_STOP_GRACE_PERIOD` | Compose | Backend SIGTERM-to-SIGKILL window; defaults to 11 minutes. |
| `MODELPORT_RUNTIME_ENV_FILE`, `MODELPORT_CONFIG_FILE`, `MODELPORT_DATABASE_CA_FILE`, `MODELPORT_OWNERSHIP_FILE` | Production Compose/preflight | Operator-owned runtime secret, reviewed config, PostgreSQL CA, and named operations-ownership paths required by the single-instance production profile. |
| `RUST_LOG` | tracing | Backend log filter. |
| `MODELPORT_RUNTIME_DIR`, `MODELPORT_PID_FILE`, `MODELPORT_LOG_FILE` | local scripts | Background process files. |
| `MODELPORT_FORCE_BUILD` | local scripts | Force rebuilding a local binary. |
| `MODELPORT_DASHBOARD_URL` | acceptance | Dashboard origin to check. |
| `MODELPORT_TOOL_USE_MOCK_HOST` | Tool Use acceptance | Hostname reachable by the backend for the temporary mock. |
| `MODELPORT_CHECK_NPM_CI` | aggregate checks | Force a clean locked dashboard install. |
| `MODELPORT_VITE_PROXY_TARGET` | Vite dev/E2E | Backend origin for Vite's same-origin proxy; defaults to `http://127.0.0.1:38082`. |
| `VITE_MODELPORT_MOCK` | dashboard build/dev | UI mock mode; never enable for production. |
| `VITE_API_BASE_URL` | dashboard build | Browser API prefix/origin. Cross-origin use requires a separately designed CORS proxy. |
| `PLAYWRIGHT_BASE_URL`, `PLAYWRIGHT_SKIP_WEBSERVER` | Playwright | E2E target and dev-server control. |

Client model variables do not reconfigure the server catalog. A client name must
resolve through an enabled provider, alias, exact model, prefix, or intentional
unknown-model passthrough.

Variables beginning `MODELPORT_TEST_` are test-only implementation details and
are not supported deployment configuration.
