# ModelPort Project Guide

This is the maintainer-facing product and engineering guide. Normative runtime
behavior lives in [Architecture](ARCHITECTURE.md),
[Configuration](CONFIGURATION.md), [API](API.md), and
[Operations](OPERATIONS.md).

## Positioning

ModelPort is a self-hosted Anthropic-compatible routing and adaptation layer for
Claude Code, VS Code Claude, and API clients, with a lightweight control plane
for one host or a small team.

It should remain:

- easier to deploy and understand than an enterprise AI platform;
- explicit about protocol loss, provider verification, and operational limits;
- safe by default on loopback and usable behind one trusted HTTPS origin;
- observable enough to diagnose routing and provider failures without requiring
  a distributed operations stack.

It is not a chat product, inference engine, public aggregation service, exact
billing system, enterprise IAM product, or multi-tenant SaaS control plane.

## Current Product Line

Implemented:

- Anthropic-compatible Messages/model endpoints;
- Anthropic and OpenAI-compatible provider paths;
- common Tool Use validation/conversion and SSE mapping;
- model/provider routing, aliases, control-plane overrides, credential pools,
  cooldown and bounded fallback;
- users, API keys, teams, quotas, provider/model management, logs, health,
  audit, redacted diagnostic snapshots, PostgreSQL/JSON state;
- Docker Compose, systemd, scripts, CI, dashboard and E2E tests.

Configured but not yet evidenced by a committed real-upstream ledger:

- every provider/model entry in the built-in catalog;
- provider-specific Tool Use and streaming behavior;
- pricing estimates beyond regression-tested local tables.

Proposed, not implemented:

- Image/Responses APIs;
- a complete internal protocol/Tool IR;
- OIDC/SSO and public multi-tenancy;
- distributed rate limits, sessions, quotas, and health;
- transactional/event-oriented usage persistence.

## Engineering Priorities

1. Close correctness gaps in live-stream completion, final usage/cost, provider
   outcomes, and errors after headers.
2. Make quota enforcement concurrency-safe before treating it as a hard budget.
3. Replace hostname-only SSRF checks with DNS-aware outbound policy where the
   deployment threat model requires it.
4. Reduce complete-document persistence and define retention/migration behavior.
5. Maintain a dated provider verification ledger and test examples as code.
6. Only then expand protocols or distributed deployment scope.

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
