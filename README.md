# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)

**English** | [简体中文](README.zh-CN.md)

ModelPort is a self-hosted Anthropic-compatible model gateway for Claude Code,
VS Code Claude, and API clients. It keeps one local `/v1/messages` endpoint in
front of Anthropic-compatible and OpenAI-compatible providers, with routing,
Tool Use conversion, authentication, quotas, request logs, provider health, and
a small-team dashboard.

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

ModelPort is intended for one trusted host or a small trusted network. It is
not a public multi-tenant model platform, a chat client, or a model runtime.

## Implemented Surface

- Anthropic-compatible `POST /v1/messages` and `GET /v1/models`.
- Anthropic pass-through and OpenAI Chat Completions conversion.
- Anthropic-style SSE conversion, including common Tool Use deltas.
- Model aliases, `provider:model`, exact-model and prefix routing.
- Legacy local token and dashboard-issued API keys with model/provider/IP,
  rolling spend-window, and user-quota policy.
- Provider credential pools, cooldown state, bounded fallback, diagnostics,
  request logs, and Prometheus metrics.
- React dashboard for users, keys, teams, quotas, providers, models, aliases,
  logs, health, audit, and redacted diagnostic snapshots.
- JSON-file or PostgreSQL control state; Docker Compose and systemd templates.

Provider entries are configuration support, not proof of real-upstream
compatibility. Dated verification belongs in the
[provider matrix](docs/PROVIDER_MATRIX.md).

## Technical Core

- **Protocol boundary:** the public contract stays Anthropic Messages while
  focused adapters either pass Anthropic-compatible traffic through or map it
  to OpenAI Chat Completions, including bounded SSE and common Tool Use events.
- **Deterministic routing:** explicit `provider:model`, aliases, exact model
  matches, prefixes, and the default Provider resolve in a documented order.
  Cooling Providers are skipped while an eligible alternative exists, and
  fallback is limited to eligible models and retryable transport/protocol, 429,
  or 5xx failures.
- **Attempt-scoped governance:** authentication and global/identity/IP limits
  run before routing. API-key policy, user quota, API-key/team spend, Provider
  credentials, capability gates, and Provider/model limits are then checked for
  each attempt. Preflight rejection is not charged; only an attempt that was
  actually sent records quota/spend consumption.
- **Defensive transport and streaming:** upstream redirects are disabled;
  request/response/SSE sizes, idle time, and concurrency are bounded; remote
  Providers require HTTPS by default; and live stream permits remain held until
  the response body completes or is dropped.
- **One control-plane truth:** environment/TOML configuration is combined with
  persisted dashboard overrides. JSON-file and PostgreSQL modes store the same
  logical auth/control documents, while the dashboard remains a client of the
  backend rather than a second routing authority.
- **Evidence-aware observability:** request IDs, retained usage logs,
  Prometheus process metrics, health/cooldown state, and dashboard aggregation
  preserve whether usage came from the upstream or a local estimate.

These mechanisms are implemented, but they do not make ModelPort an exact
billing system or a distributed hard-quota service. Provider configuration is
not real-upstream verification, and a live stream can still fail after HTTP 200
without cross-Provider replay. See the [technical core and its
boundaries](docs/ARCHITECTURE.md#technical-core).

## Quick Start With Docker Compose

Requirements: Docker with Compose v2 and credentials for at least one provider.

```bash
cp deploy/docker/modelport.env.example .env
```

Edit `.env` and replace at least these values:

```env
MODELPORT_AUTH_TOKEN=replace-with-a-long-random-local-token
ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
MODELPORT_ADMIN_USERNAME=admin
MODELPORT_ADMIN_PASSWORD=replace-with-a-long-random-admin-password
MODELPORT_POSTGRES_PASSWORD=replace-with-a-long-random-postgres-password

MODELPORT_DEFAULT_PROVIDER=deepseek
DEEPSEEK_ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
DEEPSEEK_ANTHROPIC_AUTH_TOKEN=replace-with-a-real-provider-key
DEEPSEEK_MODEL=deepseek-v4-flash
```

`deepseek-v4-flash` is the repository's configured sample, not a claim that the
model is available to every account. Use the exact model ID enabled by your
provider.

Start and inspect the stack:

```bash
docker compose up -d --build
docker compose ps
docker compose logs -f modelport
```

Open:

- Dashboard: `http://127.0.0.1:5173`
- Liveness: `http://127.0.0.1:17878/livez`
- Messages: `http://127.0.0.1:17878/v1/messages`

Log in with `MODELPORT_ADMIN_USERNAME` and `MODELPORT_ADMIN_PASSWORD`.

## Connect Claude Code

Configure the client with the published API origin and the same router token:

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:17878
ANTHROPIC_AUTH_TOKEN=replace-with-the-same-local-router-token
ANTHROPIC_MODEL=deepseek-v4-flash
ANTHROPIC_DEFAULT_OPUS_MODEL=deepseek-v4-flash
ANTHROPIC_DEFAULT_SONNET_MODEL=deepseek-v4-flash
ANTHROPIC_DEFAULT_HAIKU_MODEL=deepseek-v4-flash
ANTHROPIC_SMALL_FAST_MODEL=deepseek-v4-flash
CLAUDE_CODE_SUBAGENT_MODEL=deepseek-v4-flash
```

For the VS Code Claude extension, place these names in its environment-variable
settings and reload the extension/window. The model values must match your
configured ModelPort catalog.

## Verify

```bash
source .env

curl -fsS http://127.0.0.1:17878/livez

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/v1/models

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  http://127.0.0.1:17878/v1/messages \
  -d '{
    "model":"deepseek-v4-flash",
    "max_tokens":96,
    "messages":[{"role":"user","content":"Reply exactly: OK"}]
  }'
