# ADR-0003: Relational State And Transaction Boundaries

- Status: Accepted
- Date: 2026-07-15

## Implementation status

The first two expand steps are implemented: SQLx/Tokio/rustls, bounded pools,
embedded migrations `0001_enterprise_foundation.sql` and
`0002_idempotency_and_leases.sql`, normalized tenant parents, request/Provider-
attempt lifecycle rows, hashed idempotency claims, renewable instance leases,
and expired-row reconciliation. Compatibility auth/control documents remain in
place. Current estimated cost is stored as rounded USD micro-units; response
replay, authoritative decimal price-book settlement, Provider evidence
ingestion, and the append-only adjustment model below remain required before
billing can be considered exact.

## Context

At the time of this decision, the file and PostgreSQL storage backends persisted
two complete JSON documents, and PostgreSQL used a synchronous `NoTls` client.
That baseline could not provide efficient tenant-scoped queries, independent
retention, concurrent mutation safety, schema migrations, or atomic budget
accounting. The implementation-status section records the first completed
expand step without rewriting this decision context.

Enterprise accounting must distinguish a client request from its Provider
attempts and must reserve, settle, release, and reconcile spend without silent
historical mutation.

## Decision

Enterprise mode uses normalized PostgreSQL tables through SQLx with Tokio,
Rustls, connection pooling, explicit timeouts, and embedded versioned
migrations. PostgreSQL is the authoritative store. JSON files remain available
only in a development/compatibility profile.

Repository traits isolate domain logic from persistence, but they do not hide
transaction boundaries. Every tenant-owned repository operation requires an
explicit `TenantScope`.

The minimum transaction boundaries are:

- identity, membership, role-binding, and revocation mutation;
- versioned policy/route publication plus audit event;
- request creation plus idempotency claim plus budget reservation;
- Provider-attempt creation and terminal outcome;
- final usage settlement and reservation release;
- append-only usage adjustment and audit export cursor advancement.

Money uses PostgreSQL `NUMERIC` and a Rust decimal type with explicit currency.
Usage records retain price-book revision and evidence source. Historical
settlements are corrected with append-only adjustments, not destructive edits.

Redis may provide distributed rate limits, short leases, cache invalidation, and
ephemeral coordination. Redis is not the authoritative budget or usage ledger.

Migrations follow expand/migrate/contract:

1. Add backward-compatible schema.
2. Deploy code that reads/writes both required versions or backfills explicitly.
3. Verify migration and rollback evidence.
4. Remove legacy structures in a later release.

Legacy document import is explicit, idempotent, checksummed, dry-run capable,
and never performed as an unannounced startup rewrite.

## Consequences

- The synchronous `postgres` JSON document backend is removed from enterprise
  mode after the importer and compatibility window are complete.
- Database integration tests require real PostgreSQL and exercise transaction
  conflicts, tenant isolation, migration, and recovery.
- Usage telemetry may remain best effort only where it cannot affect access or
  accounting; budget and settlement writes fail according to an explicit
  dependency policy.
- Database schema and application version skew become release contracts.

## Rejected alternatives

- Continue storing JSONB documents: does not solve concurrency, query, or
  retention boundaries.
- Use Redis as the budget ledger: insufficient as the authoritative auditable
  transaction store.
- Dual-write indefinitely: creates two sources of truth and unbounded recovery
  ambiguity.
