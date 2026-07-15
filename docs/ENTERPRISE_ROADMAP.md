# Enterprise Gateway Roadmap

Status: approved product direction, implementation proposed unless a capability
is explicitly marked as shipped in the current documentation.

Last reviewed: 2026-07-15.

## Executive Decision

ModelPort will evolve from a trusted-host Anthropic-compatible gateway into a
self-hosted enterprise model gateway. The target product provides a stable,
multi-protocol data plane and a governed, multi-tenant control plane across
commercial and private model Providers.

This direction does not make the current `0.1.x` implementation enterprise
ready. Until the release gates in this document are met, documentation and
release notes must continue to describe the shipped single-host and small-team
boundaries accurately.

The transition has five non-negotiable outcomes:

1. OpenAI-compatible, OpenAI Responses, and Anthropic clients share the same
   identity, policy, routing, budget, and observability pipeline.
2. Tenant isolation, quota reservation, usage accounting, audit history, and
   configuration changes use transactional, queryable persistence.
3. Data-plane replicas can scale horizontally without weakening authorization,
   limits, health decisions, or accounting correctness.
4. Enterprise identity, secret management, data governance, and audit export
   are first-class product capabilities.
5. Compatibility and reliability claims are backed by automated conformance,
   load, failure, upgrade, and recovery evidence.

## What “Enterprise Ready” Means

ModelPort may be described as enterprise ready only after all of the following
admission criteria are satisfied.

| Area | Required admission evidence |
| --- | --- |
| Protocols | Anthropic Messages and OpenAI Chat Completions are GA; Responses is at least a documented beta; request, response, Tool Use, structured output, error, cancellation, and streaming contracts have conformance tests. |
| Tenancy | Organization and project isolation is enforced on every persisted object and request path; negative cross-tenant tests run in CI. |
| Identity | OIDC SSO, service accounts, scoped API keys, group/role mapping, revocation, and auditable impersonation boundaries are implemented. SCIM is required for the managed enterprise profile, but not the first self-hosted preview. |
| Authorization | Resource-level RBAC and request-time model/Provider/tool/data policies fail closed and have a documented policy evaluation order. |
| Accounting | Usage is append-oriented, idempotent, decimal-valued, versioned by pricing source, stream-finalized, and reconcilable. Hard budgets use transactional reservation and settlement. |
| Availability | At least two stateless data-plane replicas pass failover, rolling-upgrade, drain, and dependency-degradation tests. No correctness-critical limiter or session exists only in one process. |
| Security | TLS is supported for every network hop; secret values are never stored in control records; DNS-aware egress controls, dependency scanning, threat modeling, and security regression tests are release gates. |
| Observability | W3C trace context, OpenTelemetry export, bounded-cardinality metrics, structured logs, audit events, and end-to-end stream outcome telemetry are available without recording prompt content by default. |
| Operations | Schema migrations, backup/restore, point-in-time recovery guidance, rollback, disaster-recovery exercises, retention jobs, and version-skew policy are documented and tested. |
| Governance | Model inventory, Provider verification evidence, data retention and residency policy, content-policy hooks, and usage/audit export are tenant configurable. |

Certifications such as SOC 2 or ISO 27001 are organizational audit outcomes,
not software features. ModelPort can produce controls and evidence that support
an audit, but the repository must never claim certification on its own.

## Current Baseline And Structural Gaps

The existing gateway already has useful foundations: bounded transport, a first
typed Exchange IR, Anthropic Messages and scoped OpenAI Chat Completions client
edges, Anthropic/OpenAI Provider adaptation, common Tool Use conversion,
terminal stream usage evidence, deterministic routing, API keys, teams, policy
checks, quotas, Provider credential pools, health/cooldown behavior, PostgreSQL
or JSON persistence, a dashboard, metrics, and acceptance scripts.

The enterprise transition must address these structural gaps before adding a
large catalog of surface features.

