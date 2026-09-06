# Operations Agent

`modelport-ops-agent` is an optional, free, open-source companion process for a
single ModelPort instance. It evaluates sanitized runtime snapshots with
deterministic rules and writes incidents back through a versioned API. It is
off by default, is not a shell runner, and does not repair the system
automatically.

## What It Watches

The MVP evaluates six stable event groups every five minutes:

1. fail-closed readiness and storage dependencies;
2. Provider degradation, cooldown, authentication, and balance state;
3. server, protocol, and stream failure ratio at a minimum request volume;
4. budget accounts at 80 percent or exhausted;
5. unreconciled requests and stale usage reservations;
6. readiness regressions within fifteen minutes of a configuration change.

Incident evidence contains counts, booleans, bounded classifications, component
IDs, and timestamps. It excludes prompts, responses, tool values, API keys,
cookies, raw Provider bodies, and database URLs.

## Create The Dedicated Identity

Sign in as an administrator, open **API 密钥**, and create a service account:

- purpose: `modelport_ops_agent` (exact value);
- expiry: no more than 90 days;
- model scope: `__ops_agent_no_inference__`;
- Provider scope: `__ops_agent_no_inference__`;
- no team unless your operating policy explicitly requires one.

The sentinel scopes make the key unusable for normal inference. ModelPort also
checks the service-account principal and exact purpose on every internal Agent
request. Save the one-time secret in `.env` as `MODELPORT_OPS_API_KEY`.
Heartbeat identity is bound server-side to that API key ID; the Agent cannot
invent additional instance identities.

## Optional Base Model

The rules engine does not require a model. Administrators may optionally select
one routable model on **运维事件 → Agent 启用与基础模型**. Local routes
(`ollama` and `local_*`) are listed first and become the recommendation when
available; an explicit administrator selection always wins.

Model analysis is advisory and is attached only to already-detected active
incidents. It cannot create facts, change severity, close incidents, or execute
actions. Create a second service account with purpose `modelport_ops_model` and
a least-privilege inference key limited to the selected model and Provider,
then set it as `MODELPORT_OPS_MODEL_API_KEY`. Never reuse the Agent control key:
ModelPort deliberately rejects that key on `/v1`.

## Safe Rollout

There are two independent opt-in gates. The Compose profile is absent from the
default startup, and the persisted Agent setting defaults to disabled. First
start the optional process in shadow mode:

```bash
scripts/build-container.sh --with-ops-agent
MODELPORT_OPS_MODE=shadow docker compose --profile ops-agent up -d ops-agent
docker compose logs --tail=100 ops-agent
docker compose exec ops-agent curl -fsS http://127.0.0.1:38083/readyz
```

The default local build includes only the gateway and Dashboard. For a local
integration build with uncommitted changes, add `--allow-dirty`; published
release images continue to include the optional Agent as a separate artifact.

It will report a disabled heartbeat but will not evaluate rules until an
administrator opens **运维事件**, chooses a base model if desired, turns on
**启用运维 Agent**, and saves. This browser action does not start containers.
Once enabled, shadow mode fetches facts and evaluates all rules without writing
incidents.

The root Compose file intentionally does not publish port 38083; the readiness
check above runs inside the container.

After checking at least one complete interval, explicitly enable incident
writes in the deployment environment:

```env
MODELPORT_OPS_MODE=read_only
MODELPORT_OPS_INTERVAL_SECONDS=300
```

Then recreate only the Agent and inspect **运维事件** in the administrator
console:

```bash
docker compose --profile ops-agent up -d --no-deps --force-recreate ops-agent
```

To stop all evaluation immediately, turn off the persisted setting, set
`MODELPORT_OPS_MODE=disabled`, or stop the optional container. Each path is
fail-closed and never changes gateway readiness.

## Delivery And Recovery

The Agent spool is `/var/lib/modelport-ops/spool.sqlite` in the
`modelport-ops-spool` volume. It is capped at 10,000 observations. Identical
queued facts are deduplicated; the server independently deduplicates evidence.
The Compose profile defaults to 0.5 CPU, 256 MiB memory, and 128 PIDs; override
those explicit limits only after measuring the host.

PostgreSQL stores the authoritative incident, evidence, timeline, heartbeat,
and feedback records. Back up and restore it with the same ModelPort database
procedure. Deleting the SQLite volume only loses observations that were not yet
accepted; it does not delete accepted incidents.

An optional `MODELPORT_OPS_WEBHOOK_URL` receives a sanitized v1 JSON envelope
when an active observation is accepted. Webhook failure is logged and never
blocks the incident ledger or the gateway.

## Current Boundaries

- one Agent per ModelPort instance;
- no HA leadership or cross-instance incident merging;
- no arbitrary queries, commands, or automatic changes;
- optional model diagnosis uses a separately scoped key and sanitized facts;
- deterministic rules remain authoritative when the model is unavailable;
- console and generic webhook only; paging/on-call integrations are downstream;
- rules use fixed thresholds and administrator feedback is evidence for future
  tuning, not an automatic self-modifying policy.

The full safety rationale is in
[ADR-0006](adr/0006-read-only-operations-agent.md).
