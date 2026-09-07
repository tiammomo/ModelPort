# ModelPort

[![CI](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/ci.yml)
[![CodeQL](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml/badge.svg)](https://github.com/tiammomo/ModelPort/actions/workflows/codeql.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/tiammomo/ModelPort/badge)](https://scorecard.dev/viewer/?uri=github.com/tiammomo/ModelPort)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**English** | [简体中文](README.zh-CN.md)

ModelPort is a free, self-hosted model gateway for 20–50 person development
teams. Administrators connect local or cloud models and define access boundaries;
developers copy client configuration and inspect request, routing, usage and
billing evidence when something fails. Licensed under MIT.

![ModelPort architecture overview](docs/assets/modelport-overview.svg)

## What You Get

- One endpoint for Claude Code, Qwen Code and the OpenAI SDK, with Messages,
  Chat Completions, streaming and Tool Use.
- Users, teams, scoped API keys, quotas and budgets; unknown and sensitive data
  stays local by default.
- Local and cloud Providers, model catalogs, deterministic and optional smart routes.
- Five console entry points: overview, model access, requests/usage, team/policy, system.
- PostgreSQL evidence, backup/restore and metrics; an optional operations Agent.

v0.1.x is a Chinese-first Small-Team Beta for single-instance Linux x86_64.
See the [compatibility matrix](docs/COMPATIBILITY.md) for protocol and deployment
boundaries. Team activation and diagnosis take priority; GPU expansion is driven
by verified needs in the [roadmap](docs/ROADMAP.md).

## Quick Start

Requires Linux x86_64, Git, Docker Compose v2 and Provider credentials. No host
Rust or Node installation is needed.

```bash
git clone https://github.com/tiammomo/ModelPort.git
cd ModelPort
scripts/setup.sh
# Set DEEPSEEK_ANTHROPIC_AUTH_TOKEN in .env
scripts/doctor.sh --setup
scripts/build-container.sh
scripts/compose-up.sh
docker compose ps
scripts/smoke-test.sh
```

The initializer writes distinct router, administrator and database credentials
to `.env` with mode `0600`; reruns preserve existing configuration. The default
build produces the gateway and Dashboard images. Compose supplies PostgreSQL
and publishes ports on loopback only.

Open `http://127.0.0.1:33002`, sign in with the administrator credentials in
`.env`, and choose **Continue setup (继续接入)**. The four-step journey resumes
from saved configuration. For another Provider, external PostgreSQL or published
images, use [Getting Started](docs/GETTING_STARTED.md).

## Send Your First Request

Follow **model access → egress policy → key/client → request evidence**. Record
and apply project policy through the form. Selecting a cloud Provider still
requires explicit egress authorization and data classification. Client setup
checks the selected key's permissions, credentials and policy before enabling copy.

The [first-request example](docs/GETTING_STARTED.md#7-send-the-first-request)
uses explicitly classified synthetic data and may consume Provider quota.
Ordinary `smoke-test.sh` does not call an upstream. Use request logs to confirm a
complete call; administrators can open the exact routing and billing evidence
from a log. Provider keys remain on the server.

## Documentation

- [Getting started and troubleshooting](docs/GETTING_STARTED.md)
- [Configuration](docs/CONFIGURATION.md) and [API](docs/API.md)
- [Deployment](docs/DEPLOYMENT.md), [operations](docs/OPERATIONS.md) and [upgrades/rollback](docs/UPGRADING.md)
- [Documentation index](docs/README.md) and [roadmap](docs/ROADMAP.md)

## Security And Support

Keep backend and PostgreSQL ports private. Use same-origin HTTPS, exact trusted
proxy CIDRs, secure cookies, CSRF protection, and dashboard-issued API keys for
shared use. Never commit `.env`, Provider keys, backups, prompts, responses, or
raw sensitive logs.

Read [Security](.github/SECURITY.md), [Privacy](docs/project/PRIVACY.md), [Support](.github/SUPPORT.md), and
[Governance](docs/project/GOVERNANCE.md). ModelPort is free self-hosted software. The
project provides no paid edition, hosted service, or community-support SLA.

## Development

Use the [development guide](docs/DEVELOPMENT.md) for the repository map,
pinned tools, PostgreSQL setup, and frontend workflow. Native gateway commands
share `scripts/dev.sh`; use `scripts/dev.sh help` to see the available actions.
Before submitting a change, run `scripts/dev.sh check`.

## License

[MIT](LICENSE)