| Current implementation | Enterprise consequence | Direction |
| --- | --- | --- |
| The first typed Exchange IR covers text roles and common Tool Use, but not multimodal content, Responses items, reasoning, or all structured-output semantics. | Expanding through generic JSON would reintroduce silent protocol loss. | Extend the typed IR and capability/fidelity report for each new content or item type. |
| Public data plane exposes `/v1/messages` and a scoped `/v1/chat/completions`; Responses is absent. | OpenAI text SDKs can use the gateway, but Responses clients and unsupported Chat features cannot. | Expand Chat conformance and add typed `/v1/responses` while preserving both existing edges. |
| Auth and control state are two whole JSON documents, including when stored in PostgreSQL. | Writes, filtering, migrations, isolation, retention, and concurrent updates cannot meet enterprise requirements. | Replace the document store with normalized relational repositories and versioned migrations. Keep file storage for development only. |
| Compatibility auth/control access still serializes whole-document operations through one worker per namespace; only the new ledger is row-oriented and fully async. | Identity, policy, quota, and dashboard-log concurrency still cannot meet the target. | Move each remaining document domain to tenant-scoped async repositories and transactions, then remove the compatibility workers. |
| Sessions, login attempts, rate limits, stream permits, metrics, and parts of Provider health are process-local. | Horizontal replicas can make inconsistent decisions and reset enforcement on restart. | Move shared enforcement to PostgreSQL/Redis-backed services; retain local limits only as an additional safety valve. |
| Compatibility user/API-key/team quota and spend checks are preflight estimates followed by later updates; the initial tenant budget now reserves atomically before Provider egress. | The tenant hard limit is protected, but the remaining compatibility limit dimensions can still overshoot under concurrency. | Move every hard limit onto tenant-scoped reservation, settlement, release, and expiry records. |
| The tenant budget uses integer micro-USD balances and append-only events, while compatibility usage logs and aggregates still use floating-point values and mutable documents. | The initial budget evidence is safer, but the complete usage and pricing history is not yet an auditable financial ledger. | Store decimal monetary values, currency, price-book version, source, and immutable adjustments across all accounting domains. |
| Request/attempt state is persisted before egress; live owners renew leases, and a durable worker terminalizes expired rows as unbilled `unreconciled` evidence. | An expired lease cannot prove whether the Provider accepted work or reconstruct missing usage. | Add Provider evidence ingestion and operator review so exact settlement can append corrective evidence without mutating the original attempt. |
| Admin roles are coarse and local sessions are memory-only. | They do not express enterprise ownership or survive multi-instance deployment. | Add OIDC, shared sessions or signed short-lived sessions, organizations, projects, groups, service accounts, and resource-level role bindings. |
| Provider secrets are environment-variable references. | Rotation and external secret managers are not managed consistently. | Define a secret-reference interface for Vault and cloud secret managers; cache values briefly and audit access without persisting secret material. |
| Request IDs correlate local records, but there is no distributed trace context. | Cross-service and upstream latency cannot be diagnosed consistently. | Propagate W3C `traceparent`/`tracestate` and export OpenTelemetry traces, metrics, and logs. |
| Provider URL validation does not pin or revalidate DNS answers. | A configured remote Provider can cross an egress trust boundary through DNS changes. | Resolve through a controlled resolver, reject disallowed address classes on every connection, and support explicit egress allowlists/proxies. |

## Product Scope

### In scope

- A self-hosted data plane for Anthropic Messages, OpenAI Chat Completions,
  OpenAI Responses, and later embeddings and selected multimodal operations.
- Organization and project tenancy for internal enterprise teams and workloads.
- Policy-governed access to hosted Providers, private endpoints, and local
  inference runtimes.
- Routing by availability, capability, policy, region, cost, latency, and
  operator-defined priority.
- Enterprise identity, API-key and service-account lifecycle, budgets, usage,
  audit, observability, and operational delivery.
- A control-plane API and dashboard built on the same authorization model.

### Explicit non-goals for the enterprise gateway core

