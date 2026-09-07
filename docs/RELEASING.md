# Releasing ModelPort

ModelPort uses semantic versions shared by the Rust backend and dashboard. A
release tag is `v<version>`, matching `Cargo.toml` and
`dashboard/package.json`. SemVer prerelease suffixes such as `-rc.1` are
supported. Build-metadata suffixes such as `+build.1` are intentionally rejected
because `+` is not valid in the corresponding Docker image tag.

Small-Team Beta targets one planned release every four weeks. Critical security
fixes may publish between planned releases. Only the latest Beta is maintained,
with a 30-day upgrade window for the previous Beta. There is no LTS or SLA.

## Release Preconditions

1. The worktree is clean and the release commit is on protected `main`.
2. `CHANGELOG.md` describes user-visible behavior, security changes, breaking
   storage/configuration changes, and remaining limits.
3. `scripts/check-all.sh`, PostgreSQL-backed dashboard E2E, dependency audit,
   CodeQL, dependency review, and Scorecard checks are green.
   Any temporary dashboard audit exception must still match its documented
   deployment exposure and remain unexpired.
4. A clean PostgreSQL migration and historical-row preservation have been
   verified.
5. Documentation, configuration examples, Compose, systemd, and dashboard use
   the same versioned contract.
6. Any real-Provider claim has a dated, secret-free, commit-bound evidence
   artifact. Routine release CI makes no paid Provider calls.
7. The isolated OIDC/protocol/load/database-fault/restore gate passes, including
   rollback to the attested v0.1.1 binary. Evidence is stored by CI as
   `isolated-production-acceptance`; this does not replace deployment-specific
   Provider, MFA, capacity and RTO/RPO evidence.
8. The version in `deploy/release/compose.yml` matches the tag, and Linux x86_64
   install, safe stop, backup/restore, upgrade, and rollback acceptance passes.

## Version And Tag

Update:

- `Cargo.toml`;
- `dashboard/package.json` and its lockfile;
- both default image versions in `deploy/release/compose.yml`;
- the changelog comparison links and release section.

Then run:

```bash
scripts/check-all.sh
git diff --check
git tag -s vX.Y.Z -m "ModelPort vX.Y.Z"
git push origin main vX.Y.Z
```

A signed tag is preferred. If the maintainer cannot sign tags, document that
fact in the release notes rather than implying signature verification.

## Automated Outputs

The release workflow:

- rejects a tag that does not match the backend, dashboard, and dashboard
  lockfile versions;
- invokes the complete reusable CI workflow, including the explicit
  PostgreSQL legacy-row migration test, state revision concurrency/atomicity
  tests, transactional ledger test, and PostgreSQL-backed dashboard E2E;
- prevents binary or container publication until that verification succeeds;
- builds the Linux amd64 backend archive;
- emits SHA-256 checksums and an SPDX JSON SBOM;
- creates GitHub build-provenance and SBOM attestations;
- publishes versioned backend and dashboard images to GHCR;
- publishes Linux x86_64 container SBOMs, signs immutable image digests with
  keyless Cosign, and attaches GitHub provenance/SBOM attestations;
- records all three immutable image references as Release assets;
- creates the GitHub Release from the existing tag.

The tag must resolve to a commit on protected `main`. Publication first creates
a draft and uploads all assets, checks the asset count, then publishes it under
the repository's immutable-release policy. Do not delete or retag a failed
published version; issue a new patch version after fixing its cause.

Release workflows use least-privilege job permissions and pin third-party
Actions to complete commit SHAs.

## Verification

Consumers should verify checksums and GitHub attestations:

```bash
sha256sum --check SHA256SUMS
gh attestation verify model-port-vX.Y.Z-linux-amd64.tar.gz \
  --repo tiammomo/ModelPort
docker pull ghcr.io/tiammomo/modelport:X.Y.Z
cosign verify \
  --certificate-identity-regexp='https://github.com/tiammomo/ModelPort/.github/workflows/release.yml@refs/tags/vX[.]Y[.]Z' \
  --certificate-oidc-issuer=https://token.actions.githubusercontent.com \
  ghcr.io/tiammomo/modelport@sha256:<digest>
```

For container provenance, verify the immutable digest rather than relying only
on a mutable tag.

The workflow and repository changes only make a release candidate ready. They
do not make a tag, GHCR image, signature, or GitHub Release exist until an
authorized maintainer pushes the reviewed tag and the workflow succeeds. Never
document a prebuilt path as available before those external artifacts exist.

## Rollback

Application rollback and database rollback are separate decisions.

- Keep the previous image digest, binary, configuration, and database-native
  backup until production acceptance passes.
- Never point an older release at a database after a migration unless that
  downgrade path was explicitly tested.
- The operational migrations preserve historical request/attempt records and
  versioned auth/control state. Roll back by restoring the reviewed database
  snapshot and compatible application together; do not infer downgrade safety
  from a successful forward migration.
- The PostgreSQL 18 Compose baseline uses a new
  `modelport_modelport-postgres-18` volume and the versioned
  `/var/lib/postgresql/18/docker` data directory. It intentionally does not
  reuse the old PostgreSQL 16 volume. Back up the old deployment before
  upgrading, follow [the migration runbook](POSTGRESQL_MIGRATION.md), and do not
  delete its volume until restore acceptance passes.

## Repository Settings

Maintainers must enable private vulnerability reporting, Dependabot alerts and
security updates, immutable releases where available, tag protection/rulesets,
and required status checks. Repository settings are external state and cannot
be guaranteed by workflow files alone.
