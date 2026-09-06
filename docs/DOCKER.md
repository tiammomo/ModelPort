# Docker Compose Deployment

The versioned `deploy/release/compose.yml` is the normal Small-Team Beta
installation after a GitHub Release exists; it pulls prebuilt Linux x86_64
images. The root Compose file remains the current-main contributor/source-build
path. Both start the backend, a same-origin Dashboard proxy, and PostgreSQL. The
phase-one production profile uses one ModelPort
instance but an external managed PostgreSQL database and secret-manager-rendered
runtime environment; see [Single-instance production](#single-instance-production).

| Service/volume | Purpose |
| --- | --- |
| `postgres` | PostgreSQL 18.4 for all durable runtime state; no host port by default. |
| `modelport` | Rust data plane, control API, routing, metrics, and CLI. |
| `dashboard` | Static React UI plus same-origin proxy to backend routes. |
| `modelport-postgres-18` | Persistent PostgreSQL 18 data. |
| `modelport-data` | Backend working directory and explicit backup files. |

Redis, queues, Prometheus, Caddy, and an inference runtime are not part of the
default stack.

## Release Images And Runtime Hardening

The release workflow builds the backend with Rust 1.96.0 and
`cargo build --release --locked`, so `Cargo.lock` is authoritative. The
dashboard builder uses Node.js 24 and `npm ci --no-audit --no-fund`; disabling
the install-time audit and funding messages does not replace dependency review
or vulnerability scanning.

The Compose backend runs as the image's unprivileged `modelport` user with an
init process, a read-only root filesystem, all Linux capabilities dropped, and
`no-new-privileges`. Only these paths are writable at runtime:

- `/data`, backed by the `modelport-data` named volume, for explicit CLI
  backup files and bounded runtime working data;
- `/tmp`, backed by a `noexec,nosuid` 64 MiB tmpfs for temporary runtime files.

`/config/.env` is a read-only bind mount. The read-only root filesystem does
not make the `/data` named volume read-only; persistence and backup commands
depend on that volume remaining writable. PostgreSQL data is independently
stored in `modelport-postgres-18`.

The dashboard Nginx process runs as its unprivileged `nginx` user on internal
port 8080. Compose also gives it an init process, a read-only root filesystem,
no Linux capabilities, and `no-new-privileges`; its Nginx PID and temporary
files live on a `noexec,nosuid` 32 MiB `/tmp` tmpfs. These controls are defense
in depth, not a substitute for loopback publishing, a firewall, HTTPS, image
updates, and secret protection.

## Start

After `v0.1.0` and both GHCR images actually exist, use a tagged checkout and
the release profile:

```bash
scripts/setup.sh
# replace every required placeholder
export MODELPORT_COMPOSE_FILE="$PWD/deploy/release/compose.yml"
docker compose -f "$MODELPORT_COMPOSE_FILE" pull
scripts/compose-up.sh
docker compose -f "$MODELPORT_COMPOSE_FILE" ps
```

Before the first external Release, or when contributing changes on `main`, build
locally instead:

```bash
export MODELPORT_COMPOSE_FILE="$PWD/docker-compose.yml"
scripts/build-container.sh
MODELPORT_LOCAL_BUILD=1 scripts/compose-up.sh
```

`compose-up.sh` runs a read-only database alignment check before updating an
existing deployment. A major-version or data-volume mismatch is a hard stop;
follow [the PostgreSQL migration runbook](POSTGRESQL_MIGRATION.md) instead of
using raw `docker compose up` to bypass it.

The contributor build helper refuses a dirty worktree and embeds the Git revision plus
`io.modelport.source-state=clean` in the backend image. For local-only testing
of uncommitted changes, `scripts/build-container.sh --allow-dirty` produces an
explicitly dirty-labeled image that must not be promoted as a release.

Compose normally injects and mounts the root `.env`. For manifest validation or
an intentionally different deployment file, point both uses at the same path:

```bash
MODELPORT_COMPOSE_ENV_FILE=deploy/docker/modelport.env.example \
  docker compose --env-file deploy/docker/modelport.env.example \
    -f "$MODELPORT_COMPOSE_FILE" config --quiet
```

The example contains placeholders and is for validation only; do not start a
deployment with its sample credentials.

Open:

- Dashboard: `http://127.0.0.1:33002`
- Messages API: `http://127.0.0.1:38082/v1/messages`
- Chat Completions API: `http://127.0.0.1:38082/v1/chat/completions`
- Liveness: `http://127.0.0.1:38082/livez`

Claude Code uses the host-published backend:

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:38082
ANTHROPIC_AUTH_TOKEN=<same-as-MODELPORT_AUTH_TOKEN>
ANTHROPIC_MODEL=<configured-model-id>
```

Run [Production](PRODUCTION.md) checks after startup.

## Daily Commands

```bash
docker compose -f "$MODELPORT_COMPOSE_FILE" ps
docker compose -f "$MODELPORT_COMPOSE_FILE" logs -f modelport
docker compose -f "$MODELPORT_COMPOSE_FILE" logs -f dashboard
docker compose -f "$MODELPORT_COMPOSE_FILE" restart modelport
scripts/build-container.sh
MODELPORT_LOCAL_BUILD=1 scripts/compose-up.sh
docker compose -f "$MODELPORT_COMPOSE_FILE" down
```

`docker compose -f "$MODELPORT_COMPOSE_FILE" down` preserves named volumes;
adding `-v` deletes PostgreSQL and backend data and is irreversible without a
backup.

## Storage

Unless `.env` explicitly sets `MODELPORT_DATABASE_URL`, Compose constructs it
for the internal PostgreSQL service. An explicit complete URL overrides that
default. The application stores low-frequency auth/control definitions as two
`jsonb` documents in `modelport_state`:

| Namespace | Contents |
| --- | --- |
| `auth` | Users, roles, status, and password hashes. |
| `control` | Teams, API-key hashes, policy and quota definitions, route/provider overrides, credential metadata, health, and tests. |

The database is not exposed on host port 5432. If host access is required for
debugging, add an explicit non-conflicting loopback mapping such as
`127.0.0.1:15432:5432`.

The application uses SQLx with rustls. Development mode defaults to TLS
`prefer`, which allows the internal Compose database without provisioning a
certificate. A remote production database must use `verify-full` plus a trusted
root; enabling `MODELPORT_ENTERPRISE_MODE=1` enforces that boundary.

Compose interpolation does not percent-encode
`MODELPORT_POSTGRES_PASSWORD`. Use a long URL-safe value made from letters,
digits, `_`, and `-`, or explicitly set a complete `MODELPORT_DATABASE_URL`
whose password component is percent-encoded. Keep
`MODELPORT_POSTGRES_PASSWORD` itself as PostgreSQL's raw password. Characters
such as `@`, `:`, `/`, `%`, and `#` are unsafe in the constructed URL when left
unencoded.

The Compose service always supplies either the explicit or constructed database
URL. Auth, control, operational request, budget, usage, and audit state have no
runtime file or memory mode. Start or update this local deployment with
`scripts/compose-up.sh`. Old JSON files are not imported.

Persistence replaces logical documents only for low-frequency auth/control
changes. Document rows carry monotonic revisions, stale writers are rejected
instead of silently overwriting newer state, and logical backup restore updates
both rows atomically. Embedded migrations create normalized tenant, gateway-request,
Provider-attempt and audit rows, then add hashed idempotency claims, renewable
instance leases, transactional budget accounts/reservations, and append-only
evidence events.
Every paid upstream attempt is inserted before egress and finalized at the
response, stream, or expired-lease terminal state. Terminal request rows are
also the source for logs, Dashboard ranges, quota/spend usage, and management
statistics. Response replay remains open.

## Backup

The dashboard's CSRF-protected `POST /admin/backup` download is a redacted
diagnostic snapshot, creates an audit event, and is not a restore artifact.

For the local PostgreSQL Compose deployment, use the host-side backup helper.
New schema-v2 archives are atomic `0600` files containing a portable
custom-format `pg_dump`, checksums, and secret-free source/database provenance.
They deliberately exclude `.env` and `config.toml`; recover configuration from
Git and credentials from the secret manager.

```bash
./scripts/backup-compose.sh create
./scripts/backup-compose.sh verify backups/modelport-<UTC>.tar.gz
./scripts/backup-compose.sh drill backups/modelport-<UTC>.tar.gz
```

`drill` restores the dump into a new, unpublished, temporary PostgreSQL
container, checks the required `auth` and `control` namespaces, and removes the
container. It never connects `modelport` to the temporary database and never
writes to the production database. Completed archives older than
`MODELPORT_BACKUP_RETENTION_DAYS` (14 by default) are pruned only from the
configured backup directory.

Legacy schema-v1 archives remain readable but contain plaintext runtime
configuration and must be treated as credential material. Verification prints
an explicit warning. See [PostgreSQL Migration](POSTGRESQL_MIGRATION.md) before
changing a PostgreSQL major version or database endpoint.

The CLI can export a logical auth/control backup from PostgreSQL:

```bash
docker compose -f "$MODELPORT_COMPOSE_FILE" exec modelport \
  model-port backup export /data/modelport-backup.json
docker compose -f "$MODELPORT_COMPOSE_FILE" exec modelport \
  model-port backup validate /data/modelport-backup.json
```

Validation and restore both deeply deserialize the auth/control payloads before
writing. Auth checks include unique non-empty IDs/usernames, valid identity and
password-hash fields, and at least one active admin for a non-empty user set;
control records must match the current schema.

The file contains password and API-key hashes plus personal/usage metadata.
Copy it to encrypted storage and restrict access.

Restore with writers stopped:

```bash
docker compose -f "$MODELPORT_COMPOSE_FILE" stop modelport dashboard
docker compose -f "$MODELPORT_COMPOSE_FILE" run --rm modelport \
  model-port backup restore /data/modelport-backup.json --yes
scripts/compose-up.sh
```

Restore saves both previous logical values, then verifies their observed
revisions and replaces auth and control in one PostgreSQL transaction. Retain
the saved application values and a database-native backup until the restored
service has passed smoke and login checks.

Keep a database-native backup too:

```bash
docker compose -f "$MODELPORT_COMPOSE_FILE" exec postgres \
  pg_dump -U modelport modelport > modelport.sql
docker compose -f "$MODELPORT_COMPOSE_FILE" exec -T postgres \
  psql -U modelport modelport < modelport.sql
```

The Compose project has `name: modelport`; a physical volume backup therefore
uses volume `modelport_modelport-postgres-18`. PostgreSQL 18 stores data below
the versioned `PGDATA=/var/lib/postgresql/18/docker`, while Compose mounts the
parent `/var/lib/postgresql` as required by the official image. Prefer
`pg_dump` for portable restore instead of copying a live database directory.

## Single-instance Production

[`deploy/production/compose.single.yml`](../deploy/production/compose.single.yml)
is the accepted phase-one topology. It intentionally contains one ModelPort
instance and no PostgreSQL service. It requires:

- digest-pinned backend and dashboard images;
- an external `modelport_default` network, shared with approved local inference
  Providers when present;
- a reviewed non-secret `config.toml` whose Provider credentials use
  `token_env`/`api_key_env` references;
- a CA file for managed PostgreSQL;
- a reviewed operations-ownership TOML with different named Owner and Backup
  contacts plus escalation channels;
- a permission-`0600`, short-lived runtime env file rendered outside the
  repository by the production secret manager, including a dedicated scoped
  `MODELPORT_HEALTHCHECK_API_KEY` for authenticated readiness.

Do not run `docker compose config` with the real runtime env file in a log
collection or CI job: Compose renders environment values. Validate with a
synthetic file containing placeholders. The runtime database URL must use TLS
`verify-full` and reference `/run/modelport/database-ca.pem` when an explicit
root certificate is needed.

Before rendering or starting the production profile, run the secret-safe,
read-only preflight:

```bash
export MODELPORT_COMPOSE_FILE="$PWD/deploy/production/compose.single.yml"
export MODELPORT_IMAGE='registry/modelport@sha256:<digest>'
export MODELPORT_DASHBOARD_IMAGE='registry/modelport-dashboard@sha256:<digest>'
export MODELPORT_RUNTIME_ENV_FILE=/run/modelport/runtime.env
export MODELPORT_CONFIG_FILE=/etc/modelport/config.toml
export MODELPORT_DATABASE_CA_FILE=/etc/modelport/database-ca.pem
export MODELPORT_OWNERSHIP_FILE=/etc/modelport/ownership.toml

./scripts/production-preflight.sh
./scripts/compose-up.sh
docker compose -f "$MODELPORT_COMPOSE_FILE" ps
```

It verifies digest-pinned images, file ownership/permissions, repository-external
secret placement, strict database TLS, and every Provider credential reference
without printing values.

This profile is not active-active. A second ModelPort instance remains a later
milestone governed by
[ADR-0005](adr/0005-forty-user-hybrid-routing-baseline.md).

The Compose services use bounded `json-file` logging: 10 MiB per file and five
files by default. Override `MODELPORT_LOG_MAX_SIZE` or
`MODELPORT_LOG_MAX_FILES` only after checking host disk capacity.

## Reload And Restart

Compose mounts `.env` read-only at `/config/.env` and sets
`MODELPORT_ENV_FILE=/config/.env`. The dashboard can reload mounted TOML and
env-file-only values for new requests, but process environment values take
precedence over the mounted file.

Restart or recreate `modelport` for:

- bind/body/request-concurrency/stream-concurrency layers;
- HTTP client timeout, redirect, response/SSE settings;
- rate-limit values/window;
- trusted proxies, CSRF/origin, detailed-health, and private/insecure-URL policy;
- admin bootstrap/session/cookie settings;
- storage URL/paths;
- a newly added credential-profile environment variable.

Docker's `env_file` populates process variables when the container is created.
Editing an existing `.env` key and pressing reload therefore does not override
the old process value. Dashboard credential profiles also read process variables
directly. Recreate after `.env` changes through the checked helper:

```bash
scripts/compose-up.sh modelport
```

See the exact [reload matrix](CONFIGURATION.md#reload-versus-restart).

## Access Scope And Reverse Proxy

Default publishing is loopback-only:

```env
MODELPORT_API_PUBLISH=127.0.0.1:38082
MODELPORT_DASHBOARD_PUBLISH=127.0.0.1:33002
```

For a trusted LAN, bind deliberately and enforce a host firewall. For remote or
shared use, expose one HTTPS origin through a reverse proxy. The dashboard Nginx
image already proxies `/admin`, `/v1`, `/livez`, `/readyz`, `/health`, and
`/metrics` on one origin.

`deploy/docker/Caddyfile.example` addresses `dashboard:8080`; that name resolves
only when Caddy joins the same Compose network. An external host Caddy instance
must target the dashboard's published host port instead.

Behind HTTPS set:

```env
MODELPORT_ADMIN_COOKIE_SECURE=1
MODELPORT_ALLOWED_ORIGINS=https://modelport.example.com
MODELPORT_TRUSTED_PROXIES=<exact-proxy-ip-or-cidr>
```

`MODELPORT_ALLOWED_ORIGINS` is an admin-write check, not browser CORS. Keep the
dashboard and backend routes same-origin.

## Trusted Client IP

The Compose template includes the Docker bridge range so Nginx can forward the
real client IP:

```env
MODELPORT_TRUSTED_PROXIES=127.0.0.1,::1,172.16.0.0/12
```

This is broad. In a controlled network, replace it with the actual proxy subnet
or address. A wrong trust rule can let a client forge IP allowlist/rate-limit
inputs.

The bundled Nginx proxy deliberately sets `X-Forwarded-For` to its observed
`$remote_addr` instead of appending an incoming client-controlled chain.
ModelPort then walks forwarded hops from right to left and removes only peers
covered by `MODELPORT_TRUSTED_PROXIES`. If another reverse proxy is added in
front, list only its exact addresses/subnets and verify the complete hop chain.

Nginx also forwards `Host $http_host`, not `$host`. `$http_host` preserves the
published port (for example `127.0.0.1:33002`), which keeps the browser Origin
and backend Host authorities aligned for CSRF write checks. A custom proxy on a
non-default port must likewise preserve the original Host including its port;
otherwise same-origin dashboard writes can be rejected.

## Host Model Runtimes

Inside a container, `127.0.0.1` is the container itself. Use the configured
host gateway for a runtime on the Docker host:

```env
MODELPORT_ENABLE_OLLAMA=1
OLLAMA_BASE_URL=http://host.docker.internal:11434/v1
OLLAMA_MODEL=llama3.1

MODELPORT_ENABLE_CUSTOM=1
CUSTOM_OPENAI_BASE_URL=http://host.docker.internal:8000/v1
CUSTOM_OPENAI_MODEL=default
```

`host.docker.internal` is a hostname and current URL validation does not inspect
its resolved IP. Only use it for a runtime you trust. See
[Providers](PROVIDERS.md#local-runtime-contract).

Local/custom Provider classes may use HTTP for these controlled runtime paths.
Other Providers require HTTPS unless
`MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` is set. That override sends Provider
API keys and prompt/response content in plaintext across the Docker/network
path, so do not use it for an Internet endpoint or an untrusted LAN.

Provider base URLs may not contain userinfo, query parameters, or fragments.
Set runtime/provider keys through the corresponding environment variable; do
not embed a credential in the URL.

## Current Limits

- Compose is a single backend instance; rate limits and sessions are not shared.
- Concurrent-stream permits are also process-local and stay held until each
  downstream response body completes or is dropped.
- `/readyz` checks auth/control storage and the normalized ledger but is not an
  all-Provider gate.
- Fallback cannot occur after live-stream response headers have been sent.
- Provider hostname answers are resolved, policy-checked, and connection-pinned
  for each outbound request. Explicit private-Provider approval and the host's
  outbound firewall remain operator-owned controls.
- Low-frequency auth/control definitions still use two complete PostgreSQL
  documents; operational usage, audit, request, and Provider-attempt data is
  normalized.

These limits are detailed in [Operations](OPERATIONS.md#current-operational-limits).
