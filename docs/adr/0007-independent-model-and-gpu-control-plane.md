# ADR-0007: Independent Model And GPU Control Plane

- Status: Accepted
- Date: 2026-08-23
- Supersedes: the local model, artifact, runtime, and GPU ownership decision in
  [ADR-0005](0005-forty-user-hybrid-routing-baseline.md)

## Context

ModelPort v0.1.x ships a governed single-process gateway and a separate
operations Dashboard. It already owns client authentication, Provider and
model resolution, policy, routing, quota, cost evidence, health, and the
operational ledger. Hosted API Providers and local OpenAI-compatible runtimes
can both serve requests through that boundary.

The first local Qwen integration assigned model artifacts, runtime operation,
GPU state, and acceptance evidence to a particular `local-inference-stack`
checkout. That was useful for one deployment rehearsal, but it made an
external repository layout look like a permanent architecture dependency. It
also left model inventory, compute capacity, and deployment lifecycle without
clear ModelPort resource ownership.

ModelPort needs to grow without turning one integration into the product
model. Hosted APIs will remain first-class, new API Providers will be added,
and local inference engines must remain replaceable. Claude, Codex, DeepSeek,
SDKs, and internal applications are clients of ModelPort; their names must not
become Provider or deployment types.

## Decision

### Control-plane ownership

ModelPort is an independent hybrid model and GPU control plane. It owns the
desired state, observed inventory, policy, and evidence for the following
resources:

| Resource | Stable responsibility |
| --- | --- |
| Client/Harness | A caller and protocol edge, such as Claude, Codex, DeepSeek tooling, an SDK, or an internal application. It never owns Provider credentials or routing truth. |
| Provider | A governed connectivity, credential, trust, and commercial boundary. A Provider may be a hosted API or an endpoint backed by a local Deployment. |
| Model | A provider-independent catalog identity plus reviewed capabilities, limits, compatibility, and optional rate-card metadata. A model record does not prove that it is deployed or usable. |
| Runtime Adapter | A versioned contract that discovers and controls an external inference runtime. It translates lifecycle and inventory operations; it is not the runtime itself. |
| Compute Node/GPU | Observed capacity and health for a managed host and its devices. Desired labels and admission policy belong to ModelPort; driver and hardware facts remain observations. |
| Deployment | The desired and observed binding among a Model, Runtime Adapter, Compute Node/GPU allocation, endpoint, and lifecycle state. |
| Route | The client-facing logical selection policy that chooses eligible Provider/model or Deployment-backed candidates and records the decision. |

The inference engine remains out of process. llama.cpp, vLLM, Ollama, and
other runtimes own model execution, device-specific process mechanics, and
runtime-native caches. ModelPort must not link their engines into its gateway
process or make their repository directory layout part of a core contract.

### Adapter boundary

A local integration enters ModelPort through an authenticated, versioned
Runtime Adapter contract. The contract must distinguish desired state,
observed state, and immutable execution evidence. Mutating operations must be
idempotent and bounded; inventory reads must not implicitly download a model,
start a runtime, or change GPU state.

The current local Qwen configuration is one reference adapter and acceptance
example. `local-inference-stack` is not a required dependency, authoritative
inventory, release input, or cross-repository source of truth. No ModelPort
feature may require that repository's checkout path, scripts, environment
variables, or internal file formats. Existing compatibility helpers are
temporary migration surfaces and will be generalized or removed in a focused
follow-up.

### Hosted and local Providers

Hosted APIs and local Deployments share the Provider, Model, Route, policy,
health, and evidence boundaries. They are not flattened into identical
resources:

- hosted Providers own remote credentials, rate limits, regions, published
  models, and externally reported usage or cost;
- local Deployments own Runtime Adapter, Compute Node/GPU allocation,
  artifacts, endpoint lifecycle, and locally observed capacity;
- a Route can compare only candidates whose capability, data policy, health,
  and budget gates pass;
- the control plane remains useful when a deployment has only hosted
  Providers and no managed GPU.

### Product and code domains

Backend modules and Dashboard navigation will converge on eight product
domains: Models, Providers, Compute, Deployments, Routing, Governance,
Observability, and Operations. The browser remains a client of versioned
control-plane APIs and never becomes a second source of truth.

This is a staged modular-monolith evolution. It does not authorize a
microservice split, a second public gateway, or direct Dashboard access to
runtime agents.

### Delivery sequence

1. Publish this resource model and distinguish current behavior from target
   behavior in the Architecture and Roadmap.
2. Remove normative `local-inference-stack` coupling and define a generic,
   read-only Runtime Adapter capability contract with the local Qwen path as a
   reference fixture.
3. Add persisted, read-only Compute Node/GPU inventory with freshness and
   provenance evidence.
4. Add a Deployment resource and explicit lifecycle reconciliation without
   automatic placement.
5. Add policy-bounded placement and capacity decisions only after inventory,
   rollback, concurrency, and failure evidence are accepted.

Each stage must be independently useful and keep hosted API Providers working.

## Consequences

- ModelPort can manage local and remote capacity without depending on a second
  product repository.
- Models, GPU devices, and running deployments become separate resources
  instead of fields inferred from a Provider URL.
- The Dashboard can grow around stable product domains rather than accumulating
  unrelated settings panels.
- Runtime integrations incur a versioned contract and reconciliation burden;
  arbitrary shell hooks are not an acceptable substitute.
- GPU scheduling, artifact download, and runtime mutation remain unimplemented
  until their own reviewed Issues and acceptance evidence land.
- The privacy, policy, protocol-fidelity, ledger, and single-gateway decisions
  in ADR-0005 remain in force.

## Rejected Alternatives

- Keep `local-inference-stack` as the model/GPU source of truth: preserves a
  cross-repository dependency and prevents other runtime adapters from being
  first-class.
- Embed a preferred inference engine in ModelPort: couples gateway releases to
  GPU drivers and runtime internals and expands the trusted process boundary.
- Treat a running Provider endpoint as a Deployment: loses desired state,
  compute allocation, lifecycle, and provenance.
- Treat Claude, Codex, or DeepSeek Harness as Providers: mixes callers with
  upstream execution and recreates ambiguous routing ownership.
- Build an automatic scheduler before read-only inventory: makes irreversible
  placement decisions without trustworthy capacity or rollback evidence.
