# Runtime Adapter Capability Contract

ModelPort publishes a versioned, read-only discovery contract for external
inference runtimes. The shipped v1alpha1 artifact is a contract and offline validator;
it does not expose an Adapter client, inventory API, reconciler, or mutation endpoint.

## Contract Files

- `schemas/runtime-adapter-capabilities-v1alpha1.schema.json` is the normative [JSON Schema 2020-12](https://json-schema.org/draft/2020-12) document.
- `src/runtime_adapter.rs` contains the matching public Rust types and semantic validator.
- `fixtures/runtime-adapters/qwen-llama-cpp-capabilities-v1alpha1.json` is one reference fixture; Qwen and llama.cpp are not special resource types.

Validate the reference or another local document without contacting a runtime:

```bash
scripts/runtime-adapter-check.sh
scripts/runtime-adapter-check.sh --document path/to/capabilities.json --json
```

The equivalent binary command is `model-port runtime-adapter validate path/to/capabilities.json --json`.

## v1alpha1 Rules

Every document has `apiVersion: runtime.modelport.io/v1alpha1`,
`kind: RuntimeAdapterCapabilities`, a stable adapter identity, authentication
and transport requirements, runtime engines, inference protocols, inventory
kinds, and advertised operations. Unknown top-level and structured fields are
rejected; adapter-specific annotations belong only in `spec.extensions`.

The only operation identifiers are capability, health, model inventory,
compute inventory, and deployment inventory reads. Their paths are fixed,
their method is `GET`, and `sideEffectFree` must be `true`. Validation rejects
unknown versions, mutation methods, duplicate operation IDs or paths, and
operation/path mismatches.

`tls_required` is the remote default. `tls_or_loopback` permits a trusted local
security boundary. Both require an advertised bearer-token or mutual-TLS
authentication scheme; the fixture contains no credentials.

## Compatibility And Evolution

Consumers must reject an unsupported `apiVersion`; fields are not silently
reinterpreted across versions. Additive experimental data belongs in
`extensions` until a reviewed contract version defines it. The historical
`local-inference-stack` checker remains an explicitly selected compatibility
mode, not the source of this contract.

Inventory responses, RFC 3339 observation time, freshness, provenance, stable
GPU identity, authenticated transport, persistence, and writes are deferred to
reviewed Issues. Validation cannot start a process, download a model, access a
GPU, or call a network endpoint.
