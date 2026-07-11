# Security Policy

ModelPort holds upstream provider credentials and exposes a model-routing data
plane plus an administrative control plane. Its supported deployment boundary
is one trusted host or a small trusted network behind a firewall or same-origin
HTTPS reverse proxy. Do not expose the backend directly to the public internet.

## Supported Version

Security fixes target the latest `main` branch and the newest published release.
Older snapshots may not receive backports unless a release notice says so.

## Reporting A Vulnerability

Use the repository's GitHub **Security Advisories → Report a vulnerability**
flow when available. Include:

- affected version or commit;
- prerequisites and a minimal reproduction;
- the data, permission, network, or availability impact;
- whether the issue is reachable from the data plane, dashboard, or a trusted
  administrator action;
- only redacted logs and synthetic credentials.

If private reporting is unavailable, contact the repository owner through a
private channel listed on their GitHub profile and ask to open a private channel.
Do not place exploit details, provider keys, session tokens, backups, or a full
`.env` in a public issue.

## Assets To Protect

- Provider API keys and any local router token.
- Dashboard session cookies and password hashes.
- Dashboard-issued API keys; only hashes are retained after creation.
- PostgreSQL/JSON control state, which includes identity, policy, usage, IP,
  audit, and credential-variable metadata.
- CLI backups, which contain password and API-key hashes and can restore state.
- Prompts and provider responses, even though ModelPort does not intentionally
  persist complete request/response bodies in its usage log.

## Authentication Boundaries

- `/v1/*`, `/metrics`, and detailed diagnostics require a router or dashboard-
  issued API key. For a shared deployment, create control-plane keys and set
  `MODELPORT_REQUIRE_CONTROL_API_KEYS=1` so the unrestricted legacy token is not
  accepted.
- Dashboard users authenticate separately. Passwords use Argon2 hashes. Hash
  work runs outside the auth-state mutex on blocking workers, with at most four
  concurrent login hashes; waiting longer than five seconds returns HTTP 429.
  Unknown and disabled users still perform an equivalently expensive hash-class
  operation to reduce timing enumeration. Five failures for a username lock it
  for 15 minutes. Lockout counters, the worker gate, and sessions are
  process-local and reset on restart.
- Session cookies are HttpOnly and SameSite=Lax. Set
  `MODELPORT_ADMIN_COOKIE_SECURE=1` whenever the dashboard is served over HTTPS.
- Dashboard writes require a session, `X-ModelPort-CSRF`, and an allowed
  Origin/Referer when present. `MODELPORT_ALLOWED_ORIGINS` extends that write
  check; it does not enable browser CORS.
- The backend has no general CORS response policy. Serve dashboard and API from
  one trusted origin.

## Network And Provider URLs

- Keep `MODELPORT_BIND` and published Docker ports on loopback unless a trusted
  network or reverse proxy needs them.
- Configure `MODELPORT_TRUSTED_PROXIES` with exact proxy IPs/CIDRs. Forwarded
  client-IP headers are security inputs for IP policy and rate limiting.
  ModelPort walks XFF from the connected peer right-to-left and removes only
  explicitly trusted hops; a single-hop proxy should overwrite XFF with its
  observed client address. Preserve the original Host authority including its
  port so browser Origin/Host write checks remain aligned.
- Provider URL validation rejects non-HTTP schemes, userinfo, query strings,
  fragments, and literal private/link-local/metadata addresses by default,
  including private IPv4 addresses encoded as IPv4-mapped IPv6 literals.
  Local/custom providers have explicit loopback allowances. Credentials must
  come from environment-backed header configuration, never URL userinfo or a
  query parameter.
- Non-local/non-custom Providers require HTTPS by default. Plain HTTP exposes
  the Provider API key and prompt/response content to every network hop. Use
  `MODELPORT_ALLOW_INSECURE_PROVIDER_HTTP=1` only for an explicitly trusted
  internal upstream; local/custom runtimes retain HTTP support for controlled
  local integration. The HTTP override does not disable private/metadata-IP
  protection.
- Hostnames are not currently pinned or revalidated after DNS resolution. A
  hostname that resolves to an internal address is outside the current SSRF
  guard. Use outbound firewall rules or an allowlist when administrators are not
  fully trusted.
