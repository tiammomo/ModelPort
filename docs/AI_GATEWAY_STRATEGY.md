# Post-Beta AI Gateway Strategy

Status: accepted product decision framework; target capabilities remain proposed
unless another maintained document marks them implemented.

Last reviewed: 2026-08-24.

## Decision

ModelPort will mature as a governed, self-hosted hybrid AI gateway and model/GPU
control plane. It will pursue parity in the capability families that make a
small team's routes safe, operable, and explainable, not parity by feature or
Provider count. The v0.1.x Small-Team Beta contract and single-process Rust data
path remain the baseline.

This strategy uses four status labels:

- **Shipped**: implemented in the current supported product.
- **Hardening**: implemented in scope, but the next work improves activation,
  evidence, correctness, or operator confidence.
- **Evidence-gated**: a candidate only after the stated design-partner and
  operational evidence exists.
- **Deferred**: outside the post-Beta sequence; it requires a new decision and
  must not be implied by current APIs or UI.

## Market evidence and interpretation

Official documentation from [LiteLLM](https://docs.litellm.ai/docs/simple_proxy),
[Portkey](https://portkey.ai/docs/product/ai-gateway),
[Kong AI Gateway](https://docs.konghq.com/gateway/latest/ai-gateway/),
[Envoy AI Gateway](https://aigateway.envoyproxy.io/docs/),
[OpenRouter](https://openrouter.ai/docs/guides/routing/provider-selection),
[Cloudflare AI Gateway](https://developers.cloudflare.com/ai-gateway/), and
[New API](https://docs.newapi.pro/en/) establish recurring families: normalized
ingress, centralized upstream credentials and models, routing/retries/fallback,
usage governance, observability, caching, and policy extensions. These sources
are comparison inputs, not compatibility claims. Product names, breadth, and
deployment models differ, and ModelPort does not embed or depend on them.

Two standards constrain the direction. OpenTelemetry's
[generative AI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/)
provide an interoperable vocabulary, while the
[W3C Trace Context recommendation](https://www.w3.org/TR/trace-context/)
defines propagation. Neither permits ModelPort to export prompt, response, tool,
credential, or unbounded attribute content by default.

## Capability parity map

| Capability family | Current evidence | Status and next boundary |
| --- | --- | --- |
| Unified protocol ingress | Anthropic Messages and scoped OpenAI Chat Completions share a typed Exchange IR, bounded SSE, Tool Use conversion, and fail-closed fidelity checks. | **Hardening.** Preserve native semantics and publish capability evidence per client/Provider/model tuple. OpenAI Responses, realtime, embeddings, and broad multimodal ingress are **deferred** until a design partner has a blocking workload and the IR can reject loss rather than silently downgrade. |
| Typed Provider, credential, and model management | Provider configuration, environment-backed credentials, pools, health/cooldown, aliases, model inventory, and logical routing ship. | **Hardening.** Replace relay-style “channels” with distinct Provider, credential reference, Model, and Route resources. A credential belongs to an upstream trust/account boundary; it is never a Client profile or a Model. Secret-manager integrations are **evidence-gated**. |
| Routing and resilience | Deterministic aliases, bounded retry/fallback, cooldown, capability/policy gates, shadow/canary smart routing, and durable decision evidence ship. | **Hardening.** Make eligibility, attempt order, exclusions, fallback reason, and terminal outcome explainable. Persist trustworthy latency/reliability evidence before distributed routing. Semantic routing and online weight learning are **deferred**. |
| Identity, quota, and budget governance | Scoped keys, users, teams, IP/model/Provider policy, quotas, spend controls, atomic reservations, and approvals ship for the single-host profile. | **Hardening.** Prioritize exact ownership and exact cost provenance from Provider usage plus versioned rate cards. Additional identity integrations and resource policy are **evidence-gated**. Hosted/public tenancy, payments, and reseller billing are **deferred**. |
| Content-minimized observability | Request/attempt IDs, route and policy evidence, token/cost provenance, latency, metrics, retained logs, audit rows, and operations diagnostics ship without intentional prompt/response persistence. | **Hardening.** Add content-free, OpenTelemetry-compatible spans/metrics and trace-context propagation with bounded cardinality, tenant access controls, sampling, and retention. “Provider-reported” is not invoice-exact; unknown or estimated costs stay labelled. Prompt-content telemetry is **deferred**. |
| Operator UX | Dashboard flows cover onboarding, Providers, routing, governance, requests, budgets, incidents, backup, and operations. | **Hardening.** Optimize first governed request, Provider credential test state, capability evidence, route explanation, and copyable Client profiles. Compute and Deployment views follow the dependency chain below. |
| Cache and guardrail extensions | No general response cache or pluggable content guardrail is claimed. Existing size, protocol, policy, and egress checks are gateway safety controls, not that product category. | **Evidence-gated.** See the extension contract below. Exact and semantic cache are separate proposals; semantic cache follows exact-cache evidence. Guardrails remain opt-in policy components, not an implicit inspection layer. |
| Hybrid model/GPU control plane | Versioned Runtime Adapter contracts and read-only schemas exist; first-class persisted Compute and Deployment APIs do not yet ship. | **Hardening** for the immediate inventory chain, then **evidence-gated** lifecycle and placement. Hosted Providers remain first-class and no GPU is required. |

## Identity boundaries

A **Client/Harness profile** explains how a caller reaches ModelPort: base URL,
supported ingress protocol, key placement, logical model, and known compatibility.
Claude Code, Codex CLI, Qwen Code, and SDK snippets belong here. Profiles do not
own upstream credentials, health, billing, or route eligibility.

A **Provider** identifies the upstream connectivity, credential, account,
commercial, and trust boundary. A **Model** identifies capabilities and pricing
within or across Providers. DeepSeek therefore remains a Provider/model family;
it becomes a Harness only if a distinct DeepSeek caller contract is introduced
and verified. Runtime Adapters, Compute, and Deployments are operational
resources and must not be encoded as Provider channels or Harness metadata.

## Plane ownership and deployment boundary

The current data plane remains one Rust process. It owns request parsing,
authentication, policy/quota admission, model resolution, credential selection,
routing, Provider calls, bounded streaming, fallback, and attempt finalization.
The control plane owns typed desired configuration, identities, credentials by
reference, catalog/capability facts, budgets, Runtime Adapter registration,
observed Compute, desired/observed Deployments, audit, and operator UX.
PostgreSQL is shared persistence, not a second routing authority; the Dashboard
is a client of the control plane.

No service split follows from these logical ownership boundaries. A future
control-plane/data-plane split requires an ADR with measured contention or
availability need, configuration distribution and staleness semantics, secret
delivery, failure isolation, migration/rollback, and equivalent acceptance
evidence. The default must continue to support the single-host deployment.

## Ordered delivery and dependencies

```text
#25/#26 snapshot persistence
        -> #29 bounded authenticated collection
        -> #30 read-only admin API
        -> #31 Compute/GPU dashboard
        -> desired/observed Deployment + manual reconciliation
        -> policy-bounded placement
```

1. **Snapshot persistence (#25/#26):** atomically retain the latest accepted
   adapter observation with stable identity, provenance, collection time, and
   schema version; restart must not fabricate freshness.
2. **Bounded collection (#29):** authenticate adapters, bound concurrency,
   duration and payload size, validate before acceptance, and retain the last
   good snapshot plus explicit error/stale state.
3. **Read-only admin API (#30):** expose server-derived freshness and provenance
   under existing admin authorization. It cannot trigger collection or runtime
   mutation.
4. **Compute/GPU dashboard (#31):** render empty, unavailable, stale, partial,
   and fresh states from the API without becoming an inventory authority.
5. **Deployment lifecycle:** only after inventory acceptance, introduce typed
   desired/observed state, idempotent manual reconciliation, bounded operations,
   conflict handling, rollback, and durable evidence.
6. **Policy-bounded placement:** only after lifecycle gates pass, evaluate a
   recommendation first. Mutation must remain constrained by tenant, model,
   capacity, egress, budget, maintenance, and approval policy.

## Priorities and measurable gates

| Milestone | Exit evidence required before advancing |
| --- | --- |
| Activation and Provider onboarding | Beta activation gates in the Roadmap pass; at least 80% of clean Tier 1 installs reach a governed request within 30 minutes. Every enabled production route has a resolved credential test state and dated non-stream/stream/Tool Use capability evidence for the combinations it claims. |
| Explainable resilience and exact cost | Every sent attempt records candidate, exclusion/fallback reason, outcome, usage source, rate-card version when used, and cost confidence. No estimated amount is labelled actual/billable. Mock acceptance covers each retry class; paid upstream checks remain explicit. |
| Content-free telemetry | A schema review shows no prompt, response, tool argument/result, authorization, credential, or raw body fields. Trace propagation, bounded attributes/cardinality, sampling, tenant authorization, retention, and exporter failure behavior pass local acceptance with export off by default. |
| Compute inventory | Restart, stale/unavailable, oversized/invalid observation, adapter timeout, identity conflict, and last-good-snapshot cases pass. Two design-partner environments produce stable read-only observations for two weeks without runtime mutation. |
| Deployment lifecycle | Repeated reconcile is idempotent; crash/restart, conflict, timeout, partial failure, manual rollback, and audit evidence pass. At least two design partners complete 20 manual lifecycle operations each with no orphaned allocation or unexplained state. |
| Placement experiment | 100 shadow recommendations across at least two environments satisfy policy and capacity constraints and are explainable; operators accept at least 90%, and zero recommendation mutates a runtime. Enabling bounded mutation requires a separate reviewed decision and rollback drill. |

Activation, Provider evidence, explainable resilience, content-free telemetry,
and exact cost provenance precede semantic routing, semantic cache, broad
protocol expansion, or online learning even if those features are available in
other gateways.

## Cache and guardrail extension contract

Caching and guardrails may be added only as explicit, opt-in policy components.
Each proposal must define:

- tenant/project isolation and authorization for configuration and stored data;
- which request/response fields are inspected or retained, encryption, maximum
  size, TTL, deletion, legal hold interaction, and backup behavior;
- a deterministic per-request bypass that is recorded in route evidence;
- ordering relative to protocol conversion, policy, routing, retries, streaming,
  accounting, and response delivery;
- timeout and overload behavior plus an explicit fail-open or fail-closed mode;
- cache key canonicalization, model/Provider/policy/version partitioning,
  invalidation, usage/cost attribution, and prevention of cross-tenant hits; and
- guardrail version, verdict provenance, mutation/deny semantics, false-positive
  rollback, and whether inspected content can leave the trusted host.

No cache may make a stale policy decision reusable. No asynchronous guardrail
may be described as enforcement. Semantic cache additionally requires a threat
model for embeddings, similarity thresholds, poisoning, nondeterminism, and
false-match measurement. Until those contracts and evidence exist, ModelPort
does not claim either capability ships.

## Deliberate non-goals

- Public relay/reseller billing, channel resale, recharge codes, payment
  processing, opaque group/model multipliers, or presenting ModelPort estimates
  as a Provider invoice.
- A ModelPort-hosted service, public multi-tenancy, enterprise/HA readiness, or
  competition based on Provider count.
- Conflating Client/Harness profiles, Providers, credentials, Models, Runtime
  Adapters, Compute, Deployments, or Routes.
- Silent protocol or capability downgrade, generic pass-through that bypasses
  policy, or compatibility claims without dated evidence.
- Prompt, response, tool argument/result, or credential content in telemetry.
- Semantic routing, semantic cache, broad protocol expansion, or online
  learning before the earlier gates.
- Automatic GPU/runtime mutation before inventory and reconciliation evidence;
  model inference inside the gateway; or requiring a particular runtime.

## Decision rule

A parity proposal must identify the design-partner problem, typed resource
owner, plane owner, privacy and failure contract, cost provenance, measurable
gate, rollback, and displaced roadmap work. If it cannot, it remains deferred.
