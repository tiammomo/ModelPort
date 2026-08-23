# Runtime Adapter Contracts

ModelPort publishes versioned, read-only discovery and Compute Node/GPU
observation contracts for external inference runtimes. The shipped v1alpha1
artifacts include wire contracts, offline validators, and a reusable
authenticated collection client. They do not expose a persisted inventory API,
reconciler, or mutation endpoint.

## Contract Files

- `schemas/runtime-adapter-capabilities-v1alpha1.schema.json` is the normative [JSON Schema 2020-12](https://json-schema.org/draft/2020-12) document.
- `schemas/runtime-adapter-compute-inventory-v1alpha1.schema.json` defines the normative response from `inventory.compute.list`.
- `src/runtime_adapter.rs` contains the matching public Rust types and semantic validators.
- `fixtures/runtime-adapters/qwen-llama-cpp-*-v1alpha1.json` are reference fixtures; Qwen and llama.cpp are not special resource types.

Validate the reference or another local document without contacting a runtime:

```bash
scripts/runtime-adapter-check.sh
scripts/runtime-adapter-check.sh --document path/to/capabilities.json --json
scripts/runtime-adapter-check.sh --document path/to/compute-inventory.json --json
```

The equivalent binary command is
`model-port runtime-adapter validate path/to/document.json --json`. The
validator dispatches only the two recognized `kind` values and rejects an
unknown resource kind.

## Authenticated Collection Boundary

Library consumers can construct `RuntimeAdapterClientConfig` with an adapter
ID, origin URL, and [RFC 6750](https://www.rfc-editor.org/rfc/rfc6750) Bearer
credential, then call `RuntimeAdapterClient::collect_compute_inventory`. The
client always reads and validates capabilities before the Compute snapshot,
requires the advertised `inventory.compute.list` operation, and binds both
documents to the configured adapter identity.

Only HTTPS origins and literal loopback HTTP origins are accepted. Paths are
fixed by the v1alpha1 contract; redirects remain disabled and the shared HTTP
transport provides DNS pinning, timeouts, and bounded response bodies. The
credential is not serializable and is redacted from debug output and upstream
errors. Configuration-file loading, scheduling, and storage are intentionally
outside this client boundary.

## Capability Rules

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

## Compute Inventory Rules

`RuntimeAdapterComputeInventory` is the response document for the advertised
side-effect-free `inventory.compute.list` operation. Metadata binds each
snapshot to an `adapterId`, opaque `snapshotId`,
[RFC 3339](https://www.rfc-editor.org/rfc/rfc3339.html) `observedAt`, and
collector version/revision provenance. Date-time format validation is enabled
as an assertion rather than treated as a schema annotation.

Node identity follows the distinction between OpenTelemetry
[`host.id` and `host.name`](https://opentelemetry.io/docs/specs/semconv/resource/host/):
`nodeId` is stable, while optional `hostName` is descriptive and may change.
`idSource` records whether the stable value came from a machine ID, cloud
instance ID, or explicit operator assignment.

Each `gpuId` is unique across the snapshot and records an `idSource`. Prefer a
vendor device or partition UUID. [NVIDIA NVML](https://docs.nvidia.com/deploy/nvml-api/group__nvmlDeviceQueries.html)
exposes GPU UUID lookup, and
[AMD SMI](https://rocm.docs.amd.com/projects/amdsmi/en/develop/conceptual/partition.html)
exposes partition UUIDs on current ROCm versions. A PCI address may be reported
as an observation, but neither a PCI address nor an enumeration index is an
accepted identity source. A partition must reference a physical GPU on the
same node. Available memory cannot exceed total memory, and both are integer
byte counts.

Node/device `health` reports the state observed at `observedAt`; it does not
report snapshot freshness. The document cannot contain `fresh` or `stale`.
ModelPort will derive `fresh`, `stale`, or `unavailable` from the accepted
observation time and server-owned policy when persistence is implemented.
Extensions are limited to bounded primitive values or bounded primitive arrays
so they cannot become an unreviewed nested protocol.

## Compatibility And Evolution

Consumers must reject an unsupported `apiVersion`; fields are not silently
reinterpreted across versions. Additive experimental data belongs in
`extensions` until a reviewed contract version defines it. The historical
`local-inference-stack` checker remains an explicitly selected compatibility
mode, not the source of this contract.

Configuration integration, collection policy, persistence, derived freshness,
admin APIs, and all writes remain deferred to reviewed Issues. Offline
validation cannot start a process, download a model, access a GPU, or call a
network endpoint; the collection client performs only the two advertised safe
reads requested by its caller.
