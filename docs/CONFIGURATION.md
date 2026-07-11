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

## Required Minimum

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

Validate before startup:

```bash
scripts/config-validate.sh
```

The CLI and normal server startup both load `AppConfig` and evaluate the same
`validation_issues()` boundary. Any `Error` makes `config validate` exit
non-zero and makes the server refuse to bind; `Warning` entries are printed by
the CLI and logged by the server while startup continues. Base-config reload
also rejects a candidate with validation errors.

Placeholder secrets, an invalid/missing default provider, broken aliases,
unsafe provider definitions, and malformed guardrail values therefore do not
silently enter service. Numeric environment variables are checked as unsigned
integers and, where zero has no safe meaning, as greater than zero. In
particular `MODELPORT_MAX_REQUEST_BODY_BYTES`,
`MODELPORT_MAX_CONCURRENT_REQUESTS`, and `MODELPORT_USAGE_LOG_LIMIT` must be
non-zero, as must the documented request-size, session, HTTP timeout/body, and
SSE byte guardrails. Rate limiting has its separate explicit disable switch.

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
| `MODELPORT_STATE_DIR` | `.modelport` for auth | Base state directory used by the auth store. |
| `MODELPORT_AUTH_STORE_PATH` | `<state-dir>/admin-auth.json` | File backend override for auth state. |
| `MODELPORT_CONTROL_STORE` | `.modelport/control-plane.json` | File backend path for control state. |
| `MODELPORT_DATABASE_URL` | unset | Store both state documents in PostgreSQL instead of files; Compose constructs an internal default unless explicitly overridden. |
| `MODELPORT_USAGE_LOG_LIMIT` | `5000` | Maximum retained request-usage records in the control document; must be greater than zero. |

Bootstrap variables do not overwrite existing users. Dashboard sessions are
process-local and are invalidated by restart.

The stream limit is a process-local semaphore separate from the normal request
service-future limit. The general concurrency layer can release when an Axum
handler returns a response; the stream permit is deliberately wrapped around
the response body so a slow or abandoned client continues to occupy capacity
until completion or body drop. Clients receiving its 429 should honor
`Retry-After`.

The current PostgreSQL client connects with `NoTls`. Use it only on the same
host, the private Compose bridge, or another network path already protected by
an appropriate trust boundary or tunnel; do not send database credentials/state
over an untrusted network. Native PostgreSQL TLS transport remains future work.

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
| `MODELPORT_ALLOWED_ORIGINS` | unset | Extra comma-separated origins accepted for dashboard write checks. This does not enable CORS. |
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

Provider URL filtering does not currently resolve hostnames and reject private
DNS answers. Use an outbound firewall or allowlist when untrusted administrators
can edit provider URLs.

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
| `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` | `600` | Complete non-stream request timeout; for SSE, only the `send()`/response-header handshake timeout. |
| `MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS` | `300` | Maximum silence between upstream stream chunks after the handshake. |
| `MODELPORT_HTTP_MAX_RESPONSE_BYTES` | `33554432` | Maximum non-stream/error body accepted from upstream. |
| `MODELPORT_HTTP_SSE_MAX_LINE_BYTES` | `1048576` | Maximum buffered SSE line. |
| `MODELPORT_HTTP_SSE_MAX_EVENT_BYTES` | `8388608` | Maximum bytes accumulated for one SSE event. |
| `MODELPORT_HTTP_SSE_MAX_STREAM_BYTES` | `67108864` | Maximum raw bytes accepted for one upstream stream. |
| `MODELPORT_HTTP_USER_AGENT` | `model-port/<version>` | Upstream User-Agent override. |

Upstream redirects are disabled. A live SSE stream has no fixed wall-clock total
timeout after its response headers arrive: the request timeout no longer applies
then. Its lifetime is bounded by the per-chunk idle timeout and the line, event,
and total-stream byte limits. A stream that continues delivering chunks within
all limits can therefore run longer than `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS`.
Increasing byte or timeout limits increases memory/connection exposure; change
them deliberately.

An SSE handshake requires a 2xx status other than 204 and the
`text/event-stream` media type. The request timeout covers connection through
response headers. Non-2xx and wrong-content-type error bodies are then bounded
by `MODELPORT_HTTP_MAX_RESPONSE_BYTES` and by both a total body-read timeout
using the request-timeout value and the resettable stream-idle timeout, so a
slow-drip error cannot hold the connection indefinitely. Established event
streams use the idle and SSE
byte limits and must still end with the protocol's required termination event.

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
| `MODELPORT_MAX_OUTPUT_TOKENS` | `131072` | Maximum accepted `max_tokens`. |

Every `POST /v1/messages` request must include integer `max_tokens > 0` and the
value must be at most `MODELPORT_MAX_OUTPUT_TOKENS`. This is validated locally
before Provider routing. The Provider's `max_tokens_field` only selects the
outbound OpenAI-compatible field name; it does not make the client field
optional or change the global cap.

A per-window rate value of `0` disables that dimension. Rate limits are
process-local and reset on restart. Quotas and spend limits are persisted, but
the current pre-check and post-request update are not a reservation transaction;
concurrent requests can overshoot a tight budget.

## Provider Environment Pattern

