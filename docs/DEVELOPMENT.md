# Development

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

## Backend

```bash
cp .env.example .env
# replace every required placeholder
scripts/config-validate.sh
scripts/dev.sh
```

Background lifecycle:

```bash
scripts/start.sh
scripts/status.sh
scripts/restart.sh
scripts/stop.sh
```

The scripts keep PID/log files below `.modelport/` and never require committing
the local `.env`. Before launching a stopped service, `scripts/start.sh` reuses
`target/release/model-port` only when it is newer than `src/`, `Cargo.toml`,
`Cargo.lock`, and `rust-toolchain.toml`; otherwise it rebuilds with
`cargo build --release --locked`. `scripts/config-validate.sh` uses the same
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

The Vite development server listens on `127.0.0.1:5173` and proxies backend
paths to `127.0.0.1:17878`. For browser development, prefer this same-origin
proxy. Mock mode is UI-only and must not be used as evidence of backend behavior:

```bash
VITE_MODELPORT_MOCK=1 npm run dev
```

## Test Layers

Fast backend checks:

```bash
scripts/check.sh
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
scripts/check-all.sh
```

CI additionally audits both locked dependency graphs. Run the same security
gate locally after dependency changes:

```bash
cargo install cargo-audit --locked --version 0.22.2
cargo audit --deny warnings --file Cargo.lock
npm --prefix dashboard audit --audit-level=low
```

`cargo audit` downloads the current RustSec advisory database, so this networked
check is kept separate from the deterministic repository check script.

Install the Playwright browser and OS dependencies using Playwright's supported
installer for your host when needed, for example:

```bash
npx playwright install --with-deps chromium
```

Runtime verification:

```bash
scripts/doctor.sh
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
| Persistence/backup | File and PostgreSQL tests plus export/validate/restore rehearsal. |
| Documentation only | Link, command, default-value, and bilingual-entry checks below. |

## Code Boundaries

- HTTP route handlers should orchestrate, not absorb protocol conversion.
- Keep provider-specific behavior behind provider configuration or adapters.
- Do not log keys, authorization headers, prompts, raw multipart/base64 data, or
  complete provider bodies.
- Add regression tests for SSE splitting, Tool Use causality, errors after
  headers, redirect policy, body limits, and secret redaction.
- Treat API/README claims as tests: shipped, verified, and proposed must remain
  distinct.

## Documentation Checks

There is not yet a dedicated docs toolchain, so run these repository checks:

```bash
# Show Markdown targets for manual/existence checking.
rg -n '\]\([^)]+\)' -g '*.md' README*.md docs dashboard/README.md

# Find stale source/config names and placeholders.
rg -n 'src/database\.rs|gpt-4o|claude-sonnet-4-20250514|gemini-2\.5-flash' \
  README*.md docs .env.example dashboard/README.md

# Confirm documented options against scripts that provide a help mode.
scripts/acceptance.sh --help
scripts/bench.sh --help
scripts/doctor.sh --help
scripts/provider-matrix.sh --help
scripts/tool-use-acceptance.sh --help
```

Also validate a minimal environment configuration and any shipped TOML example.
Aliases in a full catalog must not be presented as a minimal copy/paste example
when their providers are disabled.

For English/Chinese entry pages, keep the same quick-start commands, endpoint
names, safety warnings, and documentation links. Prefer links to one maintained
reference over duplicating large default tables.
