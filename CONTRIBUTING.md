# Contributing To ModelPort

ModelPort prioritizes a reliable Anthropic-compatible text path, explicit
security boundaries, and low operational cost for a single host or small team.
Changes should preserve that scope and distinguish implemented behavior from
provider-specific verification or future proposals.

## Development Setup

The CI baseline is Rust stable and Node.js 24. Install Rust with `rustfmt` and
`clippy`, Node.js/npm, `curl`, and a native compiler toolchain. Docker Compose is
needed for the complete stack.

`scripts/install-deps-ubuntu.sh` installs only native helper packages; it does
not install Rust, Node.js, npm, Docker, or Playwright browsers.

```bash
git clone git@github.com:tiammomo/ModelPort.git
cd ModelPort
cp .env.example .env
# replace required placeholders; never commit this file
scripts/config-validate.sh
scripts/check.sh

cd dashboard
npm ci
npm run lint
npm run build
```

Full setup and the change-to-test matrix are in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Before A Pull Request

Run the aggregate check:

```bash
scripts/check-all.sh
```

If working incrementally, `scripts/check.sh` is the backend-only subset and
`cd dashboard && npm run check` is the dashboard subset.
Dependency updates must also pass `cargo audit --deny warnings --file Cargo.lock`
and `npm --prefix dashboard audit --audit-level=low`; CI runs both.

Then choose checks by risk:

- dashboard behavior: affected Playwright specs or `npm run e2e`;
- auth, API keys, teams, quota, or backup: `scripts/acceptance.sh`;
- protocol, SSE, or Tool Use: relevant Rust tests and
  `scripts/tool-use-acceptance.sh`;
- provider behavior: `scripts/provider-matrix.sh --model provider:model` and
  real Tool Use acceptance when certification is intended;
- Docker/systemd/reverse proxy: build/install the deployment and run smoke
  through the deployed origin.

Real upstream checks can incur cost and must use your own local secrets. CI and
ordinary pull requests should prefer mock-backed checks.

## Code And Security Conventions

- Preserve module boundaries described in
  [Architecture](docs/ARCHITECTURE.md#backend-boundaries).
- Keep protocol conversion in adapters and provider quirks in explicit provider
  configuration.
- Add tests for split SSE frames, errors after headers, Tool Use causality,
  request/response bounds, redirect behavior, and secret redaction.
- Do not log or commit API keys, session/API tokens, `.env`, `.modelport/`,
  complete backups, raw prompts/responses, or large base64/multipart payloads.
- Secret-bearing types need redacted `Debug` behavior and regression tests; a
  later derived/debug wrapper can silently undo that boundary.
- Treat control-plane changes as security-sensitive: verify role checks, CSRF,
  Origin, IP/trusted-proxy, and ownership behavior.
- Avoid presenting estimated usage/cost as exact billing.

## Documentation Contract

- Root READMEs are short user entry points. Maintained behavior belongs in
  `docs/` and should be linked instead of copied.
- Label features as **implemented**, real providers as **verified** only with a
  dated result, and future work as **proposed**.
- Keep English and Chinese README commands, endpoints, limits, and links aligned.
- Update configuration, deployment templates, scripts, and docs together when a
  variable or default changes.
- Learning/interview material is non-normative and must link back to maintained
  reference docs.
- Check relative links and example commands before submission.

## Pull Request Description

Explain:

- the behavior and user-visible outcome;
- impact on Claude Code / VS Code Claude and provider compatibility;
- validation commands and whether any used a paid upstream;
- migration, configuration, persistence, security, or cost implications;
- documentation updated for the changed contract.

Keep unrelated refactors out of the same pull request when they make the risk or
verification story harder to review.