- Upstream HTTP redirects are disabled and every GET/POST/SSE handshake requires
  2xx. A 3xx is treated as an upstream failure and mapped to client-facing 502,
  not followed or exposed as a client redirect. Upstream non-stream bodies and
  raw SSE bytes are bounded, and streams have an idle timeout. SSE additionally
  rejects 204 and any missing/non-`text/event-stream` media type before local
  headers. Non-2xx/non-SSE error-body reads have total and idle timeouts, and a
  live stream must reach its protocol termination event rather than treating
  EOF as successful completion.
- PostgreSQL connections currently use `NoTls`. The default Compose deployment
  keeps them on its private bridge; external PostgreSQL must use an already
  protected trusted path or tunnel. Do not expose this connection across an
  untrusted network. Compose's constructed URL also requires a URL-safe password
  or an explicitly percent-encoded `MODELPORT_DATABASE_URL` override.

`MODELPORT_ALLOW_PRIVATE_PROVIDER_URLS=1` deliberately weakens the URL boundary
and should only be used for a trusted internal runtime.

## Logs, Errors, And Backups

The usage log stores request ID, identity/team labels, model/provider, token and
cost estimates, status, latency, retry/fallback, client IP, and bounded error
text. It does not intentionally persist prompts, complete messages, raw provider
bodies, authorization headers, or plaintext keys.

Upstream errors redact common credential field names and common token patterns.
This is best-effort defense in depth, not proof that arbitrary third-party text
is safe to publish. Review logs before sharing them.

Common secret-bearing configuration, auth, control-store, login, password-input,
and API-key creation types use custom or redacted `Debug` output, with regression
tests for the sensitive fields. This reduces accidental tracing and diagnostic
leaks; it does not remove secrets from process memory, crash/core dumps, or
every third-party value. Keep dumps and unrestricted debug access disabled or
tightly controlled.

`MODELPORT_USAGE_LOG_LIMIT` controls retained usage records. Dashboard
diagnostic snapshots are redacted but contain personal/usage data. CLI backups
are complete restore artifacts and must be encrypted, access-controlled, and
deleted under a defined retention policy.

Diagnostic snapshot export is a CSRF-protected `POST /admin/backup` operation
and creates an audit event; no safe-method GET alias is exposed. Provider model
discovery is likewise a mutating POST because it stores provider-test and audit
state.

CLI validation/restore deeply deserializes both documents and enforces critical
auth invariants, including unique identities, valid role/status/password hashes,
and retention of an active admin. This reduces corrupt-restore risk but does not
make the sequential auth/control replacement transactional; stop writers and
retain the automatically saved previous values plus a storage-native backup.

## Deployment Checklist

1. Replace every placeholder and use a long unique admin password and router
   token.
2. Run `scripts/config-validate.sh`; never set `MODELPORT_ALLOW_NO_AUTH=1` on a
   shared host.
3. Bind to loopback or place the service behind a firewall and same-origin HTTPS
   reverse proxy.
4. Set secure cookies, exact trusted proxies, and the expected dashboard origin.
5. Require dashboard-issued API keys for shared data-plane traffic.
6. Protect `.env`, PostgreSQL/JSON state, journal logs, and complete backups.
7. Keep request/response byte limits, stream timeouts, and concurrent-stream
   permits finite.
8. Run the acceptance checks appropriate to auth, routing, or deployment changes.

## Known Security Limits

- No OIDC/SSO, enterprise IAM, or public multi-tenant isolation.
- No complete DNS-rebinding protection for provider hostnames.
- No native TLS transport for the PostgreSQL state connection.
- Rate limits, concurrent-stream permits, and login/session state are not shared
  across instances. Stream permits remain occupied until body completion/drop.
- Origin validation allows non-browser requests without Origin/Referer; it is
  not an authorization mechanism.
- Secret redaction cannot recognize every credential format.
- Quota pre-check/update is not a transactional reservation under concurrency.

See [Architecture](docs/ARCHITECTURE.md),
[Configuration](docs/CONFIGURATION.md), and
[Operations](docs/OPERATIONS.md) for the corresponding implementation limits.
