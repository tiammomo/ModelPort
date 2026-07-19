# ModelPort Dashboard

The dashboard is ModelPort's browser control plane. It is built with React,
TypeScript, Vite, Tailwind CSS, local shadcn-style primitives, TanStack Query,
Table and Virtual, Recharts, Zustand, and Playwright.

It manages users, API keys, teams, quotas, providers, credential metadata,
models, aliases, health, request-usage records, audit events, and diagnostic
snapshot export. The snapshot is not restorable; complete backup/restore is a
CLI workflow. The dashboard does not store raw model prompts or provider
responses, and it is not a chat interface.

## Requirements

- Node.js 24 and npm (matching CI).
- A running ModelPort backend for real mode.
- Playwright Chromium and host dependencies for E2E tests.

Install reproducibly:

```bash
npm ci
```

## Development

Start the backend from the repository root:

```bash
cp .env.example .env
# replace required placeholders
scripts/start.sh
```

Then run the Vite server:

```bash
cd dashboard
npm run dev
```

The default URL is `http://127.0.0.1:33002`. Vite proxies `/admin`, `/v1`,
`/livez`, `/readyz`, `/health`, and `/metrics` to the backend on port 38082.
Set `MODELPORT_VITE_PROXY_TARGET` to another backend origin when isolating E2E
runs or developing against multiple local instances; the default is
`http://127.0.0.1:38082`.

Login uses `MODELPORT_ADMIN_USERNAME` and `MODELPORT_ADMIN_PASSWORD`. The router
token is for data-plane clients and metrics, not normal dashboard login.

## Same-Origin Requirement

The supported browser layout serves dashboard and backend routes from one
origin through Vite, the Docker Nginx image, or another reverse proxy.

`VITE_API_BASE_URL` changes the browser fetch origin at build time, but the
backend does not currently emit general CORS headers. Do not point it at a
different browser origin unless a trusted proxy implements and tests the full
CORS/credential policy. `MODELPORT_ALLOWED_ORIGINS` only affects dashboard write
validation; it is not a CORS switch.

## Mock Mode

```bash
VITE_MODELPORT_MOCK=1 npm run dev
```

Mock mode is for component/layout work. Mock values, synthetic trace panels, and
mock provider health are not evidence of backend behavior. Never ship a
production build with mock mode enabled.

## Runtime Truth In The UI

The maintained product and interaction contract is
[Dashboard Experience](../docs/DASHBOARD_UX.md). Keep page behavior, E2E tests,
and that contract in sync.

- The Settings page shows bind address, request-body limit, concurrency, auth,
  and timeout/rate values as read-only runtime facts. Change them in environment
  or TOML and restart the backend.
- Default provider and provider order are control-plane settings managed from
  model/provider controls (or the control API), not by editing read-only service
  fields on Settings.
- Provider/model/alias lifecycle changes are persisted as control-plane
  overrides; they do not rewrite `.env` or `config.toml`.
- Provider edits use an explicit clear contract for the credential environment
  name: a non-empty field sends `apiKeyEnv`; an empty field sends
  `clearApiKeyEnv: true`. Omitting both preserves the current backend value.
- API-key controls follow the session role. Administrators own creation,
  team/policy/status/expiry/spend settings, and restoration. Normal users can
  read owned keys, change only name/group, and revoke or delete them. Viewers
  have no write controls. The backend enforces the same boundary.
- The API-key field persisted as `rateLimited` is a compatibility name for
  periodic spend limits, not requests-per-minute throttling. When the switch is
  off, the dashboard disables the rolling USD fields and explains that the
  saved values are not enforced.
- Request-log detail panels are reconstructed summaries. Only fields returned
  by `/admin/logs`—including the persisted request ID—should be labelled as
  observed data. Do not label a usage-record UUID as a trace ID or claim a
  stored internal protocol IR.
- Log filters and pagination are delegated to `/admin/logs`; totals and summary
  cards come from the complete filtered-set response, and the drawer fetches
  `/admin/logs/{id}`. A row without `requestId` can be an aggregate metrics
  fallback rather than an individual persisted request.
- Dashboard trend charts and range cards are aggregated by
  `/admin/dashboard` over every retained row in the selected window, not the
  current logs page. The response identifies `persisted-usage`,
  `process-metrics-estimate`, or `empty`, flags estimates, and warns when the
  retention cap may have evicted older rows. Custom ranges are capped at 90
  days.
- Token and cost values are estimates. Log `billingMode` distinguishes
  `upstream-returned` token provenance from `local-estimate`; an attempt-level
  preflight row has zero usage, while earlier ingress failures may have no
  persisted row. Neither consumes quota/spend.
  Live-stream completion/final usage currently has additional limits documented
  in [Operations](../docs/OPERATIONS.md#request-logs).

## Checks

```bash
npm run check
npm run e2e
```

Install Playwright when needed:

```bash
npx playwright install --with-deps chromium
```

The suite covers login, dashboard ranges, model visibility, provider/model
lifecycle, effective settings/reload, and user/API-key workflows. Add or update
an E2E test whenever a user-visible workflow changes.

## UI Guidelines

- Prefer real controls and observed values over decorative or synthetic detail.
- Keep operational pages dense but readable; avoid nested-card noise.
- Handle long model names, request IDs, token counts, prices, and error text
  without breaking layout.
- Maintain keyboard access, focus visibility, labels, contrast, loading/error/
  empty states, and usable login/quick checks on mobile.
- Do not render secrets returned by create-key flows after the one intended
  reveal, and do not put sensitive values in URLs or browser logs.

## Production Build

```bash
npm run build
```

The Docker image serves `dist/` with Nginx and proxies backend routes on the same
origin. Its CSP, body limit, proxy timeouts, and backend response/stream limits
should be reviewed together when changing payload behavior.