- Model inference, training, fine-tuning orchestration, or GPU scheduling.
- A general chat product or end-user prompt workspace.
- Acting as a payment processor or issuing legally authoritative Provider
  invoices.
- Silently emulating every Provider-specific feature through a lowest-common-
  denominator schema.
- Building a custom identity provider, secret manager, metrics database, or
  policy language when established integrations satisfy the requirement.
- Splitting the codebase into many network services before independent scaling
  or security boundaries require it.

## Architecture Principles

1. **One governance pipeline, multiple protocol edges.** Authentication,
   authorization, routing, budgets, attempts, and telemetry execute once. Edge
   adapters only parse and render protocol-specific contracts.
2. **Preserve meaning or report loss.** Adapters must not silently discard
   tools, reasoning items, structured output, multimodal content, stop reasons,
   usage, or safety metadata. A capability/fidelity decision occurs before an
   upstream call.
3. **The database is authoritative.** PostgreSQL owns identity, configuration,
   reservations, usage, audit, and durable workflow state. Redis accelerates
   distributed limits, caches, and leases; it is not the final usage ledger.
4. **A modular monolith comes before microservices.** Protocol, policy,
   routing, storage, and telemetry become internal modules with stable traits.
   The binary can later run as `data-plane`, `control-plane`, `worker`, or
   `all-in-one` roles without duplicating domain logic.
5. **The data plane remains usable during control-plane degradation.** A
   versioned last-known-good routing/policy snapshot can serve requests for a
   bounded interval. Security-sensitive revocations and budget exhaustion must
   define explicit fail-open or fail-closed behavior, defaulting to fail closed.
6. **Privacy is the default.** Prompt and completion content is not persisted or
   exported by default. Any content capture is scoped, sampled, encrypted,
   retained for a bounded period, and visible in audit history.
7. **Every claim needs evidence.** “Supported,” “verified,” “highly available,”
   and “enterprise ready” correspond to committed tests and dated release
   evidence.

## Target Logical Architecture

```text
Clients and SDKs
  Anthropic Messages | OpenAI Chat | OpenAI Responses | Embeddings (later)
                              |
                    ingress / load balancer
                              |
               +--------------+--------------+
               | stateless data-plane replicas|
               | parse -> identity -> policy  |
               | -> budget reserve -> route   |
               | -> adapt -> stream/finalize  |
               +--------------+--------------+
                              |
              +---------------+----------------+
              | Provider adapters and egress   |
              | hosted APIs | private APIs | ML |
              +---------------+----------------+

       control-plane API             background workers
  identity, policy, routes,     reconciliation, retention,
  secrets, budgets, audit       probes, exports, rollups
              |                         |
              +------------+------------+
                           |
             PostgreSQL (authoritative state/ledger)
             Redis (limits, cache, leases; optional in dev)
             Secret manager (secret material)
             OTLP collector (telemetry export)
```

The first enterprise preview may ship all roles in one process. The domain and
storage interfaces must nevertheless prevent process memory from being the only
source of correctness.

## Core Domain Model

The relational model should establish these ownership boundaries before UI or
API expansion:

```text
Organization
  Project
    Environment
    API client / Service account
    Virtual model and route policy
    Budget and quota
    Usage request -> Provider attempt -> settlement

Organization
  User / Group
  Role binding -> resource scope
  Provider connection -> secret reference -> credential version
  Data, retention, egress, and content policies
  Audit event
```

Minimum identifiers carried in a data-plane `RequestContext`:

- request ID, trace ID, protocol, and idempotency key when supplied;
- organization ID, project ID, environment ID, and principal ID;
- API client/service-account ID and credential ID/version;
- requested virtual model plus resolved Provider/model/region;
- effective policy revision, route revision, and price-book revision;
- data classification and retention mode when policy defines them.

Tenant identity is derived from authenticated credentials and role bindings,
never trusted from an arbitrary tenant header. Every tenant-owned table uses
tenant-scoped uniqueness and foreign keys. PostgreSQL row-level security can be
added as defense in depth, but it does not replace explicit repository scoping
and negative isolation tests.

