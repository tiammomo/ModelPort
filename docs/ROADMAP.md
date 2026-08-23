# Roadmap

Status: accepted Small-Team Beta release contract with an approved
post-Beta control-plane direction.

Last reviewed: 2026-08-23.

## Product Contract

ModelPort is free, MIT-licensed, self-hosted software for a 20–50 person
Chinese internal development team that uses local models and approved cloud
Providers. The platform administrator is the primary operator; developers get
scoped keys, stable logical models, their own request evidence, and copyable
client configuration.

The core user outcome is:

> Sensitive code stays local by default. A cloud route is used only when an
> administrator-approved project policy permits it, and every egress decision
> has an identity, reason, route, outcome, and cost provenance.

Success is the first governed request within 30 minutes and sustained weekly
team use without policy bypass—not Provider count, raw request volume, GitHub
stars, or revenue. ModelPort has no paid edition, hosted service, or feature
tier.

This is the current release contract, not the ceiling of the product. The
approved long-term direction is an independent hybrid model and GPU control
plane that manages hosted API Providers and replaceable local Runtime Adapters
through one governed resource model. The current local Qwen path is a reference
adapter, not a dependency on another product repository. See
[ADR-0007](adr/0007-independent-model-and-gpu-control-plane.md).

## Control-Plane Delivery Sequence

Control-plane work is staged so the shipped gateway remains useful and honest
at every step:

1. **Architecture contract:** define Client/Harness, Provider, Model, Runtime
   Adapter, Compute Node/GPU, Deployment, and Route ownership; align the eight
   backend and Dashboard domains without claiming unimplemented APIs.
2. **Independent adapter boundary:** the versioned, read-only Runtime Adapter
   capability and Compute Node/GPU response schemas, validators, and Qwen
   fixtures are shipped. Authenticated transport and persisted inventory remain
   separate work; hosted Providers are unchanged.
3. **Compute inventory:** persist authenticated, read-only Compute Node/GPU
   observations with freshness, provenance, stable identifiers, and explicit
   unavailable/stale states. Inventory must not start a runtime or download a
   model.
4. **Deployment lifecycle:** add desired and observed Deployment state with
   idempotent reconciliation, bounded mutation, rollback evidence, and a
   manual placement choice.
5. **Policy-bounded placement:** consider automated placement only after
   inventory and lifecycle behavior have concurrency, recovery, and design-
   partner evidence. Scheduling must never bypass egress, model, budget, or
   tenant policy.

These are sequential product boundaries, not one large implementation PR.
Hosted-only installations remain supported throughout.

The next focused Issues after the adapter wire contracts are:

- collect authenticated Compute Node/GPU observations, persist the latest
  accepted snapshots, and expose an admin read API with server-derived
  freshness;
- introduce a Deployment resource and manual lifecycle reconciliation;
- realign Dashboard navigation to Models, Providers, Compute, Deployments,
  Routing, Governance, Observability, and Operations without duplicating
  backend state.

## v0.1.x Small-Team Beta Freeze

For the first 6–8 weeks after v0.1.0, new protocol, Provider, and platform
breadth is frozen. A change may break the freeze only when it fixes a security
issue, data-loss risk, release/upgrade blocker, or a reproducible blocker found
by a design-partner team.

Work during the freeze is ordered as follows:

1. **Activation:** prebuilt signed images, digest/SBOM evidence, state-driven
   onboarding, credential resolution/test state, stable logical models, and a
   first governed request in at most 30 minutes.
2. **Developer self-service:** own scoped/expiring keys, readable model catalog,
   copyable Claude Code/SDK configuration, own request logs, and explicit local
   versus cloud route evidence.
3. **Privacy and policy:** zero maintainer telemetry, no prompt/response/tool
   content persistence, owner-scoped logs, 30/90/395-day retention preview and
   apply, legal hold, `local_strict` default, and no silent Tool Use downgrade.
4. **Operations:** independent static Dashboard, liveness/readiness separation,
   graceful drain and ledger finalization, safe maintenance upgrade/rollback,
   official Prometheus rules, Grafana dashboard, and alert runbook.
5. **Validation:** two or three real teams, each with an administrator and at
   least five active developers for two weeks, providing only previewed,
   content-free diagnostic evidence.

Beta exit evidence:

- at least 80% of clean Tier 1 installs complete a governed request in 30
  minutes;
- week two active-developer coverage is at least 60%, and week four at least
  80%, using the locally calculated definition in the product plan;
- zero unapproved cloud egress and zero cross-user request-log access;
- every request exposes a stable request ID, logical model, actual route, and
  egress policy basis;
- clean install, upgrade, safe stop, backup, restore, and rollback acceptance
  passes for Linux x86_64;
- no open P0/P1 security, privacy, ledger, or activation blocker.

## Explicitly Deferred During The Freeze

- OpenAI Responses, realtime, embeddings, image/audio/multimodal APIs, and new
  public protocol surfaces.
- Provider breadth that is not required to unblock a design partner's existing
  approved route.
- Kubernetes, multiple active replicas, active-active/high availability,
  distributed limits/sessions/stream permits, or zero-downtime upgrade claims.
- Public multi-tenancy, a hosted service, payment/licensing systems, paid
  features, or an “enterprise ready” label.
- Online learning that directly changes production routing weights, developer-
  exposed router tuning, or silent capability downgrade.
- Dashboard storage of plaintext Provider secrets, full English UI
  internationalization, a chat workspace, and maintainer-operated telemetry.
- Automatic GPU placement, model downloads, runtime mutation, and multi-node
  scheduling before the adapter, inventory, reconciliation, and rollback
  contracts above are accepted.

Experimental compatibility work may continue behind explicit flags, but it
cannot enter the default route or support matrix without the evidence required
by [Compatibility](COMPATIBILITY.md).

## After Beta Evidence

Only measured design-partner needs can reorder these candidates:

- append-only Provider invoice reconciliation and bounded remaining-budget
  metrics;
- PostgreSQL pool, active-stream, and per-Provider latency telemetry;
- authenticated Compute Node/GPU inventory and Deployment lifecycle slices
  that follow ADR-0007;
- resource-level policy and additional identity/secret-manager integrations;
- a typed protocol extension whose fidelity and Tool Use contract fails closed;
- arm64 promotion after equivalent install/upgrade/restore evidence.

Multiple replicas, public tenants, or a new platform target require a separate
ADR and cannot be inferred from Docker build success.

## Stable Release Gate

A stable (`v1.0.0`) claim requires all Beta exit evidence plus at least two
named maintainers with repository release and private security-response access.
Both must complete a release rehearsal and security handoff. Until that gate is
met, the project remains Beta and makes no response-time or availability SLA.

## Decision Rule

Privacy, fail-closed policy, migration safety, explainable evidence, and the
measured dominant workload outrank feature breadth. A proposal that expands the
support surface must name the design-partner blocker, operating cost, rollback,
test evidence, and what existing frozen work it displaces.
