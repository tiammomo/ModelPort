# Upgrading And Rollback

ModelPort v0.1.x is a single-instance beta. It promises a predictable,
evidence-preserving maintenance window, not zero-downtime or rolling upgrades.
Application and database rollback are separate decisions and may need to be
performed together.

## Release Inputs

Before the window, record:

- current and target version, Git commit, backend/dashboard image digests, and
  source state;
- current configuration revision and secret-render timestamp, without copying
  secret values;
- current PostgreSQL version and applied migration set;
- backup archive, verification result, restore-drill result, and owner;
- Provider/Tool Use acceptance scope and the rollback decision owner.

Download `SHA256SUMS`, both `.digest` files, and the SPDX JSON SBOMs from the
GitHub Release. Verify the release before editing deployment state:

```bash
sha256sum --check SHA256SUMS
gh attestation verify model-port-v0.1.1-linux-amd64.tar.gz \
  --repo tiammomo/ModelPort
gh attestation verify \
  oci://ghcr.io/tiammomo/modelport@sha256:<backend-digest> \
  --repo tiammomo/ModelPort
cosign verify \
  --certificate-identity-regexp='https://github.com/tiammomo/ModelPort/.github/workflows/release.yml@refs/tags/v0[.]1[.]1' \
  --certificate-oidc-issuer=https://token.actions.githubusercontent.com \
  ghcr.io/tiammomo/modelport@sha256:<backend-digest>
```

The binary archive includes `Cargo.lock`; its SBOM inventories locked workspace
dependencies, including optional and test dependencies. It does not claim that
every listed crate is linked into the gateway executable. Container SBOMs also
cover their runtime image contents.

Repeat the image verification for `modelport-dashboard` and, when enabled,
`modelport-ops-agent`. Verification proves
release provenance; it does not prove that a Provider account or model remains
compatible.

## Preflight And Backup

Select the manifest used by the running deployment before any Compose or helper
command. The normal release profile is:

```bash
export MODELPORT_COMPOSE_FILE="$PWD/deploy/release/compose.yml"
```

1. Read `CHANGELOG.md`, [Compatibility](COMPATIBILITY.md), and known limits.
2. Render the candidate Compose file with synthetic secrets. Do not print a
   real runtime secret file into CI or an incident log.
3. Run the applicable preflight:

   ```bash
   scripts/config-validate.sh
   scripts/doctor.sh
   scripts/database-preflight.sh
   ```

4. Create, verify, and restore-drill a new PostgreSQL backup:

   ```bash
   scripts/backup-compose.sh create
   scripts/backup-compose.sh verify backups/modelport-<UTC>.tar.gz
   scripts/backup-compose.sh drill backups/modelport-<UTC>.tar.gz
   ```

   This helper is only for the bundled Compose PostgreSQL service. For the
   external-database production profile, use the managed database's native
   point-in-time backup and isolated restore drill, and record equivalent
   evidence before continuing.

5. Keep the previous image digests, configuration, secret version, and backup
   until the post-upgrade observation window closes.

Never use `docker compose down -v`, reuse a PostgreSQL data directory across
major versions, or point an older binary at a newly migrated database without
an explicitly tested downgrade path.

## Safe Maintenance Upgrade

Set the target images to the exact release digests in the shell or the
operator-owned deployment environment:

```bash
export MODELPORT_IMAGE='ghcr.io/tiammomo/modelport@sha256:<backend-digest>'
export MODELPORT_DASHBOARD_IMAGE='ghcr.io/tiammomo/modelport-dashboard@sha256:<dashboard-digest>'
# Required only when the optional Compose profile is enabled.
export MODELPORT_OPS_AGENT_IMAGE='ghcr.io/tiammomo/modelport-ops-agent@sha256:<agent-digest>'
export MODELPORT_PULL_POLICY=always
```

Then:

1. Announce the maintenance window and stop clients from starting new work.
2. Leave the Dashboard running as a static diagnostic entry point and stop the
   backend through Compose:

   ```bash
   docker compose -f "$MODELPORT_COMPOSE_FILE" stop modelport
   ```

   Compose sends SIGTERM. ModelPort stops accepting new connections, waits for
   active HTTP bodies, then drains ledger finalizers. The default
   `stop_grace_period` is 11 minutes: the 600-second request timeout plus the
   30-second finalizer drain and margin. An operator may choose a different
   value only after matching it to configured request/stream limits. During the
   stop, Dashboard static assets remain reachable and proxied API routes return
   502.

3. Confirm no backend container is running and record any finalizer-drain or
   forced-termination warning. A forced SIGKILL requires post-start lease and
   Provider reconciliation; it is not a clean upgrade.
4. Pull and recreate the two application containers. Keep PostgreSQL running:

   ```bash
   docker compose -f "$MODELPORT_COMPOSE_FILE" pull modelport dashboard
   scripts/compose-up.sh modelport dashboard
   ```

5. Watch the backend startup migration. Do not repeatedly restart a migration
   failure. Preserve its error and determine whether the existing database is
   unchanged before retrying.
6. Verify process and storage separately:

   ```bash
   curl -fsS http://127.0.0.1:38082/livez
   curl -fsS -H "Authorization: Bearer $MODELPORT_HEALTHCHECK_API_KEY" \
     http://127.0.0.1:38082/readyz
   scripts/smoke-test.sh
   ```

7. Run the Provider/model/Tool Use acceptance that the deployment actually
   promises. Paid synthetic calls require explicit authorization.
8. Check request evidence, estimated cost provenance, pending finalizers,
   reconciled leases, error rate, and latency before reopening traffic.

For the phase-one external-database production template, export the selected
manifest and run the production preflight before the same sequence:

```bash
export MODELPORT_COMPOSE_FILE="$PWD/deploy/production/compose.single.yml"
./scripts/production-preflight.sh
```

The export switches both the raw commands and `scripts/compose-up.sh` to this
profile. The helper skips only the bundled-PostgreSQL volume check for the
external-database profile, creates its declared external network when absent,
and recreates the requested application services.

## Rollback Decision

Stop and choose one path; do not improvise a database downgrade.

### Application-only rollback

Use this only when release notes and migration evidence explicitly confirm the
old binary can use the current database unchanged:

1. Stop new traffic and the target backend through Compose.
2. Restore both previous image digests and the previous configuration/secret
   version.
3. Recreate backend and Dashboard, then repeat readiness, smoke, Provider, Tool
   Use, and ledger acceptance.

### Paired application/database rollback

Use this when a migration is not backward compatible or database state may be
damaged:

1. Stop backend and Dashboard writers.
2. Preserve a forensic dump of the failed candidate database.
3. Restore the verified pre-upgrade PostgreSQL backup to an isolated database
   and validate it.
4. Point the previous backend/dashboard digests and previous configuration at
   the restored database together.
5. Run the full acceptance path before reopening traffic.

Requests accepted after the backup point are not present in the restored
database. Reconcile them with external Provider evidence and append-only
adjustments; never rewrite older cost/usage rows to make totals align.

## Closeout

Record the final version/digests, migration set, backup retained, acceptance
results, maintenance duration, forced terminations, and every unreconciled
request. Retain the rollback point for at least the latest Beta support window
described in [Support](../.github/SUPPORT.md).
