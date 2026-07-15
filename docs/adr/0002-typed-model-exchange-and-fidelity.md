# ADR-0002: Typed Model Exchange And Protocol Fidelity

- Status: Accepted
- Date: 2026-07-15

## Context

At decision time, the request pipeline used `AnthropicRequest` as its internal
boundary. This was effective for Claude clients, but adding an OpenAI client endpoint by
converting OpenAI to Anthropic and then back to an OpenAI-compatible Provider
would discard or distort protocol-native semantics.

The target product must support Anthropic Messages, OpenAI Chat Completions,
OpenAI Responses, and later workload-specific APIs without turning conversion
code into pairwise protocol special cases.

## Decision

Introduce a typed internal exchange model between client adapters, governance,
routing, and Provider adapters.

The model contains, as first-class types:

- ordered input and output items with stable identifiers;
- text and common Tool Use content for the first migration slice;
- system, developer, user, and assistant intent;
- tool definitions, calls, results, call IDs, parallelism, and schema strictness;
- structured-output constraints, stop causes, usage, and stream lifecycle;
- capability requirements and namespaced opaque extensions.

Images, audio, document references, refusals, reasoning references, and other
typed items are added without flattening them into strings. Hidden reasoning is
never normalized into exposed text; opaque encrypted or Provider-native items
may be preserved when policy allows it.

Each protocol adapter performs four operations:

1. Parse a client contract into the exchange model.
2. Declare required capabilities and potential fidelity loss.
3. Render and parse the selected Provider protocol.
4. Render the original client protocol, including typed stream events and
   normalized errors.

Fidelity modes are explicit:

- `strict`: reject before the Provider call if meaning cannot be preserved;
- `compatible`: allow documented equivalent mappings but reject known loss;
- `best-effort`: allow documented loss or repair and record a fidelity report.

Unknown fields are not copied blindly across trust boundaries. Protocol-native
extensions require a namespace, capability declaration, and policy decision.

## Consequences

- `/v1/messages` is migrated through the exchange model before adding public
  `/v1/chat/completions`.
- OpenAI Chat and Anthropic Providers can be tested in all four client/Provider
  combinations without pairwise handler logic.
- OpenAI Responses uses typed Items and typed SSE; it is not modeled as a renamed
  Chat Completions request.
- Capability gating happens before budget reservation and Provider egress.
- Golden fixtures and a fidelity matrix become part of Provider conformance.

## Implementation status

The first text/function-tool slice is implemented through `/v1/messages` and
`/v1/chat/completions`, including cross-protocol non-stream/stream rendering,
capability/fidelity rejection, normalized usage, and terminal lifecycle tests.
Responses items, multimodal content, structured output beyond text, native
extensions, and the complete conformance matrix remain future work.

## Rejected alternatives

- Keep Anthropic as the canonical internal protocol: blocks lossless OpenAI and
  Responses features.
- Use untyped `serde_json::Value` everywhere: makes invariants, policy, and
  compatibility failures runtime-only and hard to review.
- Implement a lowest-common-denominator schema: silently removes product value
  from richer protocols.
