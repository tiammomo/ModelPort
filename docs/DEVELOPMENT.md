# Development

## Repository Map

Run the commands below from the repository root. The layout follows ownership:

| Path | Responsibility |
| --- | --- |
| `src/` | Gateway application and its colocated Rust unit tests. |
| `crates/` | Operations Agent and shared operations protocol workspace packages. |
| `dashboard/` | Console, frontend tooling, dependency audit policy, and browser tests. |
| `resources/` | Catalog and JSON schemas embedded in the gateway binary. |
| `migrations/` | Ordered PostgreSQL migrations embedded by SQLx. |
| `tests/` | Backend integration tests, script regression tests, and shared fixtures. |
| `scripts/` | Development, verification, and operations commands; compiler wrappers live in `toolchain/`. |
| `deploy/` | Deployment variants and supporting server configuration. |
| `docs/` | Maintained guides, architecture, and project policies under `project/`. |
| `.github/` | CI, repository automation, and community contribution/security/support policies. |
| `.cargo/` | Cargo-specific configuration. |

Keep standard build manifests, lockfiles, `Dockerfile`, `docker-compose.yml`,
configuration examples, READMEs, changelog, and license at the root so common
commands work without extra flags. Put new files with their owning component;
avoid adding a top-level directory for a single helper or duplicating documents.
Runtime resources belong in `resources/`; test-only examples belong in
`tests/fixtures/`. Keep SQL migrations in their conventional location.

`target/`, `.modelport/`, `dashboard/node_modules/`, and dashboard build/test
outputs are generated local state, excluded from version control. They are not
source directories. Keep `.env`, `config.toml`, logs, and backups local too.

## Toolchain

The maintained baseline is:

- Rust 1.96.0 with `rustfmt` and `clippy`, pinned by
  `rust-toolchain.toml` and used by the backend container build.
- Node.js 24 and npm for the dashboard.
- `curl` and Node.js for acceptance scripts.
- Docker with Compose v2 for the complete-stack path.
- A C/C++ linker toolchain. Repository Zig wrappers are a fallback on hosts
  where the normal compiler is unavailable.

`scripts/install-deps-ubuntu.sh` installs only a small set of native packages;
it does not install Rust, Node.js, npm, Docker, or Playwright browsers.

Before installing project dependencies, verify that the current shell resolves
only Linux tools and matches the pinned versions:

```bash
scripts/dev.sh doctor --development
```

The check rejects Node/npm or other tools resolved from Windows-mounted
`/mnt/<drive>` paths. NVM users must activate Node.js 24 in the current shell.

## Backend

The local backend does not start PostgreSQL for you. Before `scripts/dev.sh`,
make sure the `MODELPORT_DATABASE_URL` copied from `.env.example` is reachable.
For a disposable loopback-only development database, one option is:

```bash
docker run -d --rm --name modelport-dev-postgres \
  -p 127.0.0.1:5432:5432 \
  -e POSTGRES_DB=modelport \
  -e POSTGRES_USER=modelport \
  -e POSTGRES_PASSWORD=change-this-db-password \
  postgres:18.4-alpine
```

This password is deliberately development-only and matches `.env.example`.
Use a unique secret for any persistent or shared environment. Stop the
disposable database with `docker stop modelport-dev-postgres`.

```bash
cp .env.example .env
cp config.example.toml config.toml
# replace every required placeholder
scripts/dev.sh validate
scripts/dev.sh
```

For daily source development, use one entry point:

| Task | Command |
| --- | --- |
| Foreground gateway | `scripts/dev.sh` |
| Background start / restart / stop | `scripts/dev.sh start`, `restart`, or `stop` |
| Process and health summary | `scripts/dev.sh status` |
| Recent gateway logs | `scripts/dev.sh logs` |
| Configuration and runtime diagnosis | `scripts/dev.sh doctor` |
| Complete repository checks | `scripts/dev.sh check` |
| Rust-only checks | `scripts/dev.sh check --backend` |

`scripts/dev.sh help` lists these commands without requiring local configuration
or a toolchain. Commands resolve the checkout from the script location, so they
also work when invoked by absolute path from another directory. The original
`start.sh`, `stop.sh`, `restart.sh`, and `status.sh` forward to the same implementation.

Native start/stop only manage binaries under this checkout's `target/debug/`
or `target/release/`. A reused PID or another program listening on the same
port is left alone. An existing unhealthy native process must be diagnosed or
restarted explicitly. Use Compose or systemd to manage those deployments.

The scripts keep PID/log files below `.modelport/` and never require committing
the local `.env`. Before launching a stopped service, `scripts/dev.sh start` reuses
`target/release/model-port` only when it is newer than `src/`, `crates/`,
`resources/`, `migrations/`, `Cargo.toml`, `Cargo.lock`, and
`rust-toolchain.toml`; missing inputs also invalidate the cache. Otherwise it
rebuilds with
`cargo build --release --locked --bin model-port`. `scripts/dev.sh validate` uses the same
freshness helper. Set `MODELPORT_FORCE_BUILD=1` to bypass the cache explicitly.

