# PostgreSQL Migration And Cutover

This runbook covers the phase-one single-ModelPort deployment. It is designed
for a controlled move from the local Compose PostgreSQL database to a managed
PostgreSQL service, and for a local PostgreSQL major-version change. It does
not perform a migration automatically.

Never point PostgreSQL 18 directly at a PostgreSQL 16 data directory. Major
versions require a reviewed logical dump/restore or a separately designed
`pg_upgrade` workflow. ModelPort uses logical dump/restore because it also
works with a managed target and keeps the source database available for
rollback.

## Current drift guard

Before any full Compose update, run:

```bash
./scripts/database-preflight.sh
```

The command is read-only and does not print credentials. It fails if the
running PostgreSQL major version or data volume differs from
`docker-compose.yml`, if SQLx records a failed migration, or if durable state
rows are incomplete. Use `./scripts/compose-up.sh` for routine local Compose
updates so this guard runs before an existing database can be replaced.

A preflight failure is a stop condition. `docker compose up -d` must not be
used to work around it.

## Target requirements

For the forty-user target, prefer an operator-managed PostgreSQL service with:

- TLS hostname verification and an explicitly trusted CA;
- point-in-time recovery with RPO no greater than five minutes;
- a tested RTO no greater than thirty minutes;
- restricted ModelPort and migration identities;
- encrypted backups in a different failure domain;
- connection, storage, backup, and replication alerts.

Create a new empty target database. Do not restore over an existing production
database, and do not reuse the source database credentials.

## Rehearsal

1. Record the source ModelPort revision, image digest, PostgreSQL version,
   migration list, database size, and incident contacts.
2. Create a new schema-v2 backup:

   ```bash
   archive="$(./scripts/backup-compose.sh create)"
   ./scripts/backup-compose.sh verify "$archive"
   ./scripts/backup-compose.sh drill "$archive"
   ./scripts/backup-compose.sh upgrade-drill "$archive"
   ```

   Schema-v2 archives contain the database dump and deployment provenance but
   no `.env` or `config.toml`. Recover configuration from a reviewed Git
   revision and credentials from the secret manager.
   `upgrade-drill` requires an isolated PostgreSQL 18 target and reports the
   source and target versions; it never connects ModelPort to the target and
   never changes the live source database.
3. Restore the dump into an isolated target database with the target
   PostgreSQL `pg_restore`, `--exit-on-error`, `--no-owner`, and
   `--no-privileges`.
4. Start the candidate ModelPort revision against only that isolated target.
   Let embedded SQLx migrations finish before sending requests.
5. Compare source and target counts for `_sqlx_migrations`,
   `modelport_state`, `modelport_gateway_requests`,
   `modelport_provider_attempts`, budget/evidence tables, and incomplete
   leases. Compare aggregates, not Prompt or response content.
6. Run authenticated readiness, dashboard, backup restore, protocol, Tool Use,
   and the acceptance suite for each configured local Runtime Adapter. Existing
   Qwen reference deployments may still use the optional compatibility suite
   documented in [Local Qwen reference adapter](LOCAL_INFERENCE_STACK.md).
7. Destroy the rehearsal target only after saving secret-free evidence.

## Single-instance cutover

The current phase has one ModelPort instance, so cutover requires a maintenance
window:

1. Announce the window and stop new client traffic at the reverse proxy.
2. Wait for active streams and tool conversations to finish; do not replay a
   started stream on another Provider.
3. Stop only ModelPort and the dashboard. Keep the source PostgreSQL database
   running and unchanged.
4. Create and verify the final schema-v2 backup.
5. Restore the final dump to a new empty target database and repeat the row,
   migration, readiness, and acceptance checks.
6. Render short-lived runtime credentials from the secret manager. The
   database URL must use `verify-full` and the CA path mounted as
   `/run/modelport/database-ca.pem`.
7. Start [the single-instance production Compose profile](../deploy/production/compose.single.yml)
   with digest-pinned images.
8. Re-enable traffic gradually and watch readiness, errors, ledger finalizers,
   latency, Provider health, and PostgreSQL connections.

## Rollback

Keep the source database and previous ModelPort image immutable through the
rollback window. If validation fails:

1. stop client traffic;
2. stop the candidate ModelPort instance;
3. restore the previous secret reference and database endpoint;
4. start the previous digest-pinned ModelPort image against the untouched
   source database;
5. run readiness and a synthetic request before reopening traffic.

Do not attempt to copy writes from the failed target back into the source. A
cutover is complete only after the rollback window closes and backup/restore
evidence for the target has passed.

## Legacy backups

Schema-v1 archives created by older `backup-compose.sh` versions contain
plaintext runtime configuration and may contain Provider and database
credentials. Verification emits a warning. Keep them at permission `0600`,
restrict access, rotate affected credentials, and delete them only under the
organization's approved retention procedure.
