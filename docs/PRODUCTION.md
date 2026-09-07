# Production

This document combines go-live and release acceptance for the free v0.1.x
Small-Team Beta: one Linux x86_64 instance serving an internal team of roughly
20–50 people. It is not an enterprise/HA claim, certification, warranty, SLA,
or substitute for the operator's threat model and compliance review.

## Supported Boundary

- One trusted backend host or small trusted network.
- PostgreSQL for every durable runtime state path.
- A same-origin HTTPS reverse proxy in front of dashboard and API.
- Server-side Provider credentials and dashboard-issued scoped client keys.
- Operator-owned capacity, upgrades, incidents, retention, backups, Provider
  contracts, privacy, and user support.

Public multi-tenant isolation, active-active operation, distributed
sessions/rate limits, SCIM, and a maintainer-hosted service are not currently
supported.

The project has no paid or hosted tier. “Enterprise mode” is the historical
name of a fail-closed configuration switch, not an enterprise-readiness claim.

The accepted forty-user hybrid-routing target is defined in
[ADR-0005](adr/0005-forty-user-hybrid-routing-baseline.md). Its first phase
still uses one ModelPort instance. Routing modes and per-user queue rules have
implementation and automated acceptance. Managed-secret injection is operator
owned, and active-active operation remains unsupported. These implemented
rules do not establish a particular real-model throughput or latency.

## Go-Live Checklist

- [ ] Pin a released image digest or verified binary provenance.
- [ ] Back up PostgreSQL and apply migrations to an isolated restored copy.
- [ ] Use PostgreSQL TLS `verify-full` for a remote production database.
- [ ] Set unique administrator, router, database, and Provider credentials.
- [ ] Enable `MODELPORT_ENTERPRISE_MODE=1` and resolve every guardrail failure.
- [ ] Configure secure cookies, exact HTTPS origins, exact trusted proxy CIDRs,
      enabled CSRF protection, and private backend/database ports.
- [ ] Set `MODELPORT_REQUIRE_CONTROL_API_KEYS=1`.
- [ ] Issue a dedicated scoped `MODELPORT_HEALTHCHECK_API_KEY`; never place it
      in Compose, Prometheus rules, Grafana variables, or alert annotations.
- [ ] Verify backup creation, restore drill, encryption, off-host replication,
      retention, and deletion ownership.
- [ ] Alert on readiness, rejection phases, request failures, ledger
      finalization/reconciliation, database saturation, Provider cooldown,
      routing disagreement, latency, and budget exhaustion.
- [ ] Record version, commit, image digest, configuration revision, migration
      set, rollback point, and incident contacts.

## Automated Acceptance

Run the isolated runtime gate on a Linux host with Docker, Node and the pinned
Rust toolchain:

```bash
MODELPORT_ASSURANCE_OUTPUT_DIR=/tmp/modelport-assurance scripts/acceptance.sh --isolated
```

It creates and removes its own PostgreSQL container, signs OIDC tokens with an
ephemeral key, and uses only loopback synthetic model responses. It tests both
Messages and Chat Completions text, live streams and complete Tool Use turns;
40 distinct scoped users across 400 paced requests; stream cancellation and
truncation; process restart; database interruption without unrecorded egress;
and restored auth/control fingerprints plus ledger row counts. CI additionally
verifies the checksum and GitHub attestation of v0.1.1, then runs that actual
binary against the restored database for paired application rollback.

Evidence contains commit/source state, latency distributions, rejection counts
and recovery outcomes, with no credentials or conversation content. Set
`MODELPORT_ASSURANCE_LOAD_SECONDS=60` for a longer paced run (1–120 seconds;
the request budget remains 400). This synthetic gate validates gateway behavior,
not production model capacity or production RTO/RPO. `capacity-acceptance.sh`
separately checks policy unit invariants. Neither script certifies a real GPU
or cloud Provider.

Run configuration validation before starting or restarting the candidate:

```bash
scripts/config-validate.sh
scripts/check-all.sh
scripts/acceptance.sh
scripts/tool-use-acceptance.sh
```

