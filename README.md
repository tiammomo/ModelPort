# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)
[![CodeQL](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/tiammomo/ModelPort/badge)](https://scorecard.dev/viewer/?uri=github.com/tiammomo/ModelPort)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**English** | [简体中文](README.zh-CN.md)

ModelPort v0.1.x is a free, MIT-licensed, self-hosted LLM gateway for 20–50
person internal development teams that use local models and approved cloud
Providers. It gives Claude Code, SDKs, and internal applications one governed
endpoint for authentication, logical-model routing, quotas, usage, Provider
health, and request evidence. The Small-Team Beta experience is Chinese-first;
the API and maintained operator documentation remain available in English.

The approved product direction is an independent hybrid model and GPU control
plane. That direction keeps hosted API Providers first-class, treats local
Qwen as one replaceable Runtime Adapter example, and does not claim that the
target Compute or Deployment APIs ship in v0.1.x. See
[Architecture](docs/ARCHITECTURE.md) and
[ADR-0007](docs/adr/0007-independent-model-and-gpu-control-plane.md).

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

## What You Get

- `POST /v1/messages`, `POST /v1/chat/completions`, `GET /v1/models`, and
  opt-in exact token counting.
- Anthropic and OpenAI-compatible Provider adapters with bounded streaming and
  Tool Use conversion.
- Optional CPA Codex and Claude account channels that remain internal Providers
  behind ModelPort's policy, routing, and evidence boundary.
- Deterministic routes plus opt-in explainable smart routing with shadow mode,
  stable canaries, and durable decision evidence.
- Scoped client API keys, users, teams, quotas, spend controls, Provider
  credential pools, cooldown, and bounded fallback.
- A React operations dashboard and a PostgreSQL request, usage, budget, and
  audit ledger.
- An off-by-default deterministic, read-only operations Agent with a durable
  incident center, bounded offline spool, recovery evidence, and optional
  local-first, operator-selected model diagnosis.
- Docker Compose and systemd deployment paths, backup/restore tooling,
  Prometheus metrics, and acceptance scripts.

ModelPort currently supports one Linux x86_64 instance on a trusted host or
small trusted network. It is not enterprise/HA software, a public multi-tenant
service, model runtime, chat UI, payment processor, or Provider invoice. See
[Compatibility](docs/COMPATIBILITY.md), [Production](docs/PRODUCTION.md), and
[Roadmap](docs/ROADMAP.md) before making broader availability claims.

## Quick Start

Requirements: Linux x86_64, Git, Docker, Docker Compose v2, and credentials for
at least one Provider. Once `v0.1.0` appears on the GitHub Releases page, the
supported user path below pulls its prebuilt images and does not compile Rust
or the Dashboard. The maintained example uses DeepSeek's Anthropic-compatible
endpoint.

```bash
git clone --branch v0.1.0 --depth 1 https://github.com/tiammomo/ModelPort.git
cd ModelPort
cp deploy/docker/modelport.env.example .env
cp config.example.toml config.toml
```

Edit `.env` and replace every required `replace-with-...` value. At minimum set
unique router, administrator, PostgreSQL, and Provider credentials. Keep
`MODELPORT_AUTH_TOKEN` and the client-side `ANTHROPIC_AUTH_TOKEN` equal for the
first local test.

```bash
export MODELPORT_COMPOSE_FILE="$PWD/deploy/release/compose.yml"
scripts/doctor.sh --setup
docker compose -f "$MODELPORT_COMPOSE_FILE" pull
scripts/compose-up.sh
docker compose -f "$MODELPORT_COMPOSE_FILE" ps
scripts/smoke-test.sh
```

The release command is intentionally invalid before the `v0.1.0` tag and GHCR
images exist; this repository edit cannot publish external artifacts. To test
current `main` or contribute before that release, use the source-build path:

```bash
git clone https://github.com/tiammomo/ModelPort.git
cd ModelPort
cp deploy/docker/modelport.env.example .env
cp config.example.toml config.toml
# replace required placeholders
export MODELPORT_COMPOSE_FILE="$PWD/docker-compose.yml"
scripts/build-container.sh
MODELPORT_LOCAL_BUILD=1 scripts/compose-up.sh
```

Open `http://127.0.0.1:33002` and sign in with
`MODELPORT_ADMIN_USERNAME`/`MODELPORT_ADMIN_PASSWORD`.

For local Qwen, another Provider, production hardening, digest pinning, or
troubleshooting, follow the tested [Getting Started guide](docs/GETTING_STARTED.md).
The optional Agent has its own [safe rollout guide](docs/OPS_AGENT.md); it is
free and open source with the rest of ModelPort and starts in shadow mode.
After the first Release exists, building images from source is a contributor
workflow documented in [Development](docs/DEVELOPMENT.md), not a normal user
installation step.

## Send Your First Request

Cloud egress is fail-closed until the request's project has an explicit policy.
In the Dashboard, open **Governance (治理与变更审批)**, choose
`project_policy.upsert`, set the target to
`org_local/prj_default/env_default`, and record this narrow example policy:

```json
{
  "organizationId": "org_local",
  "projectId": "prj_default",
  "environmentId": "env_default",
  "maximumMode": "cloud_first",
  "defaultClassification": "unknown",
  "allowedProviders": ["deepseek"],
  "allowedModels": ["deepseek-v4-flash"],
  "allowedRegions": ["global"],
  "allowedApiVersions": ["anthropic-v1"],
  "cloudEnabled": true
}
```

Give the change a concrete reason, submit it, then apply it. The default
Small-Team mode lets the same administrator apply this recorded change with
CSRF and audit protection. Enterprise mode or
`MODELPORT_REQUIRE_DUAL_APPROVAL=1` requires a different administrator to
approve it before apply. This boundary permits only the documented DeepSeek
model/API path; requests without an explicit safe classification still remain
local-only.

```bash
source .env

curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  -H 'content-type: application/json' \
  -H 'x-modelport-data-classification: public' \
  -H 'x-modelport-hybrid-mode: cloud_first' \
  http://127.0.0.1:38082/v1/messages \
  -d '{
    "model":"deepseek-v4-flash",
    "max_tokens":96,
    "messages":[{"role":"user","content":"Reply exactly: OK"}]
  }'
```

This call can consume Provider quota. `scripts/smoke-test.sh` is local-only;
use `scripts/smoke-test.sh --upstream` when a paid synthetic call is intended.

Claude Code:

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:38082
ANTHROPIC_AUTH_TOKEN=<MODELPORT_AUTH_TOKEN>
ANTHROPIC_MODEL=deepseek-v4-flash
```

OpenAI-compatible SDK:

```env
OPENAI_BASE_URL=http://127.0.0.1:38082/v1
OPENAI_API_KEY=<MODELPORT_CLIENT_KEY>
OPENAI_MODEL=deepseek-v4-flash
```

Use a dashboard-issued scoped client key for shared deployments. Provider keys
stay in ModelPort and must never be copied into client applications.

## Documentation

Choose the document for your task instead of reading the whole documentation
set:

- [Getting Started](docs/GETTING_STARTED.md) — install, first login, first
  request, and common startup failures.
- [Learning Path](docs/LEARNING_PATH.md) — role-based 30–60 minute operator,
  client-integration, operations, and contributor tracks.
- [Local Qwen reference adapter](docs/LOCAL_INFERENCE_STACK.md) — an optional
  Linux/WSL2 compatibility walkthrough for the original integration; it is not
  a ModelPort architecture dependency.
- [Configuration](docs/CONFIGURATION.md) — environment and TOML reference.
- [API](docs/API.md) — client and control-plane contracts.
- [Providers](docs/PROVIDERS.md) — hosted Providers, local runtimes, and
  compatibility evidence.
- [Smart Routing](docs/SMART_ROUTING.md) — scoring, shadow, canary, and
  rollback.
- [Deployment](docs/DEPLOYMENT.md) — Docker Compose, systemd, and production
  topology.
- [Operations](docs/OPERATIONS.md) — health, logs, metrics, backup, retention,
  incidents, and upgrades.
- [Compatibility](docs/COMPATIBILITY.md) — Tier 1 platform and explicit
  experimental/unsupported boundaries.
- [Observability runbook](docs/OBSERVABILITY_RUNBOOK.md) — official alerts,
  Grafana dashboard, and incident actions.
- [Upgrading and rollback](docs/UPGRADING.md) — safe-stop, backup, migration,
  acceptance, and paired application/database rollback.
- [Production](docs/PRODUCTION.md) — go-live and release acceptance.
- [Development](docs/DEVELOPMENT.md) — contributor workflow and test matrix.
- [Documentation index](docs/README.md) — role-based navigation.

## Security And Support

Keep backend and PostgreSQL ports private. Use same-origin HTTPS, exact trusted
proxy CIDRs, secure cookies, CSRF protection, and dashboard-issued API keys for
shared use. Never commit `.env`, Provider keys, backups, prompts, responses, or
raw sensitive logs.

Read [Security](SECURITY.md), [Privacy](PRIVACY.md), [Support](SUPPORT.md), and
[Governance](GOVERNANCE.md). ModelPort is free self-hosted software. The
project provides no paid edition, hosted service, or community-support SLA.

## Development

The source-development path requires a reachable PostgreSQL instance; the
development scripts do not start one. See [Development](docs/DEVELOPMENT.md)
for a loopback-only disposable database command and the complete prerequisites.

```bash
cp .env.example .env
cp config.example.toml config.toml
# replace required placeholders
scripts/start.sh

cd dashboard
npm ci
npm run dev
```

Before submitting a change:

```bash
scripts/check-all.sh
```

## License

[MIT](LICENSE)
