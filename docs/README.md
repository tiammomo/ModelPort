# ModelPort Documentation

The root [README](../README.md) is the product entry point. You do not need to
read every document: choose the path that matches your task.

## I Want To Run ModelPort

1. [Learning Path](LEARNING_PATH.md) — choose a short operator, integration,
   operations, or contributor track. A
   [Chinese edition](LEARNING_PATH.zh-CN.md) is also maintained.
2. [Getting Started](GETTING_STARTED.md) — go from a clean clone to the first
   authenticated request.
3. [Local Qwen reference adapter](LOCAL_INFERENCE_STACK.md) — optionally
   reproduce the original Linux/WSL2 integration without treating the external
   repository as a ModelPort dependency.
4. [Deployment](DEPLOYMENT.md) — choose Docker Compose or systemd.
5. [Production](PRODUCTION.md) — harden and accept a shared deployment.
6. [Operations](OPERATIONS.md) — monitor, back up, troubleshoot, and upgrade it.
7. [PostgreSQL migration](POSTGRESQL_MIGRATION.md) — rehearse and cut over a
   major-version or managed-database migration without reusing an old data
   directory.
8. [40 人团队第一阶段生产基线](PRODUCTION_BASELINE_40_USERS.zh-CN.md) — 单
   ModelPort 现状下的数据库、备份、密钥和投产检查清单。
9. [Compatibility matrix](COMPATIBILITY.md) — Tier 1 platform and explicit
   experimental/unsupported boundaries.
10. [Observability runbook](OBSERVABILITY_RUNBOOK.md) — official alerts,
    triage, and recovery actions.
11. [Upgrading and rollback](UPGRADING.md) — safe stop, migration acceptance,
    and paired application/database rollback.

## I Want To Connect An Application

- [API](API.md) — authentication, Messages, Chat Completions, streaming,
  errors, request IDs, and control-plane endpoints.
- [Providers](PROVIDERS.md) — choose a hosted Provider or local runtime and
  verify its exact model path.
- [Runtime Adapter](RUNTIME_ADAPTER.md) — validate the versioned, read-only
  capability and Compute Node/GPU observation boundaries for an external
  runtime.
- [Tool Use](TOOL_USE_COMPATIBILITY.md) — request/response conversion,
  validation, streaming, and Provider-specific acceptance.
- [Smart Routing](SMART_ROUTING.md) — define smart aliases and roll them out
  through shadow and canary modes.

## I Want To Configure ModelPort

- [Configuration](CONFIGURATION.md) is the single reference for environment
  variables, TOML, precedence, security guardrails, and reload scope.
- [OIDC](OIDC.md) covers optional dashboard sign-in and identity linking.
- [Docker Compose](DOCKER.md) and [systemd](SYSTEMD.md) contain
  deployment-specific details.

## I Want To Contribute

- [Development](DEVELOPMENT.md) — toolchain, local workflow, tests, and
  change-to-test matrix.
- [Architecture](ARCHITECTURE.md) — component boundaries, request lifecycle,
  persistence, trust boundaries, and deliberate non-goals.
- [Dashboard README](../dashboard/README.md) — UI behavior and contribution
  contract.
- [ADRs](adr/README.md) — accepted architecture decisions.
- [Releasing](RELEASING.md) and
  [maintainer policy](../.github/MAINTAINERS.md) — release and repository
  administration.

## Project Direction

[Roadmap](ROADMAP.md) defines the Small-Team Beta productization freeze and the
staged control-plane direction. [ADR-0007](adr/0007-independent-model-and-gpu-control-plane.md)
defines independent model, Runtime Adapter, Compute Node/GPU, and Deployment
ownership. Deferred work must not be described as shipped.

## Documentation Rules

1. Each fact has one authoritative document; other pages link to it.
2. Distinguish implemented behavior, configured support, real-upstream
   verification, and proposed work.
3. Keep secrets, complete `.env` files, prompts, responses, and raw Provider
   bodies out of examples and reports.
4. Treat cost and token values as estimates unless their exact provenance is
   stated.
5. Update behavior, examples, tests, and documentation in the same change.
6. Run `node scripts/check-doc-links.mjs` and the relevant checks from
   [Development](DEVELOPMENT.md) before merging.

Last reviewed: 2026-08-23.
