# systemd Deployment

The repository includes a hardened single-host backend unit. This deployment
does not install the React dashboard; serve a separately built dashboard through
a same-origin reverse proxy if it is required.

## Install

Build and install the backend:

```bash
scripts/build-release.sh
sudo install -m 0755 target/release/model-port /usr/local/bin/model-port
sudo install -d -m 0750 /etc/modelport
sudo install -m 0640 deploy/systemd/modelport.env.example /etc/modelport/modelport.env
sudo install -m 0644 deploy/systemd/modelport.service /etc/systemd/system/modelport.service
```

Edit `/etc/modelport/modelport.env` and replace every required placeholder. The
file contains router, admin, database, and provider credentials; restrict access
to administrators.

Then enable the unit:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now modelport
sudo systemctl status modelport
```

## State Layout

The unit uses:

```text
StateDirectory=modelport
WorkingDirectory=/var/lib/modelport
MODELPORT_STATE_DIR=/var/lib/modelport
MODELPORT_CONTROL_STORE=/var/lib/modelport/control-plane.json
```

systemd creates `/var/lib/modelport` for the dynamic service user with mode
`0700`. The auth JSON file and control JSON file are writable despite
`ProtectSystem=strict`. Do not remove `StateDirectory` while using the file
backend.

When `MODELPORT_DATABASE_URL` is set, auth and control documents are stored in
PostgreSQL, but the state directory remains useful for backup files and a
consistent working directory. The current PostgreSQL client uses `NoTls`; keep
the database on the same host or a network path protected by an appropriate
private boundary/tunnel, and never send credentials or state across an
untrusted network. Native TLS transport is not implemented. Test connectivity
before switching storage.

## Validate And Observe

Validate using the same environment file without shell-expanding or printing
its values:

```bash
sudo systemctl stop modelport
sudo systemd-run --wait --pipe --collect \
  --property=EnvironmentFile=/etc/modelport/modelport.env \
  /usr/local/bin/model-port config validate
sudo systemctl start modelport
```

This transient command runs as root but only reads configuration. For policies
that prohibit transient units, add the validation command as an `ExecStartPre`
drop-in so it uses the service's normal identity and environment. Never paste
the environment file into an issue.

Logs and health:

```bash
sudo journalctl -u modelport -f
curl -fsS http://127.0.0.1:17878/livez
curl -fsS -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/readyz
```

`readyz` checks auth/control storage and returns authenticated diagnostics; it
does not gate on every Provider. See [Operations](OPERATIONS.md#health-semantics).

## Reverse Proxy And Dashboard

Keep `MODELPORT_BIND=127.0.0.1:17878` when Nginx/Caddy runs on the same host.
Expose one HTTPS origin that serves the dashboard and proxies `/admin`, `/v1`,
`/livez`, `/readyz`, `/health`, and `/metrics` to the backend.

Set:

```env
MODELPORT_ADMIN_COOKIE_SECURE=1
MODELPORT_ALLOWED_ORIGINS=https://modelport.example.com
MODELPORT_TRUSTED_PROXIES=127.0.0.1,::1
```

`MODELPORT_ALLOWED_ORIGINS` validates dashboard writes; it does not enable
browser CORS. A same-origin proxy is the supported layout.

Preserve the original Host authority including a non-default port. For Nginx,
use `proxy_set_header Host $http_host`; `$host` may drop the port and cause the
Origin/Host write check to fail. A single-hop proxy should overwrite
`X-Forwarded-For` with `$remote_addr`. ModelPort accepts forwarded headers only
from `MODELPORT_TRUSTED_PROXIES` and removes trusted hops from the right-hand end
of the chain, so configure every trusted hop explicitly.

## Backup And Upgrade

```bash
sudo systemctl stop modelport
sudo systemd-run --wait --pipe --collect \
  --property=EnvironmentFile=/etc/modelport/modelport.env \
  /usr/local/bin/model-port backup export /var/lib/modelport/backup.json
sudo systemd-run --wait --pipe --collect \
  --property=EnvironmentFile=/etc/modelport/modelport.env \
  /usr/local/bin/model-port backup validate /var/lib/modelport/backup.json
sudo install -m 0755 target/release/model-port /usr/local/bin/model-port
sudo systemctl start modelport
```

The transient CLI receives the service EnvironmentFile. It runs as root for this
offline maintenance operation; keep the backup mode restrictive and remove or
relocate it after validation. For PostgreSQL, use `pg_dump` in addition to the
application backup.

After upgrading, run smoke and the acceptance checks appropriate to the change.

## Hardening Notes

The shipped unit uses `DynamicUser`, a private temporary directory, no ambient
capabilities, a strict filesystem, and a restrictive umask. Review rather than
blindly weaken these controls when adding a TLS key, custom CA, Unix socket, or
external secret agent. Keep provider keys out of the unit file itself; put them
in the protected EnvironmentFile or a systemd credential mechanism.
