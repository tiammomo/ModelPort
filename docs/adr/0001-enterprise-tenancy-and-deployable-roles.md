# ADR-0001: Enterprise Tenancy And Deployable Roles

- Status: Accepted
- Date: 2026-07-15

## Context

The current control plane has users, teams, and API keys, but there is no
durable tenant boundary. Sessions, rate limits, stream permits, and parts of
runtime health are process-local. Treating a team as a tenant would overload a
billing/policy grouping with ownership and isolation semantics it was not
designed to provide.

ModelPort also runs the data plane, control plane, and background behavior in
one process. Immediately splitting the project into microservices would add
network and deployment failure modes before the internal boundaries are ready.

## Decision

The durable ownership hierarchy is:

```text
Organization -> Project -> Environment
```

- Organization is the tenant and top-level security, identity, Provider, and
  governance boundary.
- Project is the application, budget, virtual-model, route, and workload-policy
  boundary.
- Environment is an optional subdivision for production, staging, development,
  or equivalent deployment contexts.
- Team remains a grouping concept during migration and does not become the
  tenant identifier implicitly.

Tenant scope is derived from the authenticated principal and credential. An
arbitrary client header cannot select an organization or project.

The Rust codebase remains a modular monolith. Internal modules must allow the
same binary to run in these roles later:

- `all-in-one` for development and compatibility;
- `data-plane` for stateless inference traffic;
- `control-plane` for identity, configuration, and administration;
- `worker` for reconciliation, retention, probes, exports, and rollups.

PostgreSQL is authoritative across roles. A process cache may use a versioned
last-known-good policy and route snapshot for a bounded interval, but revocation,
tenant isolation, and budget exhaustion default to fail closed.

## Consequences

- Every tenant-owned repository method requires an explicit `TenantScope`.
- Every data-plane request carries a `RequestContext` containing tenant,
  principal, protocol, request, and trace identifiers.
- Cross-tenant negative tests are required for every resource type.
- Existing local deployments map to a reserved local organization, project, and
  environment during migration.
- Role separation is an internal boundary first and a deployment option later;
  no domain logic is duplicated between roles.

## Rejected alternatives

- Team as tenant: insufficient ownership and isolation semantics.
- Tenant selected by request header: vulnerable to confused-deputy and
  cross-tenant access mistakes.
- Microservices immediately: premature operational complexity without stable
  domain and consistency boundaries.
