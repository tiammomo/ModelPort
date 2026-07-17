# ModelPort Project Guide

This is the maintainer-facing product and engineering guide. Normative runtime
behavior lives in [Architecture](ARCHITECTURE.md),
[Configuration](CONFIGURATION.md), [API](API.md), and
[Operations](OPERATIONS.md).

## Positioning

ModelPort's target product is a self-hosted enterprise model gateway: a
multi-protocol data plane and governed multi-tenant control plane for enterprise
applications, developer tools, hosted model Providers, and private inference.

The current implementation remains a single-host and small-team gateway. It
must not be marketed as enterprise ready until the admission criteria in the
[Enterprise Gateway Roadmap](ENTERPRISE_ROADMAP.md) have implementation and
release evidence.

It should evolve toward:

- Anthropic Messages, OpenAI Chat Completions, and OpenAI Responses client
  contracts sharing one governance pipeline;
- organization/project tenancy, enterprise identity, transactional budgets,
  auditable usage, and resource-level policy;
- horizontally scalable data-plane replicas with explicit consistency and
  dependency-degradation behavior;
- explicit about protocol loss, provider verification, and operational limits;
- secure-by-default egress, secret references, data governance, and
  OpenTelemetry-based observability;
- an all-in-one development profile and production roles that can scale
  independently when required.

It is not a chat product, inference engine, training platform, payment
processor, identity provider, or a substitute for a Provider's legal invoice.

## Current Product Line

Implemented:

- Anthropic-compatible Messages, scoped OpenAI Chat Completions, and model
  endpoints through one governance pipeline;
- an initial typed Exchange IR for text roles, function tools, provider
  capability/fidelity checks, normalized usage, and terminal stream evidence;
- Anthropic and OpenAI-compatible provider paths;
- common Tool Use validation/conversion and SSE mapping;
- model/provider routing, aliases, control-plane overrides, credential pools,
  cooldown and bounded fallback;
- users, API keys, teams, quotas, provider/model management, logs, health,
  audit, redacted diagnostic snapshots, PostgreSQL/JSON state;
- an optional single-host [OIDC console sign-in preview](OIDC.md) using
  Authorization Code flow, PKCE, durable issuer/subject identity bindings, and
  the same first-party console session and local roles as password login;
- Docker Compose, systemd, scripts, CI, dashboard and E2E tests.

The OIDC preview authenticates humans to the ModelPort console only. It is not
a complete enterprise IAM plane: sessions and authorization state remain
process-local, and service accounts, SCIM, organization lifecycle, distributed
session coordination, and resource-level RBAC are not implemented. Data-plane
clients still require a ModelPort API key.

Configured but not yet evidenced by a committed real-upstream ledger:

- every provider/model entry in the built-in catalog;
- provider-specific Tool Use and streaming behavior;
- pricing estimates beyond regression-tested local tables.

Proposed, not implemented:

- the OpenAI Responses API and a complete multimodal/item-oriented Exchange IR;
- organization/project tenancy, service accounts, SCIM, enterprise identity
  lifecycle, and resource-level RBAC;
- distributed rate limits, sessions, quotas, and health;
- transactional/event-oriented usage persistence;
- high-availability deployment roles, Redis coordination, OTLP export, secret
  manager integrations, and enterprise release evidence.

## Engineering Priorities

1. Build on the shipped request/attempt leases and expired-row reconciler with
   Provider evidence ingestion, response replay, and append-only settlement
   adjustments.
2. Continue implementing the accepted tenant, typed protocol exchange, relational
   persistence, consistency, and deployable-role ADR boundaries.
3. Replace complete-document persistence with normalized, transactional,
   tenant-scoped PostgreSQL repositories and versioned migrations.
4. Expand Chat Completions conformance and add Responses client ingress through
   the shared adapter and capability/fidelity contract.
5. Add enterprise identity, resource-level policy, secret management, atomic
   budgets, and distributed runtime enforcement.
6. Prove horizontal availability, observability, recovery, governance, and
   security through the gates in the
   [Enterprise Gateway Roadmap](ENTERPRISE_ROADMAP.md).

## Decision Principles

- Prefer an Anthropic-compatible or OpenAI-compatible adapter over a provider-
  native API until native behavior has clear product value.
- Do not silently lose Anthropic semantics in strict mode.
- Do not call configuration support “verified”.
- Do not infer exact spend, latency, or stream success from an estimate or the
  initial HTTP status.
- Keep secrets in process/file/secret-manager inputs, not control-plane records.
- Make runtime overrides visible and distinguish them from base configuration.
- Add a deployment dependency only when it solves a measured problem for the
  intended audience.

## Repository Map

```text
src/                 Rust gateway and CLI
src/lib.rs            library orchestration; CLI/server dispatch
src/cli.rs            config validation and backup commands
src/server.rs         server state, listener and shutdown
src/routes/          client, operations, and control-plane route modules
src/providers/       Anthropic/OpenAI-compatible adapters
dashboard/           React control plane and Playwright tests
scripts/             local lifecycle, checks, smoke, acceptance, benchmark
deploy/docker/       Compose environment, Nginx and Caddy examples
deploy/systemd/      hardened backend unit and environment template
docs/                maintained reference and non-normative learning material
```

See the exact module responsibilities in
[Architecture](ARCHITECTURE.md#backend-boundaries).

## Release Evidence

A release should include:

- change summary and configuration/migration notes;
- the exact commit and build/test commands;
- Docker and systemd smoke results when affected;
- dated provider/model results only for real calls that were actually run;
- known stream, quota, DNS, persistence, and cost-estimation limits;
- a validated complete backup before persistence changes.

The provider acceptance standard is defined in
[Provider Matrix](PROVIDER_MATRIX.md#verification-procedure), and the broader
production checklist in [Acceptance](ACCEPTANCE.md).

## Documentation Ownership

- Root README: short, verifiable user entry.
- `docs/ARCHITECTURE.md`: implementation boundaries and limitations.
- `docs/CONFIGURATION.md`: one configuration reference.
- `docs/API.md`: public contract and control-plane route groups.
- `docs/OPERATIONS.md`: runtime truth, logs, metrics, backup, troubleshooting.
- `docs/PROVIDER_MATRIX.md`: static catalog plus dated real verification.
- `docs/learning/`: non-normative explanation derived from the sources above.
- Proposed work: clearly labelled proposal/RFC, never presented as shipped.

When implementation and docs disagree, fix both in the same change and add a
check that would catch the drift again.
