# Performance And Efficiency

ModelPort is designed for single-host personal and small-team traffic. The
gateway adds one local HTTP hop, authentication/policy work, JSON or SSE
conversion, metrics, and a synchronous control-state write per completed
request. Upstream queueing and generation will often dominate latency, but the
repository does not claim a universal throughput or latency target without a
dated benchmark.

## Work Per Request

- Axum/Tokio handles ingress asynchronously with a process-wide concurrency
  limit.
- reqwest/rustls reuses upstream connections and disables redirects.
- Non-stream responses are bounded and parsed before response mapping.
- Normal streams are converted frame by frame with idle and total-byte limits.
  The general request timeout covers only their response-header handshake; an
  established stream has no fixed wall-clock lifetime while chunks continue
  arriving within the idle and byte limits.
  A separate process-local stream semaphore is acquired before the upstream
  attempt and remains attached to the downstream response body until completion
  or drop. This prevents the handler-return boundary from hiding long-lived
  stream occupancy; immediate exhaustion returns 429 rather than queueing.
  A provider configured with `buffer_stream_text` intentionally waits for a
  complete non-stream generation and conversion, captures reported usage, and
  only then emits local SSE. This removes upstream generation from the
  post-header phase but makes time to first byte equal to full upstream
  generation plus conversion.
- Authentication, model routing, rate policy, quota checks, credential health,
  pricing estimates, metrics, and usage records add local work.
- Auth and control persistence currently replaces a complete logical JSON
  document synchronously. PostgreSQL stores two `jsonb` rows; file mode rewrites
  JSON files. This is measurable write amplification as usage/audit state grows.
- Log filtering and pagination happen server-side for the HTTP contract, but
  still materialize and scan retained usage rows in memory before slicing the
  page. They reduce response/UI work, not complete-document storage cost.
- Dashboard trend aggregation also scans the complete retained usage set for
  the selected window rather than the visible logs page. It is bounded to 90
  days and by `MODELPORT_USAGE_LOG_LIMIT`; reaching the retention cap can make
  an older range incomplete.

## Benchmark

Local endpoints, default 30 iterations:

```bash
scripts/bench.sh
```

Real upstream, default 3 paid calls:

```bash
scripts/bench.sh --upstream
scripts/bench.sh --upstream -n 5
```

Record at least:

- date, commit, build profile, CPU/RAM and OS;
- storage backend and retained usage-record count;
- provider/model, stream mode, context/output size;
- local endpoint p50/p95 and end-to-end client latency;
- first content delta and complete generation separately;
- failure/SSE-error count, not only initial HTTP status.

`/livez` measures HTTP/process overhead. `/v1/models` adds authentication and
catalog generation. `/v1/messages` is not a gateway-only benchmark because it
includes provider behavior and a persistence write.

## Metrics

```bash
curl -sS \
  -H "x-api-key: $MODELPORT_AUTH_TOKEN" \
  http://127.0.0.1:17878/metrics
```

Current process-local series:

- `modelport_uptime_seconds`
- `modelport_route_{requests,successes,failures,duration_ms}_total`
- `modelport_message_{requests,successes,failures,duration_ms}_total`
- `modelport_message_{input,output,cache_write,cache_read}_tokens_total`
- `modelport_message_cost_estimate_usd_total`

Message metrics are labelled by provider, model, and stream. Arbitrary model
passthrough can create high cardinality. Metrics reset on restart.

For streams, current duration and success account for upstream connection and
local stream acceptance, not guaranteed final completion. Final tokens/cost can
be estimated from the request rather than reconciled provider usage. Do not
build SLOs or invoices from those series without closing that lifecycle gap.
Request logs expose `billingMode` to distinguish Provider-returned usage from a
local estimate. Attempt-level preflight rows record zero usage; earlier ingress
failures may return before persistence. Neither updates quota/spend, though both
still incur local validation/metrics work.

## Tuning

```env
MODELPORT_MAX_CONCURRENT_REQUESTS=64
MODELPORT_MAX_CONCURRENT_STREAMS=64
MODELPORT_HTTP_CONNECT_TIMEOUT_SECS=10
MODELPORT_HTTP_REQUEST_TIMEOUT_SECS=600
MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS=300
MODELPORT_HTTP_MAX_RESPONSE_BYTES=33554432
MODELPORT_HTTP_SSE_MAX_LINE_BYTES=1048576
MODELPORT_HTTP_SSE_MAX_EVENT_BYTES=8388608
MODELPORT_HTTP_SSE_MAX_STREAM_BYTES=67108864
MODELPORT_USAGE_LOG_LIMIT=5000
```

- Lower concurrency before raising it when provider rate limits or storage
  latency are the bottleneck.
- `MODELPORT_MAX_CONCURRENT_STREAMS` defaults to the effective general
  concurrency cap. Size it for simultaneously open bodies, not request-start
  throughput: slow readers hold permits until completion/drop. A 429 includes
  `Retry-After: 1`, and raising the cap increases open sockets and upstream
  work.
- Keep request/response/SSE limits finite; larger values increase memory and
  connection exposure.
- Do not use `MODELPORT_HTTP_REQUEST_TIMEOUT_SECS` as a live-generation cap. It
  covers the full non-stream exchange but only the SSE handshake; tune the
  stream idle and byte limits for the established phase.
- Reduce usage retention when complete-document writes become visible.
- Prefer PostgreSQL over JSON files when control state changes frequently, while
  recognizing that the logical document is still replaced synchronously.
- Diagnose provider/network latency before changing gateway timeouts.
- Measure Tool Use and large-context workloads separately from tiny text calls.

Multi-instance rate limiting, transactional quota reservations, event-oriented
usage storage, and final live-stream accounting are not implemented. Those are
the scaling triggers for architectural work, not settings that can be tuned
away.