The default acceptance paths use fixtures and temporary control-plane objects.
They do not certify a real Provider. Calls made through a long-running gateway
can leave real request-ledger evidence even when temporary users, teams, keys,
and Providers are removed.

Explicit upstream checks can consume quota:

```bash
scripts/acceptance.sh --upstream
scripts/tool-use-acceptance.sh --upstream
scripts/provider-matrix.sh --model provider:model
```

Record exact Provider/model/path evidence according to
[Providers](PROVIDERS.md#discovery-and-verification). A configured model,
successful discovery, local mock pass, or stream HTTP 200 is not Provider
certification.

## Critical Manual Checks

- Authentication: valid/invalid/locked login, role-filtered navigation,
  ownership isolation, session clearing after principal changes.
- Keys and policy: one-time key reveal, owner status, provider/model/IP scope,
  quota units, rolling spend windows, expiry, revocation, and last-admin
  protection.
- Providers: create/update/disable/delete dependencies, credential state,
  discovery, pool fail-closed behavior, exact model call, stream completion,
  and Tool Use when promised.
- Requests: Messages and Chat Completions text/tool paths, unsupported-field
  rejection, request IDs, idempotency conflict, retry/fallback evidence, usage
  provenance, and smart-routing decision evidence.
- Streaming: handshake validation, semantic first-event latency, terminal SSE
  errors, cancellation, idle/byte limits, and concurrent-stream 429 behavior.
- Dashboard: empty/loading/error/stale states, range provenance, server-side
  log filtering, mobile layout, accessible dialogs, and no invented trace data.
- Operations: authenticated readiness, metrics, backup versus diagnostic export,
  restore drill, retention, migration rollback decision, and incident runbook.

## Release Evidence

Keep:

- commit, build, deployment mode, image digest, SBOM, checksums, and provenance;
- CI, dependency, security, migration, dashboard, and acceptance results;
- exact commands and whether they made paid calls;
- Provider/model/endpoint ownership and dated real-upstream evidence;
- storage backend, backup, restore-drill, retention, and rollback results;
- accepted limits for streaming, quota concurrency, DNS egress, persistence,
  estimates, and process-local enforcement.

A fixture-backed pass supports a controlled gateway trial. A dated Provider
pass supports only the exact model, path, account conditions, and commit tested.

## Deployment-Specific Evidence

Keep the following as pending until the named deployment has produced and
retained the evidence. Repository CI cannot complete these rows for an operator.

| Gate | Required evidence |
| --- | --- |
| Identity | Actual issuer/client/callback and HTTPS proxy; enforced MFA class; disabled-user behavior; session lifetime; tested operator recovery. |
| Provider protocols | Exact image digest, model, endpoint, account and date; both required client protocols; text, stream completion, tool-result continuation, failures and cancellation; an explicit request/cost cap. |
| Capacity | Actual host/GPU/model and workload; 40 separate identities; sustained concurrency, semantic TTFT/P95, queue/rejection counts, CPU/memory/DB pool use; agreed pass thresholds. |
| Recovery | Encrypted off-host backup, managed PostgreSQL TLS/PITR, timed restore and paired application rollback; operator-approved RTO/RPO. |
| Operations | Alerts delivered to an assigned owner, backup operator and maintenance window; candidate digest recorded and post-deployment smoke passed. |

Routine acceptance uses fixtures. Run paid upstream checks only for a named
account/model with an explicit budget. A published Release or merged `main`
commit does not establish which digest a production host is running.

## Reliability Objectives

ModelPort does not publish a universal end-to-end SLO because Provider
availability and local inference capacity dominate results. Each operator
should define:

- data-plane availability and error budget;
- Tool Use workflow success;
- semantic TTFT and full-lifecycle P50/P95 by workload class;
- maximum ledger finalization lag and unreconciled lease count;
- recovery time and recovery point;
- maximum accepted billing reconciliation variance.

Synthetic checks must use the `synthetic` traffic class so they do not distort
business success and cost views.
