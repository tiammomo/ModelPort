# Changelog

## Unreleased

- Add a bounded, provider-neutral Runtime Adapter registry with validated
  origins, environment-only Bearer credentials, collection policy, and
  fail-closed startup loading.
- Add shared Dashboard Client/Harness setup profiles for Claude Code, Qwen
  Code, and the OpenAI SDK, while explicitly blocking Codex CLI until the
  Responses ingress exists.
- Compose the HTTP application from domain-owned routers and verify one
  complete method/path/domain inventory without changing the public API.

All notable ModelPort changes are recorded here. The project follows
[Semantic Versioning](https://semver.org/) once a version is published.

## [Unreleased]

### Release preparation

- Prepared `v0.1.0 Small-Team Beta` for a free, MIT-licensed, self-hosted
  20–50 person internal development team; this changelog does not claim the tag,
  GHCR images, signatures, or GitHub Release exist before the release workflow
  succeeds.
- Added a versioned prebuilt-image Compose profile, Linux x86_64 compatibility
  matrix, signed-image/digest/SBOM release contract, safe upgrade/rollback
  guide, 30/90/395-day retention controls, independent Dashboard failure mode,
  and official Prometheus/Grafana/runbook package.
- Set a 6–8 week productization freeze: no new protocol, Provider, HA,
  Kubernetes, hosted, paid, or public-multi-tenant surface except to resolve a
  security/data-loss/release blocker or a verified design-partner blocker.

## [0.1.0] - Pending publication

### Added

- Relational PostgreSQL request, Provider-attempt, usage, quota/spend, budget,
  management-statistics, and append-only audit sources.
- Complete request identity, client, traffic, Tool Use, pricing provenance,
  retry/fallback, latency, and TTFT dimensions.
- Authenticated operational views, build identity, Provider evidence output,
  and rejection metrics.
- Opt-in smart-routing groups with policy/capability gates, quality/balanced/
  economy/latency profiles, shadow decisions, stable canary activation,
  session affinity, metrics, and relational decision evidence.
- Separate `cpa_codex` and `cpa_claude` internal Provider templates with
  closed model allowlists, CPA catalog discovery, and internal-HTTP URL policy.
- Free open-source governance, support, privacy, release, and supply-chain
  policies.

### Changed

- Documentation now uses a role-based index and one verified Getting Started
  path; overlapping planning, acceptance, Provider, performance, lifecycle, and
  learning documents were consolidated into maintained references.
- PostgreSQL is mandatory for every runtime deployment.
- The default Compose and CI database is PostgreSQL 18.4, using the PostgreSQL
  18 versioned data directory and a new `modelport-postgres-18` named volume.
- The dashboard runtime uses the current Nginx 1.30.4 stable security release.
- Dashboard, logs, quotas, audit, and management statistics use relational
  operational rows instead of process estimates or control-document arrays.
- Request-log SQL keeps enterprise pagination and operational time-window
  parameter contracts distinct, and minute-precision end times include the
  complete selected minute so current failures remain visible.
- The public model catalog advertises Provider-qualified IDs and explicit
  aliases.

### Removed

- Runtime JSON-file and process-memory persistence fallbacks.
- Automatic import of old JSON state.
- Old usage/activity/spend arrays and legacy management response aliases.
- The no-PostgreSQL Compose override.

### Security

- Configuration fails before binding when PostgreSQL is missing.
- Operational audit records are append-only and durable error details remain
  category-only.

### Upgrade notice

Migration `0005_current_operational_schema.sql` now preserves existing
normalized request/attempt rows, backfills conservative operational defaults,
and derives request-level Provider/retry snapshots from historical attempts.
Back up PostgreSQL and run a restore drill before upgrading. Compose still uses
the PostgreSQL 18 volume `modelport_modelport-postgres-18`; export any older
volume before removing it.

[Unreleased]: https://github.com/tiammomo/ModelPort/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tiammomo/ModelPort/releases/tag/v0.1.0