The complete built-in catalog and current defaults are in
[Provider Compatibility Matrix](PROVIDER_MATRIX.md). Most providers use:

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
| `deepseek` | `DEEPSEEK_ANTHROPIC_AUTH_TOKEN` (fallback `DEEPSEEK_API_KEY`) | `DEEPSEEK_ANTHROPIC_BASE_URL` | `DEEPSEEK_MODEL` |
| `deepseek_openai` | `DEEPSEEK_OPENAI_API_KEY` (fallback `DEEPSEEK_API_KEY`) | `DEEPSEEK_OPENAI_BASE_URL` | `DEEPSEEK_OPENAI_MODEL` |
| `mimo` | `MIMO_OPENAI_API_KEY` | `MIMO_OPENAI_BASE_URL` (fallback `BASE_URL`) | `MIMO_MODEL` |
| `anthropic` | `ANTHROPIC_API_KEY` | `ANTHROPIC_UPSTREAM_BASE_URL` | `ANTHROPIC_UPSTREAM_MODEL` |
| `gemini` | `GEMINI_API_KEY` (fallback `GOOGLE_API_KEY`) | `GEMINI_OPENAI_BASE_URL` | `GEMINI_MODEL` |
| `dashscope` | `DASHSCOPE_API_KEY` (fallback `QWEN_API_KEY`) | `DASHSCOPE_BASE_URL` | `DASHSCOPE_MODEL` |
| `kimi` | `MOONSHOT_API_KEY` (fallback `KIMI_API_KEY`) | `KIMI_BASE_URL` | `KIMI_MODEL` |
| `ark` | `ARK_API_KEY` (fallback `VOLCENGINE_API_KEY`) | `ARK_BASE_URL` | `ARK_MODEL` |

Catalog variables are:

```text
DEEPSEEK_MODELS
DEEPSEEK_OPENAI_MODELS
MIMO_MODELS
ANTHROPIC_UPSTREAM_MODELS
OPENAI_MODELS
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

[providers.example.tool_use]
supported = true
tool_choice = true
parallel_tool_calls = true
streaming_arguments = "delta"
```

`fidelity_mode="stability"` is a label for a provider configured with stream
rewriting; it does not enable deduplication by itself. Set
`deduplicate_stream_text` or `buffer_stream_text` explicitly.

`tool_use.streaming_arguments` is a runtime Tool Use argument strategy. For an
OpenAI-compatible provider, `delta` preserves incremental argument fragments,
while `cumulative` and `best_effort` enable replay deduplication and recovery of
the best complete JSON object available at stream completion. `native` is the
normal Anthropic pass-through mode. These settings cannot prove that an
upstream implements the advertised behavior; certify each provider/model with
real acceptance calls.

[`config.example.toml`](../config.example.toml) is intentionally minimal and
self-contained around DeepSeek. When extending it, keep aliases limited to
enabled providers: an alias targeting a provider filtered out for a missing key
is a validation error.

## Reload Versus Restart

| Change | Reload | Restart/recreate |
| --- | --- | --- |
| Base provider URL/key/model list | Yes for TOML or an env-file value not shadowed by the process | Recreate when changing an existing process variable |
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

## Client, Compose, Script, And Dashboard Variables

These names are consumed outside the backend configuration loader:

| Variable | Consumer | Meaning |
| --- | --- | --- |
| `ANTHROPIC_BASE_URL` | Claude client | ModelPort API origin. |
| `ANTHROPIC_AUTH_TOKEN` | Claude client; server fallback | Client token; also the server token fallback when `MODELPORT_AUTH_TOKEN` is absent. |
| `ANTHROPIC_MODEL`, `ANTHROPIC_DEFAULT_*_MODEL`, `ANTHROPIC_SMALL_FAST_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL` | Claude client | Client-side selected model names. |
| `MODELPORT_API_PUBLISH`, `MODELPORT_DASHBOARD_PUBLISH` | Compose | Host publish address/port. |
| `MODELPORT_POSTGRES_DB`, `MODELPORT_POSTGRES_USER`, `MODELPORT_POSTGRES_PASSWORD` | Compose/PostgreSQL | Internal database bootstrap. |
| `RUST_LOG` | tracing | Backend log filter. |
| `MODELPORT_RUNTIME_DIR`, `MODELPORT_PID_FILE`, `MODELPORT_LOG_FILE` | local scripts | Background process files. |
| `MODELPORT_FORCE_BUILD` | local scripts | Force rebuilding a local binary. |
| `MODELPORT_DASHBOARD_URL` | acceptance | Dashboard origin to check. |
| `MODELPORT_TOOL_USE_MOCK_HOST` | Tool Use acceptance | Hostname reachable by the backend for the temporary mock. |
| `MODELPORT_CHECK_NPM_CI` | aggregate checks | Force a clean locked dashboard install. |
| `MODELPORT_VITE_PROXY_TARGET` | Vite dev/E2E | Backend origin for Vite's same-origin proxy; defaults to `http://127.0.0.1:17878`. |
| `VITE_MODELPORT_MOCK` | dashboard build/dev | UI mock mode; never enable for production. |
| `VITE_API_BASE_URL` | dashboard build | Browser API prefix/origin. Cross-origin use requires a separately designed CORS proxy. |
| `PLAYWRIGHT_BASE_URL`, `PLAYWRIGHT_SKIP_WEBSERVER` | Playwright | E2E target and dev-server control. |

Client model variables do not reconfigure the server catalog. A client name must
resolve through an enabled provider, alias, exact model, prefix, or intentional
unknown-model passthrough.

Variables beginning `MODELPORT_TEST_` are test-only implementation details and
are not supported deployment configuration.