## Protocol And Adapter Architecture

### Stable client surfaces

| Surface | Product role | Planned status |
| --- | --- | --- |
| `POST /v1/messages` | Claude Code, Anthropic SDK, existing ModelPort clients | Preserve and graduate to GA conformance. |
| `POST /v1/chat/completions` | Broad OpenAI-compatible SDK and private inference compatibility | Initial text/function-tool compatibility slice shipped; conformance expansion required before GA. |
| `POST /v1/responses` | Typed items, agentic/tool workflows, modern OpenAI clients | Implement after the typed exchange model; beta before GA. |
| `GET /v1/models` | Project-scoped virtual model catalog | Return only models visible to the authenticated project and protocol capabilities. |
| `POST /v1/embeddings` | Retrieval and semantic search workloads | Add after core text protocol and accounting foundations. |
| Images/audio/realtime/batch | Workload-specific APIs | Separate RFCs; do not force them through the text generation pipeline. |

OpenAI currently recommends Responses for new projects while continuing to
support Chat Completions. ModelPort therefore needs both: Chat Completions for
ecosystem compatibility and Responses for future typed workflows.

### Typed exchange model

The internal exchange model is not a generic JSON bag. It should include:

- ordered input/output items with stable IDs;
- text, image, audio, document-reference, and refusal content parts;
- system/developer/user/assistant semantics without flattening them to text;
- tool definitions, tool calls, tool results, call IDs, parallelism, and JSON
  schema strictness;
- reasoning references and encrypted/opaque Provider items without exposing
  hidden reasoning content;
- structured-output constraints;
- sampling, stop, token, caching, and service-tier controls;
- typed stream lifecycle events and terminal outcomes;
- normalized usage dimensions and raw Provider usage in a bounded,
  non-customer-visible evidence record;
- protocol-native extensions namespaced by Provider and guarded by policy.

Every adapter implements four operations:

1. Parse client protocol into the exchange model.
2. Declare required capabilities and potential fidelity loss.
3. Render the selected Provider request and parse its result/events.
4. Render the client response/events and normalized error.

Adapters publish a capability manifest. Routing excludes a Provider before
budget reservation when it cannot satisfy a required capability. `strict`,
`compatible`, and `best-effort` fidelity policies remain explicit and are
recorded on every attempt.

## Routing And Provider Governance

Routing evolves from model-name matching into a versioned policy decision.
Inputs may include:

- organization/project allowlists and data classification;
- required modalities, tools, structured output, context, and stream behavior;
- Provider, model, region, residency, and private-network constraints;
- credential health, account status, concurrency, and rate-limit headroom;
- operator priority, weighted distribution, cost ceiling, and latency target;
- retry safety, request idempotency, and whether response headers were sent.

The resolved route is recorded before the Provider call. Each Provider attempt
has a unique attempt ID and an explicit reason for selection, skip, retry,
fallback, or rejection. Retries and hedging are disabled by default for
side-effecting tools and any request that cannot be proven replay safe.

Provider onboarding requires a conformance package rather than only a catalog
entry:

- endpoint and authentication checks;
- model discovery evidence;
- non-stream and stream termination tests;
- Tool Use and structured-output fixtures;
- usage and error mapping;
- timeout, 429, 5xx, invalid body, and cancellation behavior;
- dated real-upstream verification where credentials are available.

## Identity, Authorization, And Tenancy

### Human identity

- OIDC Authorization Code flow with PKCE for the dashboard.
- Issuer allowlist, discovery/JWKS caching, key rotation, nonce/state checks,
  claim validation, and bounded clock skew.
- Organization membership and group-to-role mapping.
- Local bootstrap admin retained only for recovery and disable-able after SSO.
- SCIM 2.0 Users and Groups provisioning in the managed enterprise profile.

### Workload identity

