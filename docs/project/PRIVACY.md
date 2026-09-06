# Privacy

ModelPort is self-hosted software. The open-source project does not include
maintainer-operated telemetry, advertising, or a hosted analytics service.

## Data Processed By A Deployment

A ModelPort operator may process:

- user identities, roles, teams, API-key hashes, IP policy, and audit events;
- request identity and routing metadata, token usage, estimated cost, status,
  latency, TTFT, Provider/model selection, and client IP;
- Provider credentials in process environment and complete auth/control
  backups;
- prompts and responses in transit between the client, gateway, and Provider.

ModelPort does not intentionally persist full prompts or responses in the
operational ledger. Logs, reverse proxies, clients, Providers, crash dumps, or
custom integrations may still retain them.

## Operator Responsibility

The operator chooses the deployment region, Providers, users, retention period,
backup location, and network exposure. The operator is responsible for notices,
legal basis, access controls, deletion/retention procedures, data-subject
requests, and contracts required in its jurisdiction.

Provider calls disclose request content and metadata to the configured
Provider under that Provider's terms. GitHub processes repository interactions
under GitHub's own policies; it is not part of the ModelPort runtime.

## No Maintainer Access

Maintainers cannot access a self-hosted deployment unless its operator
explicitly grants access or sends diagnostic material. Never send complete
backups, environment files, credentials, prompts, responses, or unredacted
logs in a public issue.
