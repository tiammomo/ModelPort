# Operations

This guide covers a running ModelPort instance. Docker-specific storage and
network commands are in [Docker Compose](DOCKER.md); systemd is covered in
[systemd deployment](SYSTEMD.md).

## Day-One Checks

Choose commands for the deployment you operate:

| Deployment | Status | Recent gateway logs |
| --- | --- | --- |
| Docker Compose | `docker compose ps` | `docker compose logs --tail=80 modelport` |
| systemd | `systemctl status modelport` | `journalctl -u modelport -n 80` |
| Native source checkout | `scripts/dev.sh status` | `scripts/dev.sh logs` |

For a non-default Compose manifest, add `-f "$MODELPORT_COMPOSE_FILE"` to
Compose commands, as described in [Docker Compose](DOCKER.md). For native
configuration and runtime diagnosis, use `scripts/dev.sh doctor`; startup,
validation, rebuild behavior, and prerequisites live in
[Development](DEVELOPMENT.md#backend).

`scripts/smoke-test.sh` checks liveness, authenticated readiness, and the model
list using the endpoint and credentials in the selected local environment.
It makes no upstream calls by default; `--upstream` explicitly sends a real,
potentially paid request. Health semantics are documented below.

For a release or production trial, use [Production](PRODUCTION.md).

## Runtime Adapter Collection

Configured Runtime Adapters are polled by a provider-neutral, read-only
background collector. Each enabled adapter is attempted immediately at
startup, then on its configured interval. A shared concurrency bound prevents
an adapter fleet from exhausting the process; a failing adapter is isolated
with bounded backoff and sanitized error logging. Accepted Compute inventories
are immutable ledger evidence, while `fresh`, `stale`, and `unavailable` remain
server-owned projections. Shutdown stops new collection attempts and drains
active attempts within a bounded timeout. The collector grants no Runtime
Adapter mutation authority and does not expose an admin API.

## Health Semantics

- `/livez` proves that the HTTP process can answer. It does not inspect storage
  or providers.
- `/health` is minimal when unauthenticated. A valid data-plane credential adds
  configured providers, persisted provider-health records, and storage
  locations.
- `/readyz` requires authentication and verifies that auth/control/governance storage and
  the normalized enterprise ledger can be reached before returning detailed
  diagnostics. It still does not fail merely
  because a Provider is degraded or offline, so it is storage readiness rather
  than an all-provider gate.
- Dashboard setup checks provide configuration diagnostics, not a guarantee
  that every upstream generation will succeed.

## Smart-Router Rollout

Treat a routing-policy change like a production release:

1. Configure groups with `mode = "off"`, validate the configuration, and call
   each candidate explicitly to verify protocol, Tool Use, streaming, limits,
   credentials, and billing metadata.
2. Set `mode = "shadow"` and keep `activation_percent = 0`. Compare the stored
   recommendation with the selected baseline, error rate, latency, cost, and
   downstream application evaluations.
3. Set `mode = "active"` with a small explicit percentage such as 5. Increase
   only after the canary and baseline are comparable over the intended traffic
   classes. Bucketing is deterministic per hashed session ID when supplied,
   otherwise per principal-scoped request ID. Reuse the request ID when retry
   assignment must remain stable.
4. Roll back immediately by setting `MODELPORT_SMART_ROUTING_MODE=shadow` or
   `off` and restarting/reloading the base configuration. Do not edit historical
   decision rows.

Authenticated `GET /admin/router/status` reports the loaded policy, groups,
decision counts, shadow disagreements, selected candidates, and process-local
outcome/latency observations. Prometheus exposes
`modelport_routing_decisions_total` and
`modelport_routing_shadow_disagreements_total`. Durable evidence is stored in
`modelport_routing_decisions` and is also included as `routingDecision` in
request/log API rows. Decision evidence contains IDs, models, scores, bounded
reason codes, and whether session affinity was used; it does not contain the
prompt or raw session header.

Investigate these conditions before increasing activation:

- no eligible candidate: capability, API-key policy/quota, or configuration
  removed every route;
- `all_candidates_cooling`: all otherwise eligible Providers were cooling, so
  the router retained a last-resort candidate instead of failing immediately;
- high shadow disagreement: the configured baseline and policy priors disagree
  frequently and need workload evaluation;
- one candidate dominating every profile: verify per-model pricing and
  quality/latency priors rather than assuming the score is correct.

## Request Logs

The relational request log records:

- request ID, time, identity, API-key/team labels;
- requested and resolved model, provider, and protocol;
- whether the request declares/selects tools or continues a Tool Use exchange;
- aggregate Tool Use outcome (`tool_called`, `continuation_tool_called`,
  `final_answer`, `answered_without_tool`, unobserved completion, or failure);
- bounded traffic class (`business`, `synthetic`, `diagnostic`, or low-priority `batch`);
- stream flag, status/status code, lifecycle latency, stream-only first
  semantic latency, retry/fallback;
- routing mode/profile/policy, selected and recommended candidate, bounded
  reason codes, both route scores, shadow disagreement, and decision ID;
- input/output/cache tokens and estimated cost;
- client IP, request path, and a category-only error message whose diagnostic
  detail is explicitly redacted before persistence.

It intentionally does not store prompts, complete messages, raw request bodies,
raw provider bodies, tool names/arguments/results, or plaintext keys. The
dashboard's protocol JSON panels are reconstructed summaries unless explicitly
labelled otherwise.

The same category-only policy applies to request/attempt rows and
Provider/credential health. The authenticated HTTP caller may still receive a
bounded actionable error for the current request, but that detail is not copied
into durable telemetry.

An HTTP-successful request is not automatically counted as a model tool call.
`tool_called` means a validated response contained at least one tool call;
`final_answer` means a request containing prior tool results completed without
another call. `answered_without_tool` is a successful initial tool-enabled turn
that returned text instead. These are request-level observations; ModelPort
does not execute business tools and therefore cannot by itself certify the
application's end-to-end task result.

Rows contain personal and network metadata, so protect database dumps, CLI
backups, and diagnostic exports and configure an explicit PostgreSQL retention
policy when required.

`GET /admin/logs` supports server-side filters and pages of 1–500 rows; its
`total` and token/cost/rate/latency `summary` cover the full filtered set before
pagination. The summary exposes lifecycle P95 and, for streams with a semantic
first event, TTFT P95. `GET /admin/logs/{id}` retrieves one row and returns a
standard 404 when it does not exist. See
[API](API.md#request-logs-and-latency) for the query contract. Filtering and
pagination reduce response/browser work; the lower time bound is applied in
PostgreSQL before rows are materialized, and detail retrieval uses the request
primary key.

The administrator-only Enterprise Operations page is a separate evidence
surface backed by `GET /admin/enterprise/overview` and
`GET /admin/enterprise/requests`. Its budget rail is backed by
`GET/PUT /admin/enterprise/budget` and the append-only adjustment endpoint. In PostgreSQL mode it performs indexed,
paginated request queries and loads Provider attempts only when an operator
opens a request. Use it for tenant, idempotency, lease, crash-reconciliation,
traffic class, request path, Tool Use lifecycle, latency/TTFT, retry/fallback,
and attempt-level investigation. It intentionally exposes only the presence of
an idempotency claim, never the key/hash or request fingerprint. The in-memory
ledger exists only in tests.

Treat `reservedMicrounits` as in-flight exposure and `settledMicrounits` as
terminal spend. A negative `availableMicrounits` means authoritative settlement
exceeded the earlier maximum estimate or an administrator lowered the limit
below existing exposure; new attempts remain blocked until the balance or
limit is corrected. Do not delete or rewrite evidence rows. Use a signed
adjustment with a Provider invoice, ticket, or object-store reference.

If no usage rows have been persisted, `/admin/logs` can return aggregate
process-metric fallback rows. Treat rows without `requestId` as synthetic
operational summaries, not individual traces. `/admin/latency` uses retained
request latencies for real percentiles when possible and exposes
`percentilesEstimated` and `sampleCount` so the UI can distinguish its
aggregate fallback.

A transport timeout before the downstream response is persisted with
`status=timeout`; other pre-response failures use `error`. Established streams
are finalized at response-body completion, failure, timeout, or drop. Their
`terminalReason` and effective 200/502/504/499 status mapping distinguish the
outcome even though an SSE error cannot rewrite the HTTP 200 already sent to the
client. Inspect the event stream and terminal log together.

The row's `statusCode` is ModelPort's effective client-facing HTTP status for
the pre-response result, not an independent raw-provider field. Valid upstream
statuses can be retained (such as 401); transport failures map to 502. This
makes request and provider-outcome diagnostics reflect the error contract the
client actually encountered.

Token and cost values have separate reconciliation tracks. `costEstimate` is an
operational estimate used for routing and reservations. `actualCost` exists only
when ModelPort receives a trusted Provider-reported amount or applies an exact,
versioned card to Provider-reported usage. `billableCost` is the governed amount
used for settlement and is null when evidence is insufficient. Inspect
`reconciliationStatus` and `pricingEvidence` before treating a row as spend.
`billingMode` still describes token provenance: `upstream-returned` means the
adapter exposed Provider usage, while `local-estimate` means ModelPort used its
request heuristic. Provider usage alone does not make an amount billable.
`firstByteLatencyMs` is null for non-stream requests. For live streams it starts
at the upstream attempt and stops at the first non-empty text delta or tool-call
event, not at an SSE keepalive or metadata-only frame. Live-stream records use
Provider usage when recognized Anthropic/OpenAI usage
appears in the event stream; otherwise they rely on the input estimate and
requested maximum output. The `buffer_stream_text=true` compatibility path
completes the non-stream upstream response before local SSE and can also use
reported usage, while downstream delivery completion or cancellation is
recorded separately.

A request is chargeable only after ModelPort actually starts an upstream
attempt. Attempt-level credential, policy/quota/capability, URL, or
Provider-rate rejection before `send()` can create a zero-usage log row without
incrementing user quota or API-key/team spend. Earlier authentication,
request-shape, model-resolution, global-rate, and stream-permit failures may
return before a persisted usage row exists. They increment the bounded
`modelport_inference_rejections_total` metric by pre-ledger phase and safe error
category. Neither class consumes budget.

## Dashboard Ranges And Retention

Dashboard range cards and charts come from `GET /admin/dashboard`. The backend
aggregates the complete retained usage set within the selected 1-, 3-, 7-day or
custom window before returning request/error buckets, token buckets, model
usage, and range totals. These deployment-health views use `business` traffic
only, so explicit acceptance and diagnosis calls do not distort cost, success,
or trend totals. A real synthetic upstream failure can still update
Provider/credential health and cooldown because the diagnostic observed an
actual dependency failure. The logs page can still select every traffic class.
This aggregation is independent of the logs table's current page. Custom windows
are limited to 90 days.

Use the response metadata when interpreting a chart:

- `rangeDataSource=relational-ledger` uses terminal PostgreSQL request rows;
- `empty` means no relational request covers the window;
- `rangeDataEstimated=false` confirms no process-metric estimate was used;
- `rangeDataAtRetentionLimit=false` confirms there is no document-ring limit.

The query reads only the selected window and current UTC day. Retention is an
explicit preview/apply operation; ModelPort does not silently evict rows from
an in-document ring.

## Data Lifecycle And Retention

Treat persisted data by ownership class:

| Data | Retention rule |
| --- | --- |
| Organizations, projects, environments, Providers, routes, users, and API-key metadata | Configuration authority; retain and back up while referenced. |
| Provider secrets | Keep only in the configured secret input; never copy into logs, backups of public metadata, or the repository. |
| Gateway requests, attempts, routing decisions, and feedback | Operational evidence; default request-detail minimization begins after 30 days. |
| Budget accounts, reservations, and events | Financial/audit evidence; events are append-only and are not ordinary cleanup targets. |
| Sessions and transient login/rate-limit state | Expire through the built-in lifecycle rather than manual table deletion. |
| Acceptance objects | Scripts remove temporary control objects; request-ledger evidence can remain. |

ModelPort never intentionally persists prompt, response, tool name, tool
arguments, tool results, or raw Provider bodies. Retention controls metadata;
it cannot remove content captured by a client, Provider, reverse proxy, custom
logging integration, crash dump, or host outside ModelPort's ledger.

The default policy is:

| Age | Action on explicit apply |
| ---: | --- |
| 30 days | Redact request identity, IP, error, idempotency, and other user-level detail; redact mutable Provider-attempt error/terminal/lease-owner detail; delete routing-decision evidence. Attempt IDs, usage, cost, billing provenance, and request `lastAttemptId` remain where immutable budget/reservation evidence requires them. |
| 90 days | De-identify user-level usage rows while retaining aggregate operational and immutable budget evidence. |
| 395 days (13 months) | Delete ordinary governance audit events. Append-only budget events are retained. |

Configure the policy with
`MODELPORT_REQUEST_DETAIL_RETENTION_DAYS`,
`MODELPORT_USER_USAGE_RETENTION_DAYS`, and
`MODELPORT_AUDIT_RETENTION_DAYS`. The periods must remain ordered
`request detail <= user usage <= audit`. Changing them requires restart.

Retention is administrator-only and always starts with a preview. Both preview
and apply append a `data_retention` governance audit event:

Administrators can run the same preview-first workflow from **Dashboard → 运行设置与运维 → 运维审计 → 数据保留**. The UI keeps apply disabled until a successful preview and surfaces legal-hold state and immutable-budget-evidence retention explicitly.

```http
POST /admin/retention/run
X-ModelPort-CSRF: 1
Content-Type: application/json

{"dryRun":true}
```

The body defaults to `dryRun=true` when the field is omitted. The response
reports `evaluatedAtMs`, three cutoff timestamps, the effective `policy`, and
these preview counts: `requestDetailsRedacted`,
`providerAttemptsRedacted`, `routingDecisionsDeleted`,
`userUsageRowsDeidentified`, and `auditEventsDeleted`. It also reports
`immutableBudgetEventsRetained=true` and
`policy.contentPersistence=false`. A successful preview additionally returns a
random, single-use `previewToken` and its five-minute `previewExpiresAtMs`.
Tokens are process-local because Beta is explicitly single-instance; a restart
invalidates outstanding previews.

Before applying:

1. create, verify, and restore-drill a new database backup;
2. export evidence required by the team's incident, legal, and reconciliation
   policy;
3. have the retention owner review the preview counts and cutoff timestamps;
4. before `previewExpiresAtMs`, send
   `{"dryRun":false,"previewToken":"<value from preview>"}` as the same
   administrator;
5. require `applied=true`, then rerun readiness, budget reconciliation, logs,
   and dashboard checks.

Apply uses the exact effective policy, `evaluatedAtMs`, and cutoffs authorized
by that preview; it does not move the cutoff forward while waiting for
confirmation. Missing, expired, cross-administrator, superseded, or replayed
tokens fail closed. The service consumes a valid token before starting the
operation, including when legal hold prevents mutation or storage returns an
uncertain error. Run a new preview before every retry.

Set `MODELPORT_RETENTION_LEGAL_HOLD=1` during an approved hold and restart. A
preview still works; an apply returns `applied=false` and
`skippedReason="legal_hold"`. When no hold is active, `skippedReason` is null.
Ordinary users and viewers receive 403. The endpoint requires an administrator
console session, `X-ModelPort-CSRF: 1`, and the normal same-origin check.

Budget foreign keys and append-only controls intentionally preserve required
attempt IDs, usage/cost/billing fields, reservations, and budget events. Do not
disable them to make the dashboard look empty. Any deeper physical archive or
partition policy requires separate review and restore evidence.

Migrations preserve normalized request and attempt history. Routing decisions
and feedback cascade with their owning request and store no prompt, response,
or raw session ID. Test migrations against a restored database before every
production upgrade.

Acceptance scripts modify control state and can leave request evidence. For a
zero-residue test, use an isolated ModelPort database, export the result, and
remove that isolated environment. Never point destructive test cleanup at a
shared or production database.

Safe maintenance sequence:

1. Check deployment status and logs using [Day-One Checks](#day-one-checks).
2. Create, verify, and restore-drill a backup.
3. Export the request, usage, routing, and budget evidence required by policy.
4. Apply only the reviewed retention/archive operation.
5. Re-run diagnostics, budget reconciliation, and dashboard totals.

## Quotas And Spend Windows

User quota periods are UTC calendar periods. `daily` resets at 00:00 UTC,
`weekly` at Monday 00:00 UTC, and `monthly` at 00:00 UTC on the first day of the
month. Quota create/update must target a real auth user; the server stores that
user's canonical username. Disabling or suspending the user revokes their keys
and removes their quota rows.

API-key/team spend fields use relational terminal request rows over rolling
5-hour, 24-hour, 7-day, and 30-day windows. The `rateLimited` API field enables
these periodic spend checks; it is not a requests-per-minute switch. Dashboard
API-key “today” counters and team daily/monthly display values use UTC calendar
boundaries.

## Prometheus Metrics

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:38082/metrics
```

Metrics are process-local and reset on restart:

- `modelport_uptime_seconds`
- `modelport_route_{requests,successes,failures,duration_ms}_total`
- `modelport_inference_rejections_total`
- `modelport_message_{requests,successes,failures,duration_ms}_total`
- `modelport_message_{input,output,cache_write,cache_read}_tokens_total`
- `modelport_message_cost_estimate_usd_total`
- `modelport_message_latency_ms` (global histogram)
- `modelport_ledger_operation_failures_total`
- `modelport_ledger_operation_degraded`
- `modelport_ledger_reconciled_{requests,attempts}_total`
- `modelport_ledger_pending_finalizers`
- `modelport_gateway_ready`
- `modelport_database_ready`
- `modelport_database_pool_connections`, `modelport_database_pool_max_connections`, and `modelport_database_pool_utilization_ratio`
- `modelport_provider_available` and `modelport_provider_cooldown`
- `modelport_local_scheduler_{running,interactive_queued,batch_queued,users_queued,estimated_service_ms,oldest_interactive_wait_ms,oldest_batch_wait_ms}`
- `modelport_stream_permits_available`

Message series have `provider`, `model`, `traffic_class`, and `stream` labels.
Model names are operator-controlled and can create high cardinality when
arbitrary passthrough is enabled; overflow series remain separated by bounded
traffic class. Rejection series use fixed route, phase, and category labels and
never retain the client/provider error body. Stream duration covers the
complete response-body lifecycle, while `firstByteLatencyMs` measures the first
semantic text or Tool Call event.

Ledger operation labels are bounded to gateway-owned operations such as request
and attempt finalization, lease renewal/reconciliation, and finalizer spawning.
The degraded gauge records whether the most recent operation of that type
failed. A degraded operation also makes `/readyz` fail until a later successful
operation proves recovery. `modelport_ledger_pending_finalizers` should normally
return to zero promptly and is drained for up to
`MODELPORT_FINALIZATION_DRAIN_TIMEOUT_SECONDS` during graceful shutdown.

The official minimum monitoring package is in
[`deploy/observability`](../deploy/observability/README.md). It includes
Prometheus alerts, an importable Chinese-first Grafana dashboard, an optional
authenticated `/readyz` Blackbox probe, and the
[alert runbook](OBSERVABILITY_RUNBOOK.md). Remaining budget, PostgreSQL acquire
latency, reconciled Provider invoice cost, configured stream capacity, and
per-Provider latency percentiles are not emitted; the package labels these
dependencies instead of inventing proxy precision.

## Performance And Benchmarking

ModelPort targets single-host personal and small-team traffic. Upstream
queueing and generation usually dominate latency, but authentication, routing,
policy, protocol conversion, streaming, metrics, and PostgreSQL evidence add
local work. The project does not claim a universal throughput or latency target
without a dated benchmark.

Local endpoints, default 30 iterations:

```bash
scripts/bench.sh
```

Real upstream, default 3 paid synthetic calls:

```bash
scripts/bench.sh --upstream
scripts/bench.sh --upstream -n 5
```

Record commit/build profile, hardware, retained-row count, Provider/model,
stream mode, context/output size, concurrency, local overhead, semantic TTFT,
full latency, and terminal failure count. Compare like-for-like workload
classes; an initial stream HTTP 200 is not a completed sample.

Tune only after measurement:

- reduce concurrency when Provider limits, local runtime queues, or database
  latency are saturated;
- size `MODELPORT_MAX_CONCURRENT_STREAMS` for simultaneously open bodies, not
  request-start throughput;
- keep request, response, SSE, idle, and byte limits finite;
- diagnose Provider/network/runtime latency before extending timeouts;
- apply PostgreSQL retention or partitioning only when observed cardinality and
  query plans require it;
- measure Tool Use and large-context traffic separately from tiny text calls.

Process-local limits and Provider behavior make results deployment-specific.
They cannot be tuned into multi-instance correctness or invoice-grade
reconciliation.

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

For the recommended PostgreSQL Compose deployment, the complete single-host
backup and non-destructive restore drill are:

```bash
scripts/backup-compose.sh create
scripts/backup-compose.sh verify backups/modelport-<UTC>.tar.gz
scripts/backup-compose.sh drill backups/modelport-<UTC>.tar.gz
```

New schema-v2 archives include PostgreSQL, checksums, and secret-free deployment
provenance. They exclude `config.toml` and the Compose environment file; recover
those from Git and the secret manager. Legacy schema-v1 archives include both
files and may contain plaintext Provider and service credentials. Verification
warns when it encounters one. Keep every archive access-controlled; `drill`
restores into an isolated temporary PostgreSQL container and verifies required
application namespaces without stopping or modifying production.

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
control values next to the supplied backup path, verifies both observed
revisions, and replaces both rows in one PostgreSQL transaction. A concurrent
writer causes the entire restore to fail with a state conflict. Keep the
database-native dump produced by `backup-compose.sh`; the CLI export contains
only the two logical auth/control documents and not the operational ledger.
Store both backup forms with restrictive permissions.

Before changing the PostgreSQL major version or moving to a managed database,
follow [PostgreSQL Migration](POSTGRESQL_MIGRATION.md). Run
`scripts/database-preflight.sh` before a full local Compose update; a declared
image or volume mismatch is a stop condition.

## Provider Diagnosis

Use this order:

1. `scripts/config-validate.sh`
2. Dashboard provider test/model discovery through
   `POST /admin/providers/{provider_id}/models` (a CSRF-protected mutating action)
3. `scripts/provider-matrix.sh --model provider:model --evidence artifacts/provider-matrix.json`
4. `scripts/tool-use-acceptance.sh --upstream` only when Tool Use should work
5. backend request log and SSE event body
6. upstream account status, permission, and balance; for the official
   `deepseek` provider, use the dashboard's live balance action or
   `POST /admin/providers/deepseek/balance`

Provider testing and matrix scripts can make paid calls. A configured model or
HTTP 200 at stream start is not evidence of a completed generation. The
optional matrix artifact contains no gateway URL, credential, or body; it
records the commit, source state, traffic class, and completed check outcomes.

Non-local/non-custom Providers must use HTTPS. If a trusted internal upstream
is available only over HTTP, `MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` is the
explicit restart-required escape hatch. Plain HTTP exposes the Provider key and
prompt/response content to the network path; never use it for an untrusted LAN
or Internet endpoint. Local/custom runtime classes retain HTTP support for
controlled local integration.

## Idempotency And Lease Reconciliation

`Idempotency-Key` is an at-most-once Provider-call claim scoped to the current
tenant. The database stores only its SHA-256 digest and a request fingerprint.
Duplicate claims return HTTP 409 before Provider routing. Terminal response
replay is deliberately not yet available, so a same-body retry also receives
409 after the first request completes. Database-free development mode is only
process-local and loses claims on restart; enterprise mode requires PostgreSQL.

Every active relational request owns a lease, renewed every one-third of
`MODELPORT_LEDGER_LEASE_TTL_SECS`. The guard remains alive through the complete
stream body, including slow delivery. At startup and every
`MODELPORT_LEDGER_RECONCILE_INTERVAL_SECS`, ModelPort terminalizes expired
request and attempt rows with:

- `state=failed` and `status_code=500`;
- `terminal_reason=lease_expired_unreconciled`;
- `billing_mode=unreconciled` and `chargeable=false`.

This closes orphaned lifecycle rows without inventing Provider usage or cost.
It does not prove whether the Provider accepted a request immediately before a
process failure; future settlement must use external evidence and an
append-only adjustment. Keep the lease TTL above the worst expected scheduler
pause and the reconciliation interval below the TTL.

## Common Incidents

| Symptom | Check |
| --- | --- |
| 401 on `/v1/*` | Header token, legacy-token policy, API-key status/expiry, and whether its owner still exists and is active. |
| 429 on admin login | More than four password hashes stayed busy for the five-second queue window; honor `Retry-After` and investigate abusive or overloaded login traffic. |
| 403 from policy | User/team status, provider/model allowlist, client IP and trusted proxy configuration. |
| 429 with `Retry-After` | Process-local request-rate limit or exhausted concurrent-stream permits; inspect the error message and active stream duration. |
| 429 `quota_exceeded` | API-key spend window or user quota. |
| 409 `idempotency_conflict` | The tenant already claimed that key. Preserve the original outcome; response replay is not available, and a new key authorizes a new Provider call. |
| 400 before upstream | Model/messages/Tool Use guardrails; `max_tokens` is required, positive, and capped by `MODELPORT_MAX_OUTPUT_TOKENS`. |
| 400 deleting a team | One or more active or revoked API keys still reference it; reassign or delete those keys first. |
| 413 | Request body exceeds the Axum/Nginx limit. |
| 502 before a requested stream starts | Upstream returned 204, omitted/returned a non-SSE content type, returned an invalid status, or hit a pre-header transport/protocol failure; inspect the bounded redacted error and fallback attempts. |
| SSE `event: error` with HTTP 200 | Upstream failed after stream headers or ended without `message_stop` / `[DONE]` / `finish_reason`; inspect the event and backend log. |
| SSE ends at `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` | Expected total upstream lifecycle limit. Check the configured total and idle timeouts before increasing either; post-header failure is reported in the event stream. |
| Provider is cooling down | Recent retryable/account failures; ordinary non-retryable 4xx responses do not trigger cooldown. Verify key, rate limit, and balance. |
| Provider pool has no usable credential | `failover`/`round_robin` fail closed when every credential is disabled, cooling down, or missing its environment value; repair the pool or verify the next Provider candidate. |
| CPA request shows excessive attempts or latency | Verify CPA `request-retry: 0` and a bounded `max-retry-credentials`; ModelPort already owns retry/fallback and records each ModelPort attempt. |
| CPA returns 401 | Check the CPA client key in `CPA_CODEX_API_KEY`/`CPA_CLAUDE_API_KEY`, not the upstream OAuth token; then inspect CPA auth state without copying auth files into ModelPort. |
| CPA Claude sends requests to `/v1/v1/messages` | Remove `/v1` from `CPA_CLAUDE_BASE_URL`; only `CPA_CODEX_BASE_URL` ends in `/v1`. |
| CPA model is visible but calls fail | Catalog discovery is availability metadata, not entitlement. Call the provider-qualified model and verify the exact account, protocol, stream, and Tool Use path. |
| Dashboard cross-origin failure | Use a same-origin reverse proxy; `MODELPORT_ALLOWED_ORIGINS` is not a CORS switch. |
| `config validate` rejects enterprise mode | Set a valid `MODELPORT_DATABASE_URL`, use `MODELPORT_DATABASE_TLS_MODE=verify-full`, and fix any reported pool, lease, proxy, or origin syntax before restarting. Validation errors intentionally prevent the server from binding. |
| Auth/control state write latency grows | Inspect PostgreSQL latency and the one-connection document workers. Operational usage and audit rows are independent; low-frequency identity/policy definitions still replace their complete PostgreSQL documents. |
| `/readyz` reports enterprise ledger failure | Check PostgreSQL reachability, migration permissions, pool exhaustion, TLS mode, root certificate, and hostname verification. |
| `lease_expired_unreconciled` rows appear | Check process restarts, runtime stalls, PostgreSQL availability, and heartbeat warnings. Do not bill these rows without Provider evidence. |

## Current Operational Limits

- Rate limits and sessions are process-local; there is no multi-instance
  coordination.
- Concurrent-stream permits are process-local and remain occupied through
  response body completion/drop.
- User/API-key/team quota and spend admission is transactionally reserved in
  PostgreSQL against settled plus open amounts. One logical request consumes
  one request unit even when retries add token/cost reservations; terminal work
  settles actual usage or releases non-chargeable capacity.
- Provider hostnames are resolved, private/metadata answers are rejected by
  default, and validated addresses are pinned for the outbound connection.
  Explicit private-Provider approval and host egress policy remain operator-
  owned controls.
- Low-frequency auth/control persistence still replaces complete JSON
  documents, but revision compare-and-swap rejects stale writers and readiness
  fails closed after detecting a newer database revision. This prevents silent
  lost updates; it does not provide relational cross-domain transactions.
- Request and Provider-attempt rows are normalized, tenant-scoped, leased, and
  expired rows are terminalized automatically. Crash recovery cannot infer
  whether a Provider accepted the request or reconstruct missing token usage,
  so expired rows remain explicitly unbilled and unreconciled.
- Live-stream terminal status, duration, metrics, and known Provider outcome are
  reconciled in-process. Final usage/cost commonly remains estimated, and
  fallback cannot restart after headers.
- Established upstream live streams remain bounded by the total request timeout,
  resettable idle timeout, completion/cancellation, and SSE byte limits.
- `/readyz` gates on readable auth/control storage and a reachable normalized
  ledger, not on every Provider.

These limits are acceptable for the intended single-host/small-team profile but
must be addressed before public multi-tenant or horizontally scaled use.

## Upgrade Checklist

The complete safe-stop and paired rollback procedure is maintained in
[Upgrading and Rollback](UPGRADING.md). This abbreviated list is not a
zero-downtime promise.

1. Read the release notes and compare environment/configuration changes.
2. Export and validate a complete backup.
3. Run configuration validation with the new binary/image.
4. Restart or recreate the service; do not assume every setting hot-reloads.
5. Run smoke, then provider/tool acceptance appropriate to the change.
6. Watch logs, SSE errors, storage writes, and estimated spend after rollout.