- Hashed, prefix-identifiable API keys with expiry, scopes, last-used metadata,
  rotation overlap, and immediate revocation.
- Service accounts separated from human users.
- OAuth 2.0 JWT access-token validation for workload federation.
- Optional mTLS identity for private data-plane clients and internal roles.

### Authorization

Initial built-in roles should separate organization ownership, security,
billing, operations, project administration, development, audit, and read-only
access. Role bindings are scoped to an organization or project. Resource-level
checks apply to every control-plane object; model invocation also evaluates
request attributes such as model, Provider, tools, region, and data class.

The first policy engine should remain a typed, testable in-process evaluator.
An OPA/Cedar-style external policy integration can be added behind a stable
decision interface when customers require centrally managed policy. ModelPort
must not invent an unversioned string policy language.

## Usage, Budgets, And Cost Accounting

One client request can create multiple Provider attempts but exactly one final
request outcome. The durable flow is:

```text
request accepted
  -> estimate and reserve budget atomically
  -> record each Provider attempt
  -> consume Provider response/stream
  -> finalize actual or estimated usage
  -> settle/release reservation
  -> append adjustment if later reconciliation changes evidence
```

Requirements:

- use PostgreSQL `NUMERIC`/decimal values and explicit currency;
- keep input, output, cached, reasoning, image, audio, and Provider-specific
  usage dimensions independently when available;
- attach price-book/provider-price revision and evidence source;
- make request creation, reservation, settlement, and idempotency transactional;
- never mutate historical monetary evidence silently; append adjustments;
- expire abandoned reservations through a worker with an audit trail;
- distinguish Provider usage, tokenizer estimate, and configured maximum;
- reconcile stream-final events and Provider billing exports when available;
- expose usage export, not an assertion that ModelPort replaces the invoice.

## Security And Data Governance

The enterprise threat model covers both hostile clients and compromised or
misconfigured upstream APIs. Required controls include:

- TLS for inbound, PostgreSQL, Redis, secret-manager, and telemetry traffic;
- optional mTLS and custom CA bundles for private enterprise networks;
- DNS-aware Provider resolution, IP-range validation on every connection,
  redirect denial, egress allowlists, and proxy policy;
- external secret references, envelope encryption where ModelPort owns durable
  sensitive values, rotation, least-privilege access, and redacted diagnostics;
- bounded bodies, events, streams, tools, schemas, concurrency, and queue time;
- object- and property-level authorization tests for every admin resource;
- signed release artifacts, SBOM generation, dependency/license scanning,
  secret scanning, and a vulnerability response policy;
- configurable prompt/response retention, deletion, export, residency, and
  content-capture consent;
- DLP/content-policy hooks before Provider egress and after Provider response,
  with latency and false-positive behavior visible to operators;
- append-oriented audit events for identity, policy, route, Provider,
  credential-reference, budget, export, and administrative changes;
- export to an external SIEM or immutable archive for customers needing WORM
  retention; a writable database table alone is not “immutable audit.”

Security acceptance should map test cases to the OWASP API Security Top 10,
including authorization, resource exhaustion, SSRF, misconfiguration, inventory,
and unsafe consumption of third-party APIs. AI governance should use the NIST AI
RMF and its Generative AI Profile as references without claiming automatic
regulatory compliance.

## High Availability And Runtime State

Enterprise deployments use stateless data-plane replicas behind a load
balancer. PostgreSQL is authoritative; Redis is recommended for high-rate
distributed counters, short leases, and cache invalidation.

Shared-state rules:

- hard budget reservations and usage settlement remain transactional in
  PostgreSQL;
- distributed rate limits use atomic Redis operations with explicit behavior
  during Redis loss;
- Provider/credential health uses timestamped shared observations and leases;
- policy and route snapshots are versioned, cached, and invalidated through a
  revision channel;