`model-port config validate` and normal server startup call the same application
checks and deployment-environment preflight. Add a regression test whenever a
new configuration, database/TLS, lease, proxy, or origin error should fail
closed; do not rely on the CLI wrapper alone. Preflight is intentionally
connection-free, so database reachability and certificate verification still
belong in runtime integration tests.

## Dashboard

```bash
cd dashboard
npm ci
npm run dev
```

The Vite development server listens on `127.0.0.1:33002` and proxies backend
paths to `127.0.0.1:38082`. For browser development, prefer this same-origin
proxy. Mock mode is UI-only and must not be used as evidence of backend behavior:

```bash
VITE_MODELPORT_MOCK=1 npm run dev
```

The setup initializer can also be checked without changing the local deployment:
`scripts/setup.sh /tmp/modelport-setup-check`. It preserves existing files and
creates a minimal Compose environment; the full examples remain configuration
references.

## Test Layers

Fast backend checks:

```bash
scripts/dev.sh check --backend
```

This runs `cargo fmt --all -- --check`, locked tests for all targets, and locked
clippy for all targets/features with warnings denied. Dashboard checks are
separate:

```bash
cd dashboard
npm run lint
npm run build
npm run e2e
```

The aggregate repository check also validates shell syntax, configuration
examples, dashboard type/lint/unit/build, and Rust targets:

```bash
scripts/dev.sh check
```

### Dependency Audits

CI additionally audits both locked dependency graphs. Run the same security
gate locally after dependency changes:

```bash
cargo install cargo-audit --locked --version 0.22.2
cargo audit --deny warnings --file Cargo.lock
cargo install cargo-deny --locked --version 0.20.2
cargo deny check
node scripts/audit-dashboard.mjs
```

`cargo audit` downloads the current RustSec advisory database, so this networked
check is kept separate from the deterministic repository check script. Project
exceptions live in `.cargo/audit.toml` and must document the exact dependency
path, why the affected operation is unreachable, and the condition for removing
the exception. `RUSTSEC-2023-0071` is currently limited to the transitive
`openidconnect -> rsa` dependency: ModelPort verifies provider-signed ID tokens
with public JWKs and does not perform the vulnerable RSA private-key operation.

### Runtime And Deployment Checks

Release-oriented backend images must be built with
`scripts/build-container.sh`. It refuses uncommitted source and records the Git
revision in OCI labels. Clean builds use an immutable Git archive for all
images, so concurrent worktree edits cannot change later image contents.
`--allow-dirty` is limited to local integration testing;
downstream release verification rejects the resulting dirty source-state label.
The default build produces only gateway and Dashboard images. Add
`--with-ops-agent` for the optional Agent; workspace tests and release jobs
continue to cover all three workspace members.

Install the Playwright browser and OS dependencies using Playwright's supported
installer for your host when needed, for example:

```bash
npx playwright install --with-deps chromium
```

Runtime verification:

```bash
scripts/dev.sh doctor
scripts/smoke-test.sh
scripts/acceptance.sh
scripts/tool-use-acceptance.sh
```

Commands with `--upstream`, plus `provider-matrix.sh`, make real provider calls
and may incur cost. Use mock-backed Tool Use acceptance for routine adapter work.

## Change-to-Test Matrix

| Change | Minimum additional verification |
| --- | --- |
| Protocol/request/response mapping | Rust tests, smoke; provider matrix for the affected provider. |
| SSE or Tool Use | Rust stream tests and `tool-use-acceptance.sh`; real upstream only for certification. |
| Auth/policy/quota | Rust tests and `acceptance.sh`. |
| Provider catalog/defaults | Config validation, `/v1/models`, provider matrix, docs catalog update. |
| Dashboard behavior | lint, build, affected Playwright specs. |
| Docker/systemd/reverse proxy | Render/build the deployment and run smoke through the deployed origin. |
| Persistence/backup | Clean PostgreSQL migration plus export/validate/restore rehearsal. |
| Documentation only | Link, command, default-value, and bilingual-entry checks below. |

## Code Boundaries

Follow [Architecture](ARCHITECTURE.md#backend-boundaries) for module ownership
and [Contributing](../.github/CONTRIBUTING.md#code-and-security-conventions)
for code and security conventions. Keep existing protocol and persistence
regression cases when separating a large module.

## Documentation Checks

Run the existing link checker after moving or editing documentation:

```bash
node scripts/check-doc-links.mjs
```

The complete repository check also validates configuration and Runtime Adapter
examples. Check changed command examples against `--help` and keep the English
and Chinese README paths aligned. Maintain one detailed procedure per topic;
link to it from introductory and contribution pages.

## Local Build Cache

`target/` is disposable build output. For disk pressure, first stop any active
Cargo command, then remove only `target/debug/incremental/`. This preserves
compiled dependencies and executables while reclaiming incremental compiler
state. Subsequent source recompilation may be slower. `cargo clean` removes
all build output and requires a full rebuild. Neither action removes `.env`,
configuration, logs, or database volumes.
