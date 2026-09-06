# ADR-0008: Team Gateway Focus And Task Navigation

- Status: Accepted
- Date: 2026-09-06
- Amends: ADR-0007 delivery priority and Dashboard navigation; resource ownership is retained.

## Decision

The next delivery priority is reliable model access and egress governance for
20–50 person development teams. Optimize the first governed request, correct
client configuration, and actionable request evidence before expanding the
GPU control plane. The existing Beta activation and privacy gates remain.

Compute inventory follow-ups, Deployment lifecycle, and placement are
evidence-gated. Resume that sequence only when at least two design partners
identify concrete capacity/deployment blockers that their existing runtime
operations cannot reasonably address. Retain the implemented Runtime Adapter
contracts and tests; hosted-only and externally managed local runtimes remain
supported. A blocking client protocol may take priority under the existing
design-partner exception, rather than maximizing Provider/protocol breadth.

The console exposes five administrator task groups: overview, model access,
requests/usage, team/policy, and system. Existing URLs and role-filtered
destinations remain reachable through section navigation and search. The
eight backend resource domains are ownership boundaries, not a requirement
for eight top-level console destinations or separate services.

Project setup uses explicit form fields with local-only execution and unknown
classification as defaults. Cloud authorization remains an audited server-side
policy change. Selecting a Provider must not automatically authorize egress or
classify data. A selected key's setup check gates configuration copying for all
roles and distinguishes configuration eligibility from a completed request.

The operations Agent remains opt-in, including local image builds. Releases
and workspace checks continue to cover the optional component. Preserve
PostgreSQL transactions, quota/budget admission, stream finalization, content
minimization, and restore compatibility while simplifying defaults and UI.

## Consequences

The implementation covers policy forms, key-scoped setup checks, task navigation,
a four-step journey resumed from saved configuration, optional local Agent
builds, and dependency cleanup. Model adaptation owns its draft in a separate
dialog; operations queries are isolated behind the ledger facade. Unsaved
form drafts are not persisted. Further ledger decomposition remains incremental;
no current API claims GPU inventory or Deployment lifecycle ships.
Judge subsequent work by activation time, diagnosis time, sustained team use,
and measured maintenance cost. Do not use removed test lines or feature count
as success metrics.
