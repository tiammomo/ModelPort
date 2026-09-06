# Getting Started

This is the shortest Tier 1 path to a governed request. It builds the backend
and Dashboard images locally inside Docker (no host Rust or Node toolchain is
needed), then runs them with Docker Compose, PostgreSQL, and the DeepSeek
example. Published release images are an optional path below. Use
[Providers](PROVIDERS.md) for a local runtime or another hosted Provider.

## 1. Prerequisites

- Linux x86_64 and Git.
- Docker Engine or Docker Desktop with Compose v2.
- A Provider account and API key.
- Free local ports `33002` and `38082`.

ModelPort stores all runtime state in PostgreSQL. The Compose stack supplies it;
you do not need to install PostgreSQL on the host.

## 2. Create Local Configuration

```bash
git clone --depth 1 https://github.com/tiammomo/ModelPort.git
cd ModelPort
scripts/setup.sh
```

The initializer creates `.env` with unique router, administrator and database
credentials (mode `0600`) and copies `config.toml`. Existing files are preserved.
Set `DEEPSEEK_ANTHROPIC_AUTH_TOKEN` in `.env` to your Provider key. For another
Provider, follow [Providers](PROVIDERS.md). Advanced environment options remain
in the [Docker reference](../deploy/docker/modelport.env.example).

Do not commit `.env` or `config.toml`. Provider credentials stay on the server;
client applications use a scoped ModelPort API key.

The sample model is `deepseek-v4-flash`. If the Provider account exposes a
different ID, update `DEEPSEEK_MODEL`, the `config.toml` model list/default, and
the request examples together.

## 3. Check The Setup

Run the read-only Linux and Compose preflight before building images. The
scripts default to the root source-build manifest (`docker-compose.yml`), so no
`MODELPORT_COMPOSE_FILE` export is needed:

```bash
scripts/doctor.sh --setup
```

Fix the first `[fail]` and rerun it. This mode does not start services or make a
Provider request.

## 4. Build And Start

```bash
scripts/build-container.sh
MODELPORT_LOCAL_BUILD=1 scripts/compose-up.sh
docker compose ps
```

`scripts/build-container.sh` builds `modelport:local`,
`modelport-dashboard:local` with Docker. Add `--with-ops-agent` only when the
optional Agent is needed.
`MODELPORT_LOCAL_BUILD=1` verifies the selected local images exist and disables image
pulls; with the manifest defaulting to `docker-compose.yml`, `docker compose
ps` shows the running project.

### Optional: Pull Published Release Images

```bash
export MODELPORT_COMPOSE_FILE="$PWD/deploy/release/compose.yml"
docker compose -f "$MODELPORT_COMPOSE_FILE" config --quiet
docker compose -f "$MODELPORT_COMPOSE_FILE" pull
MODELPORT_LOCAL_BUILD=0 scripts/compose-up.sh
docker compose -f "$MODELPORT_COMPOSE_FILE" ps
```

