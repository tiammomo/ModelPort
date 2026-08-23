# ADR-0005: Forty-User Hybrid Routing Production Baseline

- Status: Accepted, staged implementation; local model, artifact, runtime, and
  GPU ownership superseded by ADR-0007
- Date: 2026-07-31

## Context

ModelPort is being prepared for a trusted organization of approximately forty
people. One local Qwen runtime on a single NVIDIA GPU cannot provide forty
simultaneous interactive generations, while sending every request to a hosted
Provider would violate privacy, cost, and operator-control goals.

The first production phase still has one ModelPort process. Active-active
gateway operation is a later availability milestone and must not be implied by
the current Compose deployment, OIDC session implementation, in-memory health,
or rate-limit state.

## Decision

### Product and deployment boundary

- ModelPort is the only client-facing gateway and policy decision point.
- The current phase deploys one ModelPort instance on Linux.
- Production state moves to an operator-managed high-availability PostgreSQL
  service. The PostgreSQL service in the root Compose file remains a local
  development and migration-drill dependency.
- ModelPort owns the desired state, inventory, policy, and evidence for local
  models, Runtime Adapters, Compute Nodes/GPUs, and Deployments. External
  inference runtimes own execution mechanics. The independent boundary and
  migration from the original `local-inference-stack` integration are defined
  by [ADR-0007](0007-independent-model-and-gpu-control-plane.md).
- A later phase may run two stateless ModelPort instances. That phase requires
  distributed sessions, limits, health, and failover evidence before it can be
  described as available.

### Routing and data policy

Four stable policy modes are defined for the target implementation:

- `local_strict`: only approved local Providers; never falls back to cloud.
- `local_first`: local by default, with policy-approved cloud overflow.
- `balanced`: selects among approved local and cloud candidates by capability,
  health, queue delay, and cost.
- `cloud_first`: cloud by default and restricted to administrators or an
  explicitly approved project policy.

Administrators set the most permissive mode for a project. A user may only
choose an equally restrictive or more restrictive mode. Unclassified requests,
repository content, customer data, and internal documents default to
`local_strict`. Only organization-reviewed Provider/model/API-version/region
combinations may enter a project allowlist; arbitrary compatible endpoints are
not user-configurable.

### Identity, fairness, and cost

- Humans use OIDC; automation uses independent, expiring, scoped service
  accounts. Shared human API keys are prohibited.
- Each user may have one request executing locally and two queued locally. The
  global interactive local queue is limited to sixteen requests.
- `local_first` and `balanced` requests overflow to an approved cloud Provider
  when predicted local wait exceeds five seconds.
- `local_strict` requests never leave the local boundary, wait at most sixty
  seconds, and then receive HTTP 429 with `Retry-After`.
- Batch traffic uses a separate low-priority queue.
- Project cloud budgets warn at 80 percent and become hard limits at 100
  percent. A time-bounded, audited break-glass grant is the only override.

These limits are target behavior. Until the scheduler and identity work is
shipped and accepted, operators must use current documented process-local
limits and must not claim per-user fairness.

### Privacy, tools, and retry consistency

- Prompt, response, tool argument, and tool result content is not persisted by
  default. Operational records contain identity, project, routing decision,
  model, Provider, usage, latency, cost, and bounded failure classification.
- Content diagnostics require explicit project-scoped approval, encryption,
  visible status, and automatic expiry no later than twenty-four hours.
- ModelPort validates and translates Tool Use but never executes arbitrary
  tools. Applications or a separately isolated tool runner own execution,
  approval, sandboxing, egress, and business credentials.
- Automatic retry or Provider fallback is permitted only before any response
  bytes or tool calls are emitted. A started stream and a Tool Use conversation
  remain pinned to the selected Provider/model attempt.

### Governance and reliability targets

- High-risk changes to data egress, Provider allowlists, `cloud_first`, hard
  budgets, identity permissions, or production model promotion require two
  approvers. Low-risk changes may use one administrator. Break-glass changes
  are time-bounded and immediately audited. This production target is enforced
  by enterprise mode or `MODELPORT_REQUIRE_DUAL_APPROVAL=1`; default Small-Team
  mode keeps the workflow optional and authorizes administrator writes through
  CSRF protection plus the audit trail.
- The target monthly SLO is 99.9 percent for the gateway and 99.0 percent for
  the single local inference capability.
- PostgreSQL targets RPO no greater than five minutes and RTO no greater than
  thirty minutes.
- Production secrets come from an external secret manager or workload
  identity. Repository `.env` files are development-only and are excluded from
  new backup archives.
- OpenTelemetry is the long-term telemetry boundary; the built-in dashboard is
  a lightweight user and administrator surface, not a metrics database.

## Staged delivery

1. Establish migration, secret-free backup, recovery, CI, and documentation
   evidence for the current single instance.
2. Add OIDC-backed project policy, service accounts, routing modes, and
   per-user fairness.
3. Add hybrid scheduling, budget enforcement, approved Provider governance,
   and circuit breakers.
4. Move the production database and secrets to managed dependencies and add
   standard telemetry. Validate a second ModelPort instance only after shared
   state is complete.
5. Roll out to five, then fifteen, then forty people. Add a local GPU node when
   cloud overflow exceeds 30 percent, `local_strict` 429 responses exceed 1
   percent, or local queue P95 exceeds five seconds for two consecutive weeks.

Every stage must distinguish implemented behavior from target behavior and
must pass its migration, recovery, privacy, and protocol acceptance gates.

## Consequences

- One ModelPort instance is an explicit availability limitation in the first
  phase, not an accidental claim of high availability.
- Hybrid routing is policy-controlled and auditable rather than a silent
  availability shortcut.
- Production database and secret lifecycle are external operational
  dependencies; the repository provides validation and runbooks rather than a
  replacement database or secret manager.
- Scaling local inference is horizontal by independently accepted nodes. No
  automatic LAN discovery or unvalidated multi-GPU aggregation is allowed.

## Rejected alternatives

- Forty people sharing one ModelPort API key: prevents identity, quota,
  revocation, fairness, and useful audit evidence.
- Always-cloud fallback: violates the default-local data boundary.
- Silent post-token fallback or parallel real-request shadowing: can duplicate
  side effects, cost, data exposure, and inconsistent Tool Use state.
- Claiming active-active support from two containers alone: current sessions,
  limits, health, and some control state are not yet distributed.
