# Governance

ModelPort is an open-source project maintained by `@tiammomo`.

## Roles

- **Users** operate ModelPort and provide reproducible feedback.
- **Contributors** submit documentation, tests, code, or review.
- **Maintainers** triage issues, review changes, manage security reports, and
  publish releases.
- The current **lead maintainer** has final responsibility for project scope,
  security decisions, trademarks, and releases.

New maintainers are invited based on sustained, high-quality contributions,
sound security judgment, respectful collaboration, and willingness to perform
maintenance work. Maintainer changes are recorded in this file and CODEOWNERS.

## Decisions

Routine changes are decided through issue and pull-request review. Significant
changes should include an ADR when they affect public protocols, persistent
state, security boundaries, deployment topology, or long-term maintenance
cost.

The lead maintainer seeks rough consensus but may make the final decision when
tradeoffs remain. Decisions should be based on implementation evidence,
security, operational cost, and the documented product scope—not vendor
marketing or unverified Provider behavior.

## Releases And Security

Only maintainers may create release tags, publish container images, or issue
security advisories. A release must satisfy [the release process](../RELEASING.md).
Security reports follow [SECURITY.md](../../.github/SECURITY.md) and are handled privately
until coordinated disclosure is appropriate.

Small-Team Beta targets a four-week release cadence, with urgent security fixes
outside that schedule. Only the latest Beta is maintained; see
[SUPPORT.md](../../.github/SUPPORT.md). A stable release is blocked until at least two named
maintainers have repository release and private security-response access and
have completed a release/security handoff. This is an explicit continuity gate,
not a claim that a second maintainer already exists.

## Contribution Terms

Unless explicitly stated otherwise, contributions are submitted under the
repository's MIT License. The project does not currently require a contributor
license agreement. Contributors must have the right to submit their work and
must identify generated, copied, or third-party material whose license affects
the project.

## Free Open-Source Boundary

The project ships one complete MIT-licensed, self-hosted codebase. It has no
paid edition, hosted tier, license gate, or feature segmentation; security,
privacy, governance, and reliability work belongs in the open-source core. The
MIT License still permits third parties to use, distribute, host, or sell
services around the software, but doing so grants no project governance control
and creates no maintainer support obligation.
