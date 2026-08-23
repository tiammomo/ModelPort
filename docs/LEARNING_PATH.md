# ModelPort Learning Path

This path gives a first-time operator or contributor a sequence of small,
verifiable outcomes. Use Linux or WSL2 for every repository command.

## Mental Model

```text
Claude Code / SDK
        |
        | ModelPort API key
        v
    ModelPort -------------> PostgreSQL
        |
        | Provider key (server-side only)
        v
 Hosted provider or local model runtime
```

ModelPort is an authentication, routing, protocol, policy, and evidence
gateway. The dashboard is an operations console, not a chat application.

For the optional local Qwen reference path, use the
[local Qwen reference adapter guide](LOCAL_INFERENCE_STACK.md). It keeps static
contract checks, external GPU runtime activation, and gateway verification
separate; ModelPort does not require that integration or its repository.

## Track A: Run It In 30 Minutes

This track requires Git, Docker, and Docker Compose v2. Rust is not required.

```bash
git clone https://github.com/tiammomo/ModelPort.git
cd ModelPort
cp deploy/docker/modelport.env.example .env
cp config.example.toml config.toml
# Replace every required replace-with-... value.
scripts/doctor.sh --setup
scripts/build-container.sh
scripts/compose-up.sh
scripts/smoke-test.sh
```

Success means the PostgreSQL and backend containers are healthy, the local
smoke test passes, and `http://127.0.0.1:33002` accepts the configured
administrator login. No Provider call is made by this smoke test.

## Track B: Connect A Client In 30 Minutes

A Provider key belongs only in ModelPort. A ModelPort API key is what Claude
Code, an SDK, or another client receives.

```bash
source .env
curl -fsS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:38082/v1/models
```

Only after accepting possible Provider cost, run:

```bash
scripts/smoke-test.sh --upstream
```

Find that request in Dashboard Request Logs and identify the selected Provider,
model, status, latency, usage provenance, and any fallback attempts. Continue
with [API](API.md) and [Providers](PROVIDERS.md).

## Track C: Operate It In 45 Minutes

Use this fixed diagnostic order:

```bash
docker compose ps
scripts/smoke-test.sh
scripts/doctor.sh
docker compose logs --tail=100 modelport
```

- `/livez` means the process is alive.
- `/readyz` means required persistence is ready for traffic.
- `/metrics` exposes Prometheus text.
- `state_conflict` means a stale auth/control write was rejected instead of
  overwriting a newer revision.

Before an upgrade, rehearse backup verification and isolated restore:

```bash
archive="$(scripts/backup-compose.sh create)"
scripts/backup-compose.sh verify "$archive"
scripts/backup-compose.sh drill "$archive"
scripts/database-preflight.sh
```

Continue with [Operations](OPERATIONS.md) and [Production](PRODUCTION.md).

## Track D: Contribute In 60 Minutes

Do not mix Windows executables from `/mnt/c` into the Linux development
toolchain.

```bash
scripts/doctor.sh --development
npm --prefix dashboard ci
scripts/check.sh
npm --prefix dashboard run check
scripts/check-all.sh
```

Read the backend in request-flow order:

1. `src/routes/client_api.rs` for the HTTP boundary;
2. `src/exchange.rs` and `src/types.rs` for protocol conversion;
3. `src/providers/` for Provider adapters;
4. `src/enterprise_ledger.rs` for requests, attempts, budgets, and audit;
5. `src/auth.rs`, `src/control.rs`, and `src/storage.rs` for control state.

Choose the smallest relevant test first, then run the complete repository
check. Never put real keys, prompts, responses, or backups in logs, fixtures,
or commits. Continue with [Development](DEVELOPMENT.md) and
[Architecture](ARCHITECTURE.md).

## Short Troubleshooting Decision

1. `doctor --setup` fails: fix the first Linux, Docker, file, or placeholder
   failure.
2. A container is unhealthy: inspect the last 100 lines for that service.
3. Liveness passes but readiness fails: inspect PostgreSQL, migrations, and
   state revisions.
4. HTTP 401/403: verify the ModelPort key, account state, and policy.
5. HTTP 429: inspect local rate, concurrency, quota, and budget controls.
6. Upstream failure: inspect Provider/credential health before making a paid
   diagnostic request.

The complete operational contract remains in [Operations](OPERATIONS.md).