- browser sessions are shared or cryptographically verifiable and revocable;
- stream concurrency has local safety caps plus project/Provider-wide limits;
- background jobs use leases and idempotent handlers;
- graceful shutdown stops accepting new calls, drains normal requests, gives
  streams a configured grace period, and records forced termination.

Deployment deliverables include a Helm chart, PodDisruptionBudget, topology
spread guidance, autoscaling signals, network policies, external secret
examples, migration jobs, and separate liveness/readiness/startup probes. Docker
Compose remains the development and evaluation profile.

## Observability And SLOs

ModelPort will propagate W3C `traceparent` and `tracestate` and export OTLP.
OpenTelemetry Generative AI conventions are evolving, so the implementation
must pin a documented convention version and isolate semantic mapping from the
core domain.

Required service indicators include:

- request rate and terminal success/error/cancellation by protocol;
- gateway overhead separately from Provider time-to-first-byte and generation;
- complete stream duration and terminal reason;
- policy, budget, routing, retry, and fallback decisions;
- active/queued requests and streams by bounded dimensions;
- Provider/credential availability, throttling, and circuit state;
- reservation/settlement/reconciliation lag and failures;
- control-plane/database/cache/secret-manager dependency health;
- audit and telemetry export lag/drop counts.

Metrics labels must use bounded identifiers or controlled catalogs. Raw prompts,
completion text, API keys, and unbounded user-supplied model strings are not
metric labels. Trace content capture is off by default.

Before GA, publish SLOs for availability, gateway overhead, control-plane
mutation durability, revocation propagation, and usage-finalization lag. Numeric
targets should be selected from repeatable load and failure tests rather than
invented in this planning document.

## Delivery Workstreams

### E0 — Correctness and contracts (P0)

- Write ADRs for tenancy, the typed exchange model, persistence, consistency,
  failure semantics, and deployable roles.
- Add a terminal stream lifecycle object that records completion, upstream
  error, downstream cancellation, timeout, byte-limit termination, and forced
  shutdown.
- Define request/attempt/usage/audit IDs and idempotency behavior.
- Freeze current Anthropic behavior with black-box conformance fixtures.
- Add an OpenAPI contract for ModelPort-owned control-plane APIs and machine-
  readable protocol fixtures for vendor-compatible data-plane endpoints.

Exit gate: current `/v1/messages` behavior and every stream terminal state are
observable and regression tested; architectural decisions are reviewed.

### E1 — Relational and tenant foundation (P0)

Current progress (2026-07-15): SQLx/Tokio pooling, rustls TLS policy, embedded
versioned migrations, organization/project/environment keys, and mandatory-
tenant request/Provider-attempt lifecycle rows, hashed request idempotency
claims, renewable instance leases, periodic expired-row reconciliation,
transactional tenant-budget reservation/settlement/release, and append-only
manual adjustment evidence are shipped. The compatibility auth/control
documents still exist, and response replay, principals, memberships, policies,
Provider evidence ingestion, import/rollback tooling, and multi-instance
conflict tests remain open. The E1 exit gate is therefore not met.

- Introduce async pooled PostgreSQL with TLS and versioned migrations.
- Create organization, project, environment, principal, membership, role
  binding, API client, Provider connection, route, policy, request, attempt,
  reservation, usage, adjustment, and audit tables.
- Add repository interfaces with mandatory tenant scope and concurrency tests.
- Migrate current auth/control documents through an idempotent import command;
  retain validated backups and a documented rollback boundary.
- Keep JSON storage only for a non-enterprise development profile.

Exit gate: two application instances can mutate independent tenant data without
lost updates; cross-tenant access tests fail closed; backup/restore and migration
rollback exercises pass.

### E2 — Multi-protocol data plane (P0)

Current progress (2026-07-15): the initial text/function-tool Exchange IR,
shared governance handler, OpenAI Chat route, Anthropic/OpenAI non-stream
rendering, standard chunk streaming, optional usage chunks, terminal usage
reconciliation, and initial four-way fixtures are shipped. Responses,
multimodal/structured item types, complete error/tool stream conformance, and a
published fidelity matrix remain open, so the E2 exit gate is not met.

