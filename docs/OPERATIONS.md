# Operations

This guide covers a running ModelPort instance. Docker-specific storage and
network commands are in [Docker Compose](DOCKER.md); systemd is covered in
[systemd deployment](SYSTEMD.md).

## Day-One Checks

```bash
scripts/config-validate.sh
scripts/status.sh
scripts/smoke-test.sh
```

When the service is not already healthy, `scripts/start.sh` reuses the release
binary only when it is newer than the Rust sources, Cargo manifest/lockfile,
and pinned toolchain file; otherwise it rebuilds with
`cargo build --release --locked`. Use
`MODELPORT_FORCE_BUILD=1 scripts/start.sh` to force a rebuild.

`smoke-test.sh` checks liveness, authenticated diagnostics, and the model list.
It does not call an upstream by default. A real call can cost money:

```bash
scripts/smoke-test.sh --upstream
```

For a release or production trial, use [Production Acceptance](ACCEPTANCE.md).

`scripts/config-validate.sh` uses the same configuration issue set as server
startup. Validation errors—including placeholders, broken provider/alias
relationships, and invalid or zero-valued non-zero guardrails—also make the
server refuse to start. Warnings remain visible in the service log but do not
block startup.

## Health Semantics

- `/livez` proves that the HTTP process can answer. It does not inspect storage
  or providers.
- `/health` is minimal when unauthenticated. A valid data-plane credential adds
  configured providers, persisted provider-health records, and storage
  locations.
- `/readyz` requires authentication and verifies that auth/control storage can
  be read before returning detailed diagnostics. It still does not fail merely
  because a Provider is degraded or offline, so it is storage readiness rather
  than an all-provider gate.
- Dashboard setup checks provide configuration diagnostics, not a guarantee
  that every upstream generation will succeed.

## Request Logs

The persisted usage log records:

- request ID, time, identity, API-key/team labels;
- requested and resolved model, provider, and protocol;
- stream flag, status/status code, latency, retry/fallback;
- input/output/cache tokens and estimated cost;
- client IP, request path, and a bounded error message.

It intentionally does not store prompts, complete messages, raw request bodies,
raw provider bodies, or plaintext keys. The dashboard's protocol JSON panels
are reconstructed summaries unless explicitly labelled otherwise.

`MODELPORT_USAGE_LOG_LIMIT` defaults to 5,000 records. Records contain personal
and network metadata, so protect database dumps, CLI backups, and diagnostic
exports according to your retention policy.

