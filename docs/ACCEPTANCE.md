# Production Acceptance

This checklist validates the supported single-host/personal-or-small-team
profile. It does not prove public multi-tenant safety, all-provider health,
exact billing, or crash recovery for an in-flight live-stream lifecycle.

## Prerequisites

- A running backend and configured `.env`.
- `curl` and Node.js.
- Admin username/password for control-plane checks.
- Dashboard reachable when dashboard acceptance is required.
- A complete backup before running against important existing state.

The scripts create temporary users, teams, API keys, provider records, audit
events, and backup files, then clean temporary control records. Run them only on
an instance where those changes are acceptable.

## Local Control-Plane Acceptance

```bash
scripts/acceptance.sh
```

Default mode does not call an upstream model. It checks:

- `/livez` and authenticated `/readyz` diagnostics;
- optional dashboard reachability and admin login;
- authenticated `/v1/models`;
- temporary user, team, and API-key creation;
- API-key IP rejection and tiny spend-limit rejection before upstream routing;
- audit-event creation;
- complete CLI backup export and deep auth/control validation;
- cleanup of temporary records.

The Rust black-box suite additionally verifies OpenAI Chat Completions bearer
authentication, non-stream passthrough, OpenAI SSE plus final usage chunks,
terminal usage reconciliation, OpenAI-to-Anthropic conversion, and explicit
rejection of unsupported fields. These are fixture-backed contract checks, not
real-Provider certification.

`readyz` returning success means auth/control storage and the normalized
request/attempt ledger were reachable and detailed diagnostics were accessible;
it does not prove that every Provider is ready.

## Tool Use Adapter Acceptance

```bash
scripts/tool-use-acceptance.sh
```

Default mode starts a temporary local OpenAI-compatible mock, creates a
temporary provider through the dashboard API, and validates:

- non-stream Tool Use response mapping;
- streaming `input_json_delta` mapping;
- `tool_result` continuation to OpenAI `role=tool`;
- malformed Tool Use rejection before upstream;
- `disable_parallel_tool_use` mapping;
- cleanup of the provider and mock process.

This proves the adapter against the fixture, not a real provider.

## Paid Upstream Acceptance

```bash
scripts/acceptance.sh --upstream
scripts/tool-use-acceptance.sh --upstream
scripts/provider-matrix.sh --model provider:model
```

These commands consume provider quota and depend on account entitlement, model
availability, and upstream status. Provider matrix stream acceptance inspects
text deltas and `event: error`; record the date/commit/result in
[Provider Matrix](PROVIDER_MATRIX.md).

An upstream acceptance pass does not close these known limits:

- ordinary live-stream terminal state is reconciled in-process, but final
  Provider usage may remain estimated and post-header fallback is impossible;
  expired durable leases are terminalized as unbilled `unreconciled` evidence,
  not reconstructed Provider usage;
- concurrent quota checks are not transactional reservations;
- provider hostname DNS answers are not private-range revalidated;
- persisted control state is synchronously written as a complete document.

## Code And Dashboard Checks

Use the aggregate repository check when available:

```bash
scripts/check-all.sh
```

Or run the layers explicitly:

```bash
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets --all-features -- -D warnings

cd dashboard
npm ci
npm run check
npm run e2e
```

Playwright requires Chromium and host libraries. Install them using the
supported Playwright command for the machine, such as:

```bash
npx playwright install --with-deps chromium
```

## Manual Product Checks

- Login: correct/incorrect/locked credentials, mobile layout, focus and error
  text.
- Dashboard: ranges, long numbers/names, empty/error/loading states.
- Dashboard navigation: role-filtered sidebar and command palette, explicit
  access denied pages, return-to-source login, and query cache isolation after
  changing users.
- Dashboard ranges: server totals cover all retained rows rather than the
  visible log page; source/estimated/retention-limit labels match the response.
- Logs: real request ID, debounced and shareable URL filters, local-time
  shortcuts, automatic refresh without a fixed end time, mobile request cards, accessible detail
  drawer, stream-estimate labels; `upstream-returned` versus `local-estimate`
  provenance; no claim of stored raw request/provider IR or per-request Tool Use
  stages that were not persisted.
- Models/providers: create/update/disable/delete dependencies, credential status,
  `clearApiKeyEnv`, automatic-pool fail-closed behavior, discovery and a real
  model test when intended.
- Settings: service-level runtime fields are read-only; default provider/order
  save correctly; reload reports accurate restart scope.
- API keys/users/teams/quotas: ownership and role boundaries, one-time key
  reveal acknowledgement, preview-versus-secret wording, self/last-admin
  protection, owner-active checks, separate quota units, explicit zero-limit
  blocking confirmation, user UTC calendar quota resets versus rolling spend
  windows, referenced-team deletion rejection, expiration and cleanup.
- Messages/billing: missing, zero, and oversized `max_tokens` rejection;
  canonical API-key/quota usernames; locally rejected requests consuming zero
  quota/spend; correct `billingMode` provenance after a real attempt.
- Chat Completions: common OpenAI SDK text request, developer/system roles,
  function tools, bearer authentication, `chat.completion` response shape,
  standard chunk stream, optional final usage chunk, and cross-protocol routing.
- Proxy/SSE: right-to-left trusted-hop extraction, Host port preservation,
  strict `text/event-stream` handshake, handshake/error-body/idle timeouts, and
  missing termination events becoming SSE errors and terminal request/provider
  evidence; downstream cancellation becoming 499 without penalizing an already
  completed upstream; concurrent-stream exhaustion returns 429 and a permit
  remains held until body completion/drop.
- Operations: diagnostic export versus complete CLI backup is clearly labelled.

## Release Evidence

Record:

- commit/build and deployment mode;
- every command run and whether it used a paid provider;
- configured provider/model and endpoint ownership;
- pass/fail plus any SSE error body (redacted);
- storage backend and backup validation;
- known limitations accepted for this rollout.

A default acceptance pass supports a controlled trial of the local control
plane. A dated real-provider result supports only the exact provider/model/path
that was tested.