- Implement the typed exchange model for text, tools, structured output, usage,
  errors, and stream lifecycle.
- Refactor the existing Anthropic and OpenAI Provider paths behind adapter
  traits and capability manifests.
- Add `/v1/chat/completions` with official SDK compatibility tests.
- Add `/v1/responses` as beta with typed item and typed SSE tests.
- Make `/v1/models` project- and protocol-aware.
- Publish a fidelity matrix and Provider conformance harness.

Exit gate: Anthropic and OpenAI clients can call Anthropic- and OpenAI-compatible
upstreams in all four combinations for non-stream, stream, tools, errors, and
usage without silent semantic loss.

### E3 — Enterprise identity, policy, and secrets (P0)

- Add OIDC SSO, group mapping, service accounts, scoped credentials, shared
  revocation, and recovery-admin controls.
- Replace coarse admin checks with tenant/resource role bindings.
- Add versioned invocation policies for models, Providers, tools, data class,
  region, retention, and cost.
- Add secret-manager interfaces, rotation, credential versioning, and audit.
- Add SCIM after OIDC and role semantics are stable.

Exit gate: identity lifecycle, privilege escalation, confused-deputy,
cross-tenant, revocation, key rotation, and secret-failure tests pass.

### E4 — Distributed enforcement and high availability (P0)

- Add atomic budget reservation/settlement and incomplete-request recovery.
- Add Redis-backed distributed rate limits, caches, leases, and invalidation.
- Share Provider/credential health and circuit decisions across replicas.
- Add deployable roles, graceful stream drain, rolling-upgrade compatibility,
  Helm packaging, and dependency-degradation modes.
- Run load, soak, failover, chaos, and disaster-recovery exercises.

Exit gate: loss of one data-plane replica causes no tenant isolation, budget,
audit, or completed-request evidence loss; rolling upgrades meet the published
availability objective.

### E5 — Observability, governance, and enterprise GA (P1)

- Add W3C trace propagation, OTLP export, dashboards, alerts, and SLO reports.
- Add retention/deletion jobs, usage and audit exports, SIEM integration, and
  content-policy hooks.
- Add model approval and lifecycle workflows plus dated Provider verification.
- Add SBOM/provenance, signed releases, upgrade compatibility tests, security
  review evidence, and an operator runbook for incidents and recovery.
- Complete the enterprise admission checklist and remove preview labels only
  when every required row has evidence.

Exit gate: an enterprise release evidence bundle is reproducible from CI and a
staged multi-replica environment.

### E6 — Post-GA expansion (P2)

- Embeddings GA, then separate image, audio, batch, and realtime RFCs.
- Advanced routing optimization, customer-managed policy engines, chargeback
  reports, billing-export reconciliation, and regional control planes.
- SCIM event extensions, customer-managed encryption keys, private connectivity
  templates, and additional compliance evidence integrations where demanded.

## First Implementation Tranche

The first tranche is now partially shipped and remains small enough to review as
a coherent foundation:

1. ADR: enterprise tenancy and deployable-role boundaries.
2. ADR: typed exchange model and protocol fidelity rules.
3. ADR: relational storage, transaction boundaries, and migration strategy.
4. Add `RequestContext`, `RequestId`, `AttemptId`, and `TenantScope` domain
   types without changing the public API.
5. Add a stream finalizer that distinguishes completed, upstream failed,
   downstream cancelled, limited, timed out, and shutdown outcomes.
6. Persist request and attempt lifecycle through a new repository trait while
   keeping the legacy store behind an adapter during migration.
7. Add black-box Anthropic fixtures that protect current behavior before the
   internal protocol refactor.
8. Create the minimal typed exchange model for text and common Tool Use.
9. Route the existing `/v1/messages` path through that model with no contract
   change.
10. Add `/v1/chat/completions` only after the initial four-way adapter tests
    pass. The text/function-tool compatibility slice now follows this boundary;
    it is not yet the full E2 conformance gate.

