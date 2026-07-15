# Dashboard Experience Contract

This document defines the user-facing contract for the ModelPort control plane.
It complements the API and operations documents; it does not redefine backend
behavior.

## Product Goal

The dashboard should help an operator answer three questions quickly:

1. Can clients route a request now?
2. If a request failed, where should I investigate next?
3. Are access, quota, or estimated-cost controls approaching risk?

The preferred first-run path is:

1. Add an upstream Provider and reference its credential environment variable.
2. Check configuration readiness and test the connection when a real credential
   is available.
3. Discover or enter models and choose the default route or a stable alias.
4. Create the user or project boundary that owns client access.
5. Issue a client API key and save the one-time secret.
6. Send the first request using a copied endpoint, model, and key.
7. Open the matching request log to verify routing, latency, token provenance,
   and estimated cost.

The dashboard may guide this sequence, but it must not claim that configuration
readiness proves a paid Provider request succeeded.

## Navigation And Roles

The primary navigation is grouped by operator intent:

| Group | Pages | Main question |
| --- | --- | --- |
| Run | Dashboard, Request Logs | Is the gateway working and what changed? |
| Connect | Models and Providers, API Keys | Where does traffic route and how do clients connect? |
| Govern | Users, Quotas | Who can call which resources and under what limit? |
| System | Runtime Settings and Operations | Which deployment facts are active and what can be diagnosed safely? |

Navigation, quick search, and direct routes follow the same role policy. An
administrator sees management actions. A normal user sees only owned workflows.
A viewer receives read-only views. A denied direct route displays an explicit
access-denied state rather than silently redirecting to an unrelated page.

Authentication returns the user to the protected page they originally opened.
Changing the authenticated principal clears all client-side query data so cached
records cannot cross account boundaries.

## Data Truth And Provenance

Every operational conclusion must come from an identified source:

- **Observed**: persisted or returned directly by a backend endpoint.
- **Estimated**: calculated locally or by process metrics; show the estimate
  label and never present it as a Provider invoice.
- **Configuration-derived**: inferred from saved configuration; describe it as
  readiness or attribution, not proof of a successful upstream call.
- **Not recorded**: render `—` or `Not recorded`. Do not substitute a plausible
  default.

Request-log detail is a reconstructed summary. It must not claim that an auth,
policy, routing, adapter, Tool Use, or streaming stage ran unless the backend
persisted that stage. Protocol capability references are explicitly separated
from per-request evidence.

Provider status is intentionally split into configuration readiness, credential
availability, runtime health, and default-route selection. `GET /readyz` reports
storage/control readiness and diagnostics; it does not prove every Provider is
healthy.

## Page Contracts

### Dashboard

- Keep the primary row limited to range-consistent requests, success, estimated
  cost, and clearly labelled process latency.
- Display data range, source, freshness, estimate status, and retention warnings.
- Invalid custom ranges show an inline error; they do not silently masquerade as
  the selected range.
- Zero requests produce an unknown success rate and an actionable first-run
  guide, not a synthetic 100% success rate.
- Empty charts show a useful next action. Provider and error summaries link to
  the page where the operator can act.

### Models And Providers

- Separate model routes, Provider/credential setup, protocol capabilities,
  aliases, default routing, and restart-required templates.
- Configuration readiness never claims runtime availability. Explain the next
  action for missing credentials, disabled models, cooldown, or account issues.
- Destructive actions show dependencies and consequences before confirmation.
- Forms start with identity, endpoint, credential reference, and models; protocol
  tuning remains secondary and clearly describes its effect.

### Client API Keys

- Call the issued secret a **client API key** and upstream references
  **Provider credentials**.
- The full client secret appears once. The user must explicitly acknowledge that
  it was saved before closing the reveal step.
- The one-time reveal provides ready-to-copy Claude Code/Anthropic client
  environment settings using the real dashboard origin. A model remains an
  explicit choice from `/v1/models`; the UI does not invent one. OpenAI SDK
  clients may use `/v1/chat/completions`; the UI must describe its documented
  compatibility slice rather than imply complete OpenAI API parity.
- Prefixes and previews are labelled as identifiers and are never offered as a
  usable secret.
- List every active rolling spend window and its source. Do not invent usage
  percentages when the backend does not return usage.

### Users And Quotas

- Search and filter users by identity, role, and status. Role descriptions state
  actual capabilities.
- Prevent self-lockout and removal of the final active administrator in the UI,
  while retaining backend enforcement.
- Quota summaries never add requests, tokens, and USD into one number.
- A zero quota is an explicit blocking rule with confirmation. Positive limits
  are validated, and UTC calendar boundaries are shown.

### Request Logs

- Text filters are actually debounced. Time shortcuts write local
  `datetime-local` values without a UTC offset shift.
- Filters, page, and page size are encoded in the URL. Time bounds use epoch
  milliseconds so a copied troubleshooting link preserves the same instant in
  another timezone.
- Automatic refresh removes a fixed end time, refreshes only while the tab is
  visible, and displays freshness and stale-data states.
- Desktop uses a compact diagnostic table; mobile uses request summary cards.
- The detail drawer is an accessible modal with focus entry, focus trapping,
  Escape close, and focus restoration.
- Persisted fields, reconstructed summaries, and general protocol capability
  references are visibly distinct.

### Runtime Settings And Operations

- Runtime settings are read-only deployment facts unless an endpoint genuinely
  supports a dynamic change.
- Missing backend values stay missing; the UI does not create a loopback address
  or another operational default.
- Provider tests, reload, and diagnostic export show scope, side effects, and
  privacy impact. Diagnostic snapshots are not labelled as restorable backups.
- Audit request failures are errors, not empty history.

## Shared State Contract

Every data-backed page distinguishes:

| State | Required behavior |
| --- | --- |
| Initial loading | Stable skeleton or loading surface without false zero values |
| Fresh data | Show source and useful actions |
| Background refresh | Preserve current data and expose refresh activity |
| Refresh failure with data | Keep stale data, label its timestamp, and offer retry |
| Initial failure | Explain the failed resource and offer retry |
| True empty result | Explain why it can be empty and provide the next safe action |
| Access denied | State the required role without leaking protected data |

## Responsive And Accessible Behavior

- The main experience must work at 390 CSS pixels without page-level horizontal
  scrolling. Wide operational tables switch to cards or deliberately contained
  horizontal regions.
- Every icon-only control has an accessible name. Form labels are associated with
  controls, validation is announced, and keyboard focus remains visible.
- Menus, dialogs, drawers, tabs, and virtual rows support keyboard operation.
- Reduced-motion preferences disable non-essential animation.
- Mobile navigation is modal, closes with Escape or route selection, and does not
  leave focus behind an overlay.

## Verification

For every user-visible workflow change, run:

```bash
cd dashboard
npm run check
npm run e2e
```

Also inspect logged-in desktop and 390 px mobile views for every primary page.
Verify there is no page-level horizontal overflow, no console/page error, and
that loading, failure, empty, populated, and permission states remain truthful.
