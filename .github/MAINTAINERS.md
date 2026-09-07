# Maintainer Policy

This file records repository administration policy. Runtime and release
behavior lives in the maintained documents under `docs/`.

## Repository Metadata

- Description: `Self-hosted multi-protocol model gateway for Anthropic and OpenAI-compatible clients.`
- Topics: `claude-code`, `anthropic`, `openai-compatible`, `llm-gateway`,
  `model-router`, `rust`, `vscode`, `deepseek`.
- License: MIT.

Do not claim Provider verification without dated evidence produced by the
[Provider procedure](../docs/PROVIDERS.md#discovery-and-verification).

## Branch Protection

Protect `main` with:

- pull requests before merge;
- Repository checks, PostgreSQL dashboard E2E, dependency review, and CodeQL as
  required statuses;
- up-to-date branches and resolved review conversations;
- maintainer review for security, persistence, and deployment changes;
- restricted force pushes and branch deletion;
- signed commits and tags where the signing setup is available.

The Beta repository enforces PRs, the five GitHub Actions checks (including both
CodeQL languages), current branches and resolved conversations for administrators
too. Force pushes and branch deletion are disabled. The independent-approval
count is currently zero: a second release/security maintainer has not completed
the continuity gate below. Do not describe this as enforced two-person review.
Raise the required review count and enable CODEOWNERS review after that handoff.
The active version-tag ruleset prohibits updating or deleting `v*` tags without
a bypass. Private vulnerability reporting, automatic Dependabot security
updates and immutable publication are enabled. Existing older Releases are not
retroactively made immutable by that setting.

Routine CI uses the pinned Rust and Node versions, locked dependencies,
fmt/test/clippy, dashboard checks, shell syntax, examples, and documentation
links through `scripts/check-all.sh`. Paid Provider tests require an explicit,
budget-capped workflow and must not run for forks.

## Issues And Pull Requests

Issue forms and reviews must not request a complete `.env`, Provider key,
session/client token, database backup, raw prompt/response, or unreviewed log.
Security reports follow [SECURITY.md](SECURITY.md), not a public issue.

## Repository Controls

Enable private vulnerability reporting, Dependabot alerts and security updates,
immutable releases where available, tag protection, and artifact attestations.
Workflow files cannot enable these external repository settings.

Follow [Releasing](../docs/RELEASING.md) for versioning, evidence, artifacts,
and rollback.

## Stable Release Continuity Gate

Do not publish `v1.0.0` until two named maintainers have tag/release, package,
private vulnerability-reporting, and security-advisory access. Both must
complete one release rehearsal and document the security handoff. The current
single-maintainer state is acceptable for Beta and must not be represented as
stable-project bus-factor coverage.
