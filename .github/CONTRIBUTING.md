# Contributing To ModelPort

ModelPort prioritizes a reliable Anthropic-compatible text path, explicit
security boundaries, and low operational cost for a single host or small team.
Changes should preserve that scope and distinguish implemented behavior from
provider-specific verification or future proposals.

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
Project decisions and contribution licensing are described in
[GOVERNANCE.md](../docs/project/GOVERNANCE.md).

## Development And Verification

Follow [Development](../docs/DEVELOPMENT.md) for the pinned toolchain,
PostgreSQL setup, and daily commands. Run `scripts/dev.sh check` before a pull
request; `scripts/dev.sh check --backend` is the Rust-only subset.

Use the [change-to-test matrix](../docs/DEVELOPMENT.md#change-to-test-matrix)
for additional checks. Dependency changes must also pass the
[dependency audits](../docs/DEVELOPMENT.md#dependency-audits). Never add a broad
or non-expiring audit exception. Ordinary verification uses synthetic data;
real Provider certification requires explicitly intended, potentially paid calls.

## Code And Security Conventions

- Preserve module boundaries described in
  [Architecture](../docs/ARCHITECTURE.md#backend-boundaries).
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
- Confirm every contribution is your work or is legally reusable under terms
  compatible with the MIT-licensed repository. Identify generated and
  third-party material in the pull request.

## Documentation Contract

- Root READMEs are short user entry points. Maintained behavior belongs in
  `docs/` and should be linked instead of copied.
- Label features as **implemented**, real providers as **verified** only with a
  dated result, and future work as **proposed**.
- Keep English and Chinese README commands, endpoints, limits, and links aligned.
- Update configuration, deployment templates, scripts, and docs together when a
  variable or default changes.
- Product tutorials belong in the maintained role-based documentation tree;
  internal interview notes and dated task plans do not.
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
