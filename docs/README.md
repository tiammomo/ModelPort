# ModelPort Documentation

This directory contains the maintained documentation for ModelPort. The root
[README](../README.md) is the short product entry point; the documents below are
the source of truth for behavior and operations.

## Start Here

- [Architecture](ARCHITECTURE.md): component boundaries, request flow, state,
  [technical core](ARCHITECTURE.md#technical-core), trust boundaries, and known
  design limits.
- [Configuration](CONFIGURATION.md): environment variables, TOML, precedence,
  provider activation, Qwen-only/DeepSeek/combined recipes, QuantPilot client
  secret boundaries, validation, and reload scope.
- [API](API.md): public endpoints, authentication, model resolution, errors,
  streaming, and control-plane route groups.
- [OIDC console sign-in](OIDC.md): optional single-host SSO preview,
  identity linking, configuration, security boundaries, and troubleshooting.
- [Operations](OPERATIONS.md): health checks, metrics, request logs, backup,
  reload, troubleshooting, and current operational limitations.
- [Data lifecycle](DATA_LIFECYCLE.md): retention ownership, append-only budget
  evidence, acceptance-test residue, and safe maintenance boundaries.
- [Development](DEVELOPMENT.md): toolchain, local workflow, test layers, and
  documentation maintenance.
- [Enterprise gateway roadmap](ENTERPRISE_ROADMAP.md): target product,
  enterprise admission criteria, architecture direction, migration workstreams,
  and release gates. Roadmap items are proposed unless explicitly marked as
  shipped elsewhere.
- [Dashboard experience](DASHBOARD_UX.md): navigation, role behavior, data-truth
  rules, page state contracts, responsive design, and accessibility.
- [systemd deployment](SYSTEMD.md): hardened single-host installation and data
  directory layout.
- [Docker Compose](DOCKER.md): the recommended complete deployment with the
  dashboard and PostgreSQL.

## Technical Core

ModelPort's implemented core is the bounded pipeline summarized in
[Architecture: Technical Core](ARCHITECTURE.md#technical-core): typed
Anthropic/OpenAI client edges and Provider adaptation, deterministic model routing, eligible
fallback, attempt-scoped policy and spend checks, environment-backed Provider
credentials, persisted control overrides, defensive transport, and
source-labelled observability. That section also records the non-distributed,
non-transactional, streaming, DNS, persistence, and estimation boundaries;
these limits are part of the design contract rather than optional caveats.

## Protocol And Providers

- [Provider compatibility matrix](PROVIDER_MATRIX.md)
- [Tool Use compatibility](TOOL_USE_COMPATIBILITY.md)
- [Local runtime integration](LOCAL_RUNTIME.md)
- [Production acceptance](ACCEPTANCE.md)
- [Performance and efficiency](PERFORMANCE.md)

## Maintainer Material

- [Security policy](../SECURITY.md)
- [Contributing guide](../CONTRIBUTING.md)
- [Project guide](PROJECT_GUIDE.md)
- [Architecture Decision Records](adr/README.md)
- [Repository and release setup](GITHUB_SETUP.md)
- [Learning and interview material](learning/README.md) — non-normative

## Documentation Contract

The implementation and checked examples take precedence when a conflict is
found. Documentation changes should keep these rules:

1. Distinguish **implemented**, **verified against a real upstream**, and
   **proposed** behavior.
2. Do not call a provider verified without a dated acceptance result.
3. Treat cost, token, latency, and health values as estimates unless the exact
   measurement path is documented. Preserve `upstream-returned` versus
   `local-estimate` provenance and do not describe a preflight rejection as
   consuming quota/spend.
4. Keep secrets, complete `.env` files, prompts, and raw provider bodies out of
   issues and documentation examples.
5. Run the checks in [Development](DEVELOPMENT.md#documentation-checks) after
   changing links, commands, provider defaults, or configuration names.

Last reviewed: 2026-07-18.
