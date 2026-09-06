# Support

## Community Support

Use GitHub issues for reproducible defects and feature proposals. Before
opening an issue:

1. read the maintained documentation in [`docs/`](../docs/README.md);
2. run `scripts/config-validate.sh` and `scripts/doctor.sh`;
3. search existing issues;
4. remove API keys, tokens, prompts, responses, personal data, database URLs,
   and complete environment files from all output.

Questions that do not identify a product defect may be closed with a pointer
to the relevant documentation so the issue tracker remains actionable.

## Supported Versions

During Small-Team Beta, only the latest published Beta is supported. The
project targets one planned release every four weeks and gives the previous
Beta a 30-day upgrade window after its successor is published. Security fixes
may be released outside that cadence. `main` is a contributor integration line,
not a supported production release; older Betas receive no guaranteed
backports. Storage-breaking releases include explicit upgrade and rollback
instructions.

## Response Expectations

Community support is best effort and has no response-time, availability, or
resolution SLA. Provider outages, account billing, model availability, and
third-party API behavior remain the responsibility of the Provider.

ModelPort is free, self-hosted MIT software. This project provides no paid
edition, paid support plan, hosted ModelPort service, response SLA, or LTS line.
A third-party reseller, host, or consultant cannot create obligations for the
ModelPort maintainers.

## Security

Do not report vulnerabilities or secrets in a public issue. Follow the private
process in [SECURITY.md](SECURITY.md).