`GET /admin/logs` supports server-side filters and pages of 1–500 rows; its
`total` and token/cost/rate `summary` cover the full filtered set before
pagination. `GET /admin/logs/{id}` retrieves one row and returns a standard 404
when it has expired or does not exist. See [API](API.md#request-logs-and-latency)
for the query contract. Filtering and pagination reduce response/browser work,
but the backend still materializes and scans the retained control document in
memory; they are not indexed PostgreSQL row queries.

If no usage rows have been persisted, `/admin/logs` can return aggregate
process-metric fallback rows. Treat rows without `requestId` as synthetic
operational summaries, not individual traces. `/admin/latency` uses retained
request latencies for real percentiles when possible and exposes
`percentilesEstimated` and `sampleCount` so the UI can distinguish its
aggregate fallback.

A transport timeout before the downstream response is persisted with
`status=timeout`; other pre-response failures use `error`. A live-stream idle
timeout after HTTP/SSE headers remains outside final lifecycle reconciliation
and may leave the accepted row at its earlier status. Inspect the event stream
and service log as well as the status filter.

The row's `statusCode` is ModelPort's effective client-facing HTTP status for
the pre-response result, not an independent raw-provider field. Valid upstream
statuses can be retained (such as 401); transport failures map to 502. This
makes request and provider-outcome diagnostics reflect the error contract the
client actually encountered.

Token and cost values are operational estimates, not invoices. Inspect
`billingMode` for provenance: `upstream-returned` means the completed adapter
path exposed Provider-reported token counts, while `local-estimate` means
ModelPort used its request heuristic. Both use ModelPort's local pricing table.
Normal live-stream records may rely on the input estimate and requested maximum
output because final usage is not yet reconciled through the request lifecycle.
The `buffer_stream_text=true` compatibility path is an exception: it completes
the non-stream upstream response before local SSE and can use reported usage,
although it still does not observe downstream delivery completion.

A request is chargeable only after ModelPort actually starts an upstream
attempt. Attempt-level credential, policy/quota/capability, URL, or
Provider-rate rejection before `send()` can create a zero-usage log row without
incrementing user quota or API-key/team spend. Earlier authentication,
request-shape, model-resolution, global-rate, and stream-permit failures may
return before a persisted usage row exists. Neither class consumes budget.

## Dashboard Ranges And Retention

Dashboard range cards and charts come from `GET /admin/dashboard`. The backend
aggregates the complete retained usage set within the selected 1-, 3-, 7-day or
custom window before returning request/error buckets, token buckets, model
usage, and range totals. This aggregation is independent of the logs table's
current page. Custom windows are limited to 90 days.

Use the response metadata when interpreting a chart:

- `rangeDataSource=persisted-usage` uses retained request rows;
- `process-metrics-estimate` is a process-lifetime fallback and is explicitly
  marked by `rangeDataEstimated=true`;
- `empty` means neither source covers the window;
- `rangeDataAtRetentionLimit=true` means the store has reached
  `MODELPORT_USAGE_LOG_LIMIT`, so older data may already have been evicted.

Server-side full-window aggregation does not override retention. The default
5,000-row cap can make a long or busy range incomplete; increase retention only
after considering complete-document persistence cost and personal-data policy.

## Quotas And Spend Windows

User quota periods are UTC calendar periods. `daily` resets at 00:00 UTC,
`weekly` at Monday 00:00 UTC, and `monthly` at 00:00 UTC on the first day of the
month. Quota create/update must target a real auth user; the server stores that
user's canonical username. Disabling or suspending the user revokes their keys
and removes their quota rows.

API-key/team spend fields use rolling windows instead: 5 hours, 24 hours, 7
days, and 30 days. For API keys, the legacy JSON/API field `rateLimited` enables
these periodic spend checks; it is not a requests-per-minute switch. The
dashboard deliberately calls it “periodic spend limits”. The spend ledger uses
hour buckets and includes the oldest overlapping hour in full, so boundary
checks are conservative and may include almost one extra hour.

## Prometheus Metrics

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/metrics
```

Metrics are process-local and reset on restart:

- `modelport_uptime_seconds`
- `modelport_route_{requests,successes,failures,duration_ms}_total`
- `modelport_message_{requests,successes,failures,duration_ms}_total`
- `modelport_message_{input,output,cache_write,cache_read}_tokens_total`
- `modelport_message_cost_estimate_usd_total`

Message series have `provider`, `model`, and `stream` labels. Model names are
operator-controlled and can create high cardinality when arbitrary passthrough
is enabled. Stream duration currently measures request setup/acceptance rather
than complete generation time.

## Streaming Concurrency

`MODELPORT_MAX_CONCURRENT_STREAMS` bounds established or establishing streaming
requests independently of the normal Axum request service future. When unset,
it inherits the effective `MODELPORT_MAX_CONCURRENT_REQUESTS`. The stream permit
is retained by the returned response body and is released only when that body
finishes or is dropped, so slow readers and abandoned clients remain counted.

If no permit is immediately available, ModelPort returns HTTP 429
`rate_limit_error` with `Retry-After: 1` before an upstream attempt. It does not
consume quota/spend and may return before a persisted usage row is created.
Clients should back off; operators should inspect client cancellation, idle timeout,
stream byte limits, and Provider latency before raising the cap. This semaphore
is process-local and requires restart/recreate after a configuration change.

## Configuration Reload

The dashboard Operations tab can reload the base configuration. Provider,
model, alias, and route values can update for new requests. Process layers,
security policies, transport settings, storage, sessions, and newly introduced
process environment variables require a restart. Use the full matrix in
[Configuration](CONFIGURATION.md#reload-versus-restart).

Dashboard Settings exposes effective server/auth/rate values as read-only
runtime facts. Default provider and provider order remain runtime control-plane
operations managed from the model/provider controls or API; the Settings form
does not pretend to persist service-level fields. Edit environment/TOML and
restart for those fields.

## Backup And Restore

There are two different exports:

1. `POST /admin/backup` is a redacted diagnostic snapshot. It requires an admin
   session and CSRF header, creates an audit event, contains user/usage data but
   no plaintext key, and is not a full restore artifact.
2. The CLI backup contains password and API-key hashes and can restore state.
   Treat it like a credential database.

```bash
model-port backup export /secure/modelport-backup.json
model-port backup validate /secure/modelport-backup.json
model-port backup restore /secure/modelport-backup.json --yes
```

Both `backup validate` and `backup restore` first deserialize the complete auth
and control documents. Auth validation rejects empty/duplicate user IDs,
case-insensitive duplicate usernames, invalid email/role/status/password-hash
records, and any non-empty user set without an active administrator. Control
records must deserialize into the current control schema. This catches corrupt
or structurally incompatible data before writes; it is not proof that every
business relationship or external credential is still usable.

Stop writers before restore. The command saves the previous logical auth and
control values next to the supplied backup path before replacing them. Auth and
control are then replaced sequentially, not in one cross-document transaction;
a second-write failure can require recovery from those saved values. For
PostgreSQL deployments, also keep normal `pg_dump` backups; for JSON storage,
back up the state directory with restrictive permissions.

## Provider Diagnosis

Use this order:

1. `scripts/config-validate.sh`
2. Dashboard provider test/model discovery through
   `POST /admin/providers/{provider_id}/models` (a CSRF-protected mutating action)
3. `scripts/provider-matrix.sh --model provider:model`
4. `scripts/tool-use-acceptance.sh --upstream` only when Tool Use should work
5. backend request log and SSE event body
6. upstream account status, permission, and balance

Provider testing and matrix scripts can make paid calls. A configured model or
HTTP 200 at stream start is not evidence of a completed generation.

Non-local/non-custom Providers must use HTTPS. If a trusted internal upstream
is available only over HTTP, `MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` is the
explicit restart-required escape hatch. Plain HTTP exposes the Provider key and
prompt/response content to the network path; never use it for an untrusted LAN
or Internet endpoint. Local/custom runtime classes retain HTTP support for
controlled local integration.

## Common Incidents

| Symptom | Check |
| --- | --- |
| 401 on `/v1/*` | Header token, legacy-token policy, API-key status/expiry, and whether its owner still exists and is active. |
| 429 on admin login | More than four password hashes stayed busy for the five-second queue window; honor `Retry-After` and investigate abusive or overloaded login traffic. |
| 403 from policy | User/team status, provider/model allowlist, client IP and trusted proxy configuration. |
| 429 with `Retry-After` | Process-local request-rate limit or exhausted concurrent-stream permits; inspect the error message and active stream duration. |
| 429 `quota_exceeded` | API-key spend window or user quota. |
| 400 before upstream | Model/messages/Tool Use guardrails; `max_tokens` is required, positive, and capped by `MODELPORT_MAX_OUTPUT_TOKENS`. |
| 400 deleting a team | One or more active or revoked API keys still reference it; reassign or delete those keys first. |
| 413 | Request body exceeds the Axum/Nginx limit. |
| 502 before a requested stream starts | Upstream returned 204, omitted/returned a non-SSE content type, returned an invalid status, or hit a pre-header transport/protocol failure; inspect the bounded redacted error and fallback attempts. |
| SSE `event: error` with HTTP 200 | Upstream failed after stream headers or ended without `message_stop` / `[DONE]` / `finish_reason`; inspect the event and backend log. |
| Active SSE exceeds `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` | Expected: that value governs only the SSE handshake. Check the resettable stream idle timeout and byte ceilings for the established phase. |
| Provider is cooling down | Recent retryable/account failures; ordinary non-retryable 4xx responses do not trigger cooldown. Verify key, rate limit, and balance. |
| Provider pool has no usable credential | `failover`/`round_robin` fail closed when every credential is disabled, cooling down, or missing its environment value; repair the pool or verify the next Provider candidate. |
| Dashboard cross-origin failure | Use a same-origin reverse proxy; `MODELPORT_ALLOWED_ORIGINS` is not a CORS switch. |
| State write latency grows | Lower usage retention or move from JSON files to PostgreSQL; complete-document writes remain synchronous. |

## Current Operational Limits

- Rate limits and sessions are process-local; there is no multi-instance
  coordination.
- Concurrent-stream permits are process-local and remain occupied through
  response body completion/drop.
- Quota checks and usage updates are not transactional reservations. Parallel
  requests can overshoot a tight limit.
- API-key/team rolling spend uses hourly buckets; the oldest overlapping hour
  is conservatively counted in full.
- Provider URL checks do not revalidate DNS answers against private ranges.
- Auth/control persistence synchronously replaces complete JSON documents.
- Live-stream completion, final cost/usage, provider health, and fallback are
  not fully reconciled after headers have been sent.
- Established live streams have no fixed total-duration timeout; an active
  stream ends through completion, cancellation, idle timeout, or a byte limit.
- `/readyz` gates on readable auth/control storage, not on every Provider.

These limits are acceptable for the intended single-host/small-team profile but
must be addressed before public multi-tenant or horizontally scaled use.

## Upgrade Checklist

1. Read the release notes and compare environment/configuration changes.
2. Export and validate a complete backup.
3. Run configuration validation with the new binary/image.
4. Restart or recreate the service; do not assume every setting hot-reloads.
5. Run smoke, then provider/tool acceptance appropriate to the change.
6. Watch logs, SSE errors, storage writes, and estimated spend after rollout.