The images are published by the tagged release. Initial evaluation may use the
version tag in Compose; shared/production deployments must use the two main
image digests (plus the optional Agent digest when enabled) listed in the
GitHub Release. Verify checksums, signatures,
attestations, and SBOMs through
[Upgrading and Rollback](UPGRADING.md#release-inputs).

Use this path only after the matching tag and all required images appear in
[GitHub Releases](https://github.com/tiammomo/ModelPort/releases). Source builds
remain available for unreleased changes.

Expected services:

| Service | Expected state |
| --- | --- |
| `postgres` | healthy |
| `modelport` | healthy |
| `dashboard` | running |

The table shows the default internal-database stack. When `.env` sets
`MODELPORT_DATABASE_URL` to an external PostgreSQL instance, `postgres` is not
started (Compose profile `internal-db`); the remaining services connect to that
external instance, so expect only `modelport` and `dashboard` in `docker
compose ps`.

The optional operations Agent is not started by the default profile. After the
gateway is configured, follow [Operations Agent](OPS_AGENT.md) to create its
dedicated, non-inference service account. Starting the optional container and
enabling it in **运维事件** are separate confirmations; both default off. Local
models are recommended first for optional diagnosis. Roll out `shadow` before
`read_only` mode.

The backend's Compose healthcheck uses authenticated `/readyz`, so `healthy`
means required persistence is usable. The image-level `/livez` check proves only
that the process answers. Dashboard has no backend startup dependency: its
static page remains available and API routes return 502 when the backend is
unavailable.

If a service does not start:

```bash
docker compose -f "${MODELPORT_COMPOSE_FILE:-docker-compose.yml}" logs --tail=100 postgres
docker compose -f "${MODELPORT_COMPOSE_FILE:-docker-compose.yml}" logs --tail=100 modelport
docker compose -f "${MODELPORT_COMPOSE_FILE:-docker-compose.yml}" logs --tail=100 dashboard
```

## 5. Verify The Gateway

```bash
scripts/smoke-test.sh
```

This checks process liveness, authenticated storage readiness, and the model
catalog without generating model output.

Open `http://127.0.0.1:33002` and sign in with the generated administrator
username and password from `.env`. Choose **继续接入** on the Dashboard to follow
the four-step flow; saved configuration restores progress after a reload. The backend API remains at
`http://127.0.0.1:38082`.

## 6. Authorize The First Governed Request

ModelPort fails closed for cloud egress when a project has no policy. Before
calling the DeepSeek example, open **Governance (治理与变更审批)** in the
Dashboard under **团队与策略 → 治理与审批**. Choose the project routing policy,
select DeepSeek and its exact model, explicitly allow approved cloud execution,
and keep the default classification `unknown`. The advanced fields should use:

- Target: `org_local/prj_default/env_default`
- Reason: a concrete explanation such as `Allow the documented public synthetic DeepSeek test`
- Region: `global`; API version: `anthropic-v1`

Submit the recorded change. In default Small-Team mode, choose
**Direct apply (直接应用)**; the write still requires CSRF protection and is
audited. Enterprise mode or `MODELPORT_REQUIRE_DUAL_APPROVAL=1` requires a
different administrator to approve the change before **Apply change (应用变更)**
becomes available.

This example authorizes only the exact DeepSeek Provider, model, global region,
and Anthropic v1 path for `org_local/prj_default/env_default`. Its default
classification stays `unknown`, so unclassified or sensitive input remains
`local_strict`. Do not paste private source code or other sensitive data into
the public synthetic test.

## 7. Send The First Request

This request can consume Provider quota:

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

Or run the synthetic upstream smoke path:

```bash
scripts/smoke-test.sh --upstream
```

Open **Request Logs** in the dashboard and confirm the selected Provider/model,
status, latency, token provenance, and estimated cost. A Provider-returned usage
value is still not an authoritative invoice.

## 8. Connect A Client

Create a user and scoped client API key under **团队与策略**, then open
**模型接入 → 用户使用说明**. Select that key and a model. Both administrators
and developers see the selected key's effective catalog. Configuration copying
requires a current setup check for that key/model, including its default data
classification and project egress policy. The key reveal dialog uses the same
check without retaining its one-time secret.

The public synthetic curl above supplies its own classification header. It does
not make the project's `unknown` default suitable for ordinary cloud calls.
Use a local route, or explicitly classify a dedicated approved project's data
through the policy form. Unknown and sensitive data remain local-only. Never
classify arbitrary source code as public to bypass this check.

Setup checks do not send a Provider request or reserve budget. Actual client IP,
quota, upstream health and payload-specific Tool Use/fidelity checks still apply
when a request runs; inspect its request log. Production hardening can require
scoped keys with `MODELPORT_REQUIRE_CONTROL_API_KEYS=1`. Never give a client the
upstream Provider key.

## 9. Stop, Restart, Or Upgrade

```bash
docker compose -f "$MODELPORT_COMPOSE_FILE" stop
docker compose -f "$MODELPORT_COMPOSE_FILE" start
docker compose -f "$MODELPORT_COMPOSE_FILE" logs -f modelport
docker compose -f "$MODELPORT_COMPOSE_FILE" down
```

`docker compose -f "$MODELPORT_COMPOSE_FILE" down` preserves named volumes. Do
not add `-v` unless permanent database deletion is intentional and a verified
backup exists.

Before an upgrade, follow [Upgrading and Rollback](UPGRADING.md),
[Operations: Backup And Restore](OPERATIONS.md#backup-and-restore), and
[Production](PRODUCTION.md).

## Common First-Run Problems

| Symptom | Resolution |
| --- | --- |
| Compose reports missing `config.toml` | Copy `config.example.toml` to `config.toml`. |
| Startup rejects a placeholder | Replace every required `replace-with-...` value in `.env`. |
| Dashboard opens but login fails | Use the admin username/password, not the router token. |
| Dashboard opens but API calls return 502 | Static Nginx is healthy but the backend is absent/unreachable; check `/livez`, `/readyz`, and backend logs. |
| `/v1/*` returns 401 | Send `x-api-key: <MODELPORT_AUTH_TOKEN>` or `Authorization: Bearer <key>`. |
| Model is not listed | Align the Provider's configured model ID with the account/runtime catalog. |
| Local runtime is unreachable from Docker | Use `host.docker.internal`, not container loopback. |
| Stream starts with HTTP 200 then fails | Inspect the SSE `event: error` and matching request log. |
| Port is already allocated | Change `MODELPORT_API_PUBLISH` or `MODELPORT_DASHBOARD_PUBLISH` in `.env`. |

Continue with [Configuration](CONFIGURATION.md), [Providers](PROVIDERS.md),
[Deployment](DEPLOYMENT.md), or [Operations](OPERATIONS.md) only when your task
requires the additional detail.
