# Small-Team Beta Compatibility Matrix

This matrix is the support boundary for ModelPort v0.1.x. “Tier 1” means the
project's release gate exercises install, startup, migration, smoke,
backup/restore, upgrade, and rollback for that combination. It is not an
availability SLA or certification.

## Tier 1

| Area | Supported combination | Evidence boundary |
| --- | --- | --- |
| Host | Linux x86_64 (`linux/amd64`) | The release gate must build, sign, and attest the archive and container images for this platform; `doctor.sh --setup` rejects other architectures. |
| Deployment | One backend and one Dashboard Nginx container on one trusted host/small trusted network | No active-active, rolling-upgrade, or distributed-state claim. |
| Container tooling | Docker Engine with Docker Compose v2 | Compose rendering, install, safe stop, backup/restore, and smoke paths are release gates. |
| Database | PostgreSQL 18.4 with the v18 data directory | Default Compose and CI use the exact image; managed PostgreSQL remains operator-verified. |
| Browser | Current Chromium-family browser | Dashboard E2E uses Chromium; Firefox/Safari remain community-tested. |
| Client protocols | Documented Anthropic Messages and scoped OpenAI-compatible Chat Completions | OpenAI Responses is not included; undocumented fields are not promised. |
| Product scale | One internal development team of approximately 20–50 people | Capacity depends on Provider/model latency, stream duration, PostgreSQL, and host resources. |
| Product language | Complete Chinese-first Dashboard; maintained English API/operator docs | Full UI internationalization is deferred until a real English design partner exists. |

### Client/Harness setup profiles

Client/Harness profiles describe callers, not upstream Providers. Claude Code
uses the Anthropic Messages edge. The OpenAI SDK and a
[Qwen Code](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/model-providers/)
setup profile use the scoped OpenAI-compatible Chat Completions edge; Qwen Code
references its ModelPort client key through an environment variable rather than
storing the key in `settings.json`. The shipped setup profile does not replace
dated Provider/model/stream/Tool Use acceptance evidence for the selected
route.

[Codex CLI](https://developers.openai.com/codex/config-reference/) custom
Providers require the Responses wire API. ModelPort does not ship
`POST /v1/responses`, so the Dashboard reports Codex CLI as blocked and does not
offer a copyable configuration. Adding the scoped Responses ingress is a
separate future change, not part of the current compatibility profile.

Pin production images to the immutable digests recorded in the GitHub Release.
A mutable version tag is acceptable only for initial local evaluation.

## Tier 2 Evaluation

| Combination | Boundary |
| --- | --- |
| WSL2 x86_64 with Docker | Suitable for evaluation and contribution. Networking, suspend/resume, filesystem permissions, and background services differ from a Linux production host. |
| systemd on Linux x86_64 | Maintained as an advanced guide, without the complete container install/rollback acceptance matrix. |
| Managed PostgreSQL 18.x | Requires operator evidence for TLS `verify-full`, migrations, connection limits, backup/PITR, and restore. Only 18.4 is the exact repository baseline. |

Tier 2 failures are actionable with a reproducible report, but do not alone
block a release.

## Experimental

- Linux arm64 source builds. v0.1.x does not publish an arm64 image until its
  install, upgrade, backup, restore, and rollback evidence matches x86_64.
- Firefox and Safari Dashboard use.
- Unverified OpenAI-compatible endpoints explicitly enabled by an operator.
- Kubernetes manifests, alternative container engines, and undocumented
  reverse-proxy layouts.

Experimental means there is no release-quality compatibility promise. An
experimental path must never become a silent production fallback.

## Not Supported In v0.1.x

- Native Windows or macOS service installation.
- Public Internet multi-tenancy, untrusted tenant isolation, or a maintainer-
  hosted ModelPort service.
- Multiple active backend replicas, zero-downtime/rolling upgrades, automatic
  failover, or distributed sessions/rate limits/stream permits.
- OpenAI Responses, realtime, embeddings, image/audio APIs, a chat UI, model
  inference/training, payment processing, or authoritative Provider billing.
- Silent Tool Use downgrade or a claim that all OpenAI-compatible Providers are
  interchangeable.

## Provider And Model Evidence

A Provider template, model discovery result, or HTTP 200 at stream start is not
compatibility evidence. A Provider/model may enter a `code-*` logical model only
after the versioned procedures in [Tool Use Compatibility](TOOL_USE_COMPATIBILITY.md)
and [Providers](PROVIDERS.md#discovery-and-verification) pass for the exact
model, endpoint, stream mode, Tool Use features, and account entitlement.
Unsupported capabilities fail before egress; they are never silently dropped.

## Changing This Matrix

Every promotion requires dated CI or acceptance evidence plus updated install,
upgrade, backup, restore, rollback, and known-limit documentation. A community
success report can start that work but does not itself change the support tier.
