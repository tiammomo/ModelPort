# Production Acceptance

This checklist is for the main ModelPort audience: personal use and small teams running a lightweight self-hosted gateway.

## Run The Script

Start ModelPort first, then run:

```bash
scripts/acceptance.sh
```

Default mode verifies the control plane and safety policies without making a real upstream model call.

To include one real `/v1/messages` call through the created API key:

```bash
scripts/acceptance.sh --upstream
```

## What It Checks

The script verifies:

- `/health` is reachable.
- Dashboard URL is reachable when `MODELPORT_DASHBOARD_URL` points to it.
- Admin login works.
- Authenticated `/v1/models` works.
- A temporary user can be created.
- A temporary API key can be created.
- API key IP restriction rejects a disallowed client IP.
- API key spend limit rejects an over-limit request before upstream routing.
- Audit events are recorded.
- Full local backup export and validation work.
- Temporary user and key are cleaned up.

`--upstream` additionally verifies:

- The same temporary API key can make a successful real model request when IP policy allows it.

## Environment

The script reads `.env` through the shared script loader. Required values:

```env
MODELPORT_BIND=127.0.0.1:17878
MODELPORT_AUTH_TOKEN=...
MODELPORT_ADMIN_USERNAME=admin
MODELPORT_ADMIN_PASSWORD=...
```

Optional:

```env
MODELPORT_DASHBOARD_URL=http://127.0.0.1:5173
```

The script requires `curl` and `node`. `node` is already part of the dashboard toolchain.

## Docker Compose

For Docker Compose:

```bash
cp deploy/docker/modelport.env.example .env
nano .env
docker compose up -d --build
MODELPORT_DASHBOARD_URL=http://127.0.0.1:5173 scripts/acceptance.sh
```

If you published the dashboard or API on different host ports, adjust `MODELPORT_BIND` and `MODELPORT_DASHBOARD_URL` in `.env` or in the command environment.

## Interpreting Results

Passing default acceptance means ModelPort is ready for personal or small-team trial production.

Passing `--upstream` means the full path is working:

```text
Claude-compatible client -> ModelPort auth/policy -> provider route -> upstream model
```

If default acceptance passes but `--upstream` fails, investigate provider credentials, model names, base URL, or upstream quota.
