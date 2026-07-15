# Docker Compose Deployment

Docker Compose is the recommended complete ModelPort deployment. It starts the
backend, a same-origin dashboard proxy, and PostgreSQL.

| Service/volume | Purpose |
| --- | --- |
| `postgres` | PostgreSQL 16 for auth and control documents; no host port by default. |
| `modelport` | Rust data plane, control API, routing, metrics, and CLI. |
| `dashboard` | Static React UI plus same-origin proxy to backend routes. |
| `modelport-postgres` | Persistent PostgreSQL data. |
| `modelport-data` | Backend working/backup files and file-backend migration source. |

Redis, queues, Prometheus, Caddy, and an inference runtime are not part of the
default stack.

## Image Build And Runtime Hardening

The backend image builds with Rust 1.96.0 and
`cargo build --release --locked`, so `Cargo.lock` is authoritative. The
dashboard builder uses Node.js 24 and `npm ci --no-audit --no-fund`; disabling
the install-time audit and funding messages does not replace dependency review
or vulnerability scanning.

The Compose backend runs as the image's unprivileged `modelport` user with an
init process, a read-only root filesystem, all Linux capabilities dropped, and
`no-new-privileges`. Only these paths are writable at runtime:

- `/data`, backed by the `modelport-data` named volume, for JSON state,
  migration input, and CLI backup files;
- `/tmp`, backed by a `noexec,nosuid` 64 MiB tmpfs for temporary runtime files.

`/config/.env` is a read-only bind mount. The read-only root filesystem does
not make the `/data` named volume read-only; persistence and backup commands
depend on that volume remaining writable. PostgreSQL data is independently
stored in `modelport-postgres`.

The dashboard Nginx process runs as its unprivileged `nginx` user on internal
port 8080. Compose also gives it an init process, a read-only root filesystem,
no Linux capabilities, and `no-new-privileges`; its Nginx PID and temporary
files live on a `noexec,nosuid` 32 MiB `/tmp` tmpfs. These controls are defense
in depth, not a substitute for loopback publishing, a firewall, HTTPS, image
updates, and secret protection.

## Start

```bash
cp deploy/docker/modelport.env.example .env
# replace every required placeholder
docker compose up -d --build
docker compose ps
```

Compose normally injects and mounts the root `.env`. For manifest validation or
an intentionally different deployment file, point both uses at the same path:

```bash
MODELPORT_COMPOSE_ENV_FILE=deploy/docker/modelport.env.example \
  docker compose --env-file deploy/docker/modelport.env.example config --quiet
```

The example contains placeholders and is for validation only; do not start a
deployment with its sample credentials.

Open:

- Dashboard: `http://127.0.0.1:5173`
- Messages API: `http://127.0.0.1:17878/v1/messages`
- Chat Completions API: `http://127.0.0.1:17878/v1/chat/completions`
- Liveness: `http://127.0.0.1:17878/livez`

Claude Code uses the host-published backend:

```env
ANTHROPIC_BASE_URL=http://127.0.0.1:17878
ANTHROPIC_AUTH_TOKEN=<same-as-MODELPORT_AUTH_TOKEN>
ANTHROPIC_MODEL=<configured-model-id>
```

Run [Production Acceptance](ACCEPTANCE.md) after startup.

## Daily Commands

```bash
docker compose ps
docker compose logs -f modelport
docker compose logs -f dashboard
docker compose restart modelport
docker compose up -d --build
docker compose down
```

`docker compose down` preserves named volumes; `docker compose down -v` deletes
PostgreSQL and backend data and is irreversible without a backup.

## Storage

Unless `.env` explicitly sets `MODELPORT_DATABASE_URL`, Compose constructs it
for the internal PostgreSQL service. An explicit complete URL overrides that
default. During the compatibility migration, the application stores two
`jsonb` documents in `modelport_state`:

| Namespace | Contents |
| --- | --- |
| `auth` | Users, roles, status, and password hashes. |
| `control` | Teams, API-key hashes, policy, quota, usage, audit, route/provider overrides, credential metadata, health, and tests. |

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
URL. The default command therefore selects PostgreSQL. To explicitly select the
single-instance compatibility deployment instead, use the supplied override:

```bash
docker compose -f docker-compose.yml -f docker-compose.files.yml up -d --build
```

That override removes PostgreSQL from the active services and supplies empty
database URLs. File-backend paths are
`/data/admin-auth.json` and `/data/control-plane.json`. On first PostgreSQL use,
an empty namespace imports an existing corresponding JSON file.

File mode keeps the normalized request and budget ledger only in process
memory. It is suitable for development or a small disposable installation, not
for multi-replica or durable enterprise enforcement. Switch back to the
recommended PostgreSQL deployment with plain `docker compose up -d --build`.

Persistence currently synchronously replaces the complete logical document on
auth/control changes, including the compatibility usage log. Separately,
embedded migrations create normalized tenant, gateway-request, and Provider-
attempt rows, then add hashed idempotency claims, renewable instance leases,
transactional budget accounts/reservations, and append-only evidence events.
Every paid upstream attempt is inserted before egress and finalized at the
response, stream, or expired-lease terminal state. This ledger is the first
relational slice; identity, policy, response replay, and the dashboard log query
still use the compatibility path or remain open.

## Backup

The dashboard's CSRF-protected `POST /admin/backup` download is a redacted
diagnostic snapshot, creates an audit event, and is not a restore artifact. Use
the CLI for a restorable application backup:

```bash
docker compose exec modelport \
  model-port backup export /data/modelport-backup.json
docker compose exec modelport \
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
docker compose stop modelport dashboard
docker compose run --rm modelport \
  model-port backup restore /data/modelport-backup.json --yes
docker compose up -d
```

Restore saves both previous logical values, then writes auth and control
sequentially. It is not an atomic two-document transaction; retain the saved
application values and a database-native backup until the restored service has
passed smoke and login checks.

Keep a database-native backup too:

```bash
docker compose exec postgres pg_dump -U modelport modelport > modelport.sql
docker compose exec -T postgres psql -U modelport modelport < modelport.sql
```

The Compose project has `name: modelport`; a physical volume backup therefore
uses volume `modelport_modelport-postgres`. Prefer `pg_dump` for portable
restore instead of copying a live database directory.

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
directly. Recreate after `.env` changes:

```bash
docker compose up -d --force-recreate modelport
```

See the exact [reload matrix](CONFIGURATION.md#reload-versus-restart).

## Access Scope And Reverse Proxy

Default publishing is loopback-only:

```env
MODELPORT_API_PUBLISH=127.0.0.1:17878
MODELPORT_DASHBOARD_PUBLISH=127.0.0.1:5173
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
published port (for example `127.0.0.1:5173`), which keeps the browser Origin
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
[Local Runtime Integration](LOCAL_RUNTIME.md).

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
- Live-stream completion/final usage and fallback after headers are incomplete.
- Quota enforcement is not a concurrent reservation transaction.
- Provider hostname DNS answers are not revalidated by the SSRF guard.
- Auth/control and retained dashboard usage still live in complete compatibility
  documents; only request/Provider-attempt lifecycle is normalized so far.

These limits are detailed in [Operations](OPERATIONS.md#current-operational-limits).