```

The message request is a paid upstream call. Local checks that do not generate
text are:

```bash
scripts/config-validate.sh
scripts/status.sh
scripts/smoke-test.sh
```

Run `scripts/acceptance.sh` for control-plane acceptance and
`scripts/tool-use-acceptance.sh` for the local mock-backed Tool Use path.
Commands with `--upstream` and `provider-matrix.sh` can incur provider cost.
Every Messages request must include positive `max_tokens` no greater than
`MODELPORT_MAX_OUTPUT_TOKENS` (default 131072); invalid values are rejected
before routing.

## Important Operational Limits

- `/readyz` is authenticated diagnostics; it does not currently fail when an
  upstream is degraded.
- A stream can fail through SSE `event: error` after the initial HTTP 200.
  Live-stream completion, final usage/cost, provider health, and fallback are
  not yet fully reconciled after response headers are sent; buffered
  compatibility mode completes the upstream first but delays the first byte.
- Rate limits, concurrent-stream permits, and dashboard sessions are
  process-local. Stream permits stay held through response body completion;
  quota checks are not a transactional reservation, so concurrent requests can
  overshoot a tight cap.
- Provider URL validation blocks dangerous literal addresses, but DNS answers
  are not pinned/revalidated against private ranges.
- Auth/control persistence synchronously writes complete logical JSON documents;
  retention and throughput should stay within the intended small-team profile.
- Cost and token values are operational estimates, not provider invoices. Logs
  label Provider-returned usage as `upstream-returned` and heuristic values as
  `local-estimate`; only an actually sent upstream attempt consumes user quota
  or API-key/team spend.

See [Architecture](docs/ARCHITECTURE.md) and
[Operations](docs/OPERATIONS.md) before a shared deployment.

## Security

Keep the default loopback publishing unless a trusted LAN or same-origin HTTPS
reverse proxy needs access. Do not expose the backend directly to the public
internet. Do not commit `.env`, provider keys, complete backups, prompts, or raw
sensitive logs.

Remote Providers must use HTTPS by default. Plain HTTP exposes Provider API
keys and prompt/response content; the insecure override is only for an
explicitly trusted internal upstream. Local/custom runtimes may continue to use
HTTP on loopback or a controlled local network.

For shared use:

1. Create dashboard API keys for real active users and set
   `MODELPORT_REQUIRE_CONTROL_API_KEYS=1`.
2. Configure exact trusted proxy CIDRs and allowed browser origins.
3. Set `MODELPORT_ADMIN_COOKIE_SECURE=1` behind HTTPS.
4. Protect PostgreSQL/JSON state and CLI backup files as credential material.

Read [SECURITY.md](SECURITY.md) for the threat model and reporting process.

## Local Development

```bash
cp .env.example .env
# replace required placeholders
scripts/config-validate.sh
scripts/start.sh

cd dashboard
npm ci
npm run dev
```

Before submitting changes:

```bash
scripts/check-all.sh
```

The complete toolchain and test matrix are in
[Development](docs/DEVELOPMENT.md).

## Documentation

- [Documentation index](docs/README.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Configuration reference](docs/CONFIGURATION.md)
- [API reference](docs/API.md)
- [Operations](docs/OPERATIONS.md)
- [Docker Compose](docs/DOCKER.md)
- [systemd](docs/SYSTEMD.md)
- [Provider compatibility](docs/PROVIDER_MATRIX.md)
- [Tool Use compatibility](docs/TOOL_USE_COMPATIBILITY.md)
- [Production acceptance](docs/ACCEPTANCE.md)

## License

[MIT](LICENSE)