This order fixes current correctness gaps and creates a safe seam for the first
new enterprise-facing protocol instead of adding a second handler coupled to
`AnthropicRequest`.

## Migration And Compatibility Policy

- Existing `/v1/messages`, model aliases, API keys, and single-host deployment
  remain supported through a documented compatibility window.
- The enterprise database importer is explicit, idempotent, dry-run capable,
  checksummed, and produces a rollback backup. Startup must not silently rewrite
  legacy state.
- Public API breaking changes require a versioned endpoint or a declared major
  release. Provider quirks stay in adapters, not global protocol behavior.
- Database migrations support the previous released application version during
  rolling upgrades whenever practical; destructive cleanup occurs in a later
  release.
- Policy and route changes are revisioned and auditable. Data-plane replicas
  report the revision serving each request.
- Compatibility tests use official client SDKs where possible and raw HTTP
  fixtures where SDK behavior would hide protocol details.

## Quality And Release Gates

Every enterprise milestone adds evidence in these layers:

- unit and property tests for protocol, policy, money, and tenant-scoping logic;
- golden request/response/SSE fixtures for each client/Provider protocol pair;
- integration tests with PostgreSQL, Redis, OIDC, secret-manager stubs, and OTLP;
- negative authorization and cross-tenant tests;
- paid real-Provider tests only in explicit, budget-capped workflows;
- load and soak tests for non-stream, stream, cancellation, and slow consumers;
- fault injection for DNS, TLS, database, Redis, Provider, worker, and replica
  loss;
- migration, backup/restore, point-in-time recovery, and rolling-upgrade tests;
- SAST, dependency, license, secret, container, and SBOM/provenance checks;
- accessibility and browser E2E tests for new control-plane workflows.

## Decision Queue

The following decisions need ADRs before their implementations merge:

1. Organization/project/environment hierarchy and tenant-isolation strategy.
2. Exchange-model item types, native extension policy, and fidelity contract.
3. Async PostgreSQL library, migration tool, pooling, TLS, and repository
   boundaries.
4. Budget reservation consistency model and monetary precision.
5. Redis failure policy for rate limits, caches, health, and revocation.
6. OIDC session strategy, group claims, service accounts, and SCIM scope.
7. Secret-manager interface and supported first-party backends.
8. Route-policy schema, revisioning, and whether/when to integrate an external
   policy engine.
9. OTLP signal model, sampling, semantic-convention version, and content-capture
   controls.
10. Data-plane/control-plane/worker process roles and version-skew contract.
11. Audit export integrity model and external immutable-storage integration.
12. Enterprise release SLO, RPO, RTO, support window, and deprecation policy.

Accepted decisions are indexed in the
[Architecture Decision Records](adr/README.md). Items above remain in the queue
until the corresponding ADR is accepted; ADR-0001 through ADR-0003 establish the
initial tenancy, protocol-exchange, and relational-storage direction.

## Standards And External References

- OpenAI, [Migrate to the Responses API](https://developers.openai.com/api/docs/guides/migrate-to-responses).
- Anthropic, [Messages API](https://docs.anthropic.com/en/api/messages).
- OpenID Foundation, [OpenID Connect Core 1.0](https://openid.net/specs/openid-connect-core-1_0-final.html).
- IETF, [SCIM Protocol RFC 7644](https://www.rfc-editor.org/rfc/rfc7644).
- W3C, [Trace Context](https://www.w3.org/TR/trace-context/).
- OpenTelemetry, [GenAI Semantic Conventions](https://github.com/open-telemetry/semantic-conventions-genai).
- OWASP, [API Security Top 10 2023](https://owasp.org/API-Security/editions/2023/en/0x00-header/).
- NIST, [AI Risk Management Framework](https://www.nist.gov/itl/ai-risk-management-framework).

These references guide interoperability and control design. They do not by
themselves establish product compliance or Provider compatibility.
