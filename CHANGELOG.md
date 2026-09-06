# Changelog

All notable ModelPort changes are recorded here. The project follows
[Semantic Versioning](https://semver.org/) once a version is published.

## [Unreleased]

## [0.1.1] - 2026-09-06

### Release correction

- Scan the staged binary directory for the SPDX SBOM and keep all release
  uploads in the final publication job. The v0.1.0 attempt failed before
  publishing assets because the SBOM action received an archive as a directory.
- Pin release examples and images to v0.1.1; preserve the failed v0.1.0 tag.
- Honor the selected Compose deployment in backup, verification and restore
  drills; keep generated config readable by the non-root container user.
- Build clean local images from one immutable Git snapshot.

### Team setup and maintenance

- Generate minimal local configuration and independent credentials with
  `scripts/setup.sh`; preserve existing files on rerun.
- Honor deployment `MODELPORT_BIND` with TOML and allow external-database
  Compose rendering without an unused internal database password.
- Resume the four-step setup journey from saved configuration, require a
  completed request for success, and link logs to exact ledger evidence.
- Deduplicate session initialization to avoid detached loading queries; share
  clipboard fallback behavior and report rejected copies accurately.
- Load chart dependencies with the Dashboard instead of the login page, prune
  unused mock initialization, and isolate model adaptation and operations queries.
- Add audited project-policy forms with local/unknown defaults and selected-key
  client setup checks; stop treating the administrator catalog as a key's scope.
- Group the console into five task entry points while retaining page URLs and
  role checks. Prioritize team activation over GPU expansion per ADR-0008.
- Build the optional operations Agent locally only with `--with-ops-agent`.
- Remove unused date-fns and framer-motion dependencies; update React Router
  and Browserslist patches and retire the resolved router audit exception.
- Add a bounded, provider-neutral Runtime Adapter registry with validated
  origins, environment-only Bearer credentials, collection policy, and
  fail-closed startup loading.
- Add shared Dashboard Client/Harness setup profiles for Claude Code, Qwen
  Code, and the OpenAI SDK, while explicitly blocking Codex CLI until the
  Responses ingress exists.

### Release preparation

- Prepared `v0.1.1 Small-Team Beta` for a free, MIT-licensed, self-hosted
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

## [0.1.0] - Unpublished candidate

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

[Unreleased]: https://github.com/tiammomo/ModelPort/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/tiammomo/ModelPort/releases/tag/v0.1.1
[0.1.0]: https://github.com/tiammomo/ModelPort/tree/v0.1.0
