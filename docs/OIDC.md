# OIDC Console Sign-In

ModelPort can use an external OpenID Connect (OIDC) provider for human console
sign-in. OIDC authenticates a person to the ModelPort control plane. It does not
turn a ChatGPT browser subscription, cookie, or session into an OpenAI API
credential.

The data plane and Provider credential remain separate:

```text
browser -> OIDC provider -> ModelPort console session
SDK/BFF -> ModelPort API key -> ModelPort data plane
ModelPort -> server-side Provider credential -> OpenAI or another Provider
```

## Prerequisites

- Serve the dashboard and backend from one HTTPS origin.
- Register an OIDC confidential or public web client with the identity provider.
- Register the exact callback URL
  `https://modelport.example.com/admin/auth/oidc/callback`.
- Keep the local bootstrap administrator available as a recovery identity until
  the OIDC configuration has been exercised successfully.

## Configuration

OIDC is disabled unless all required values are present:

```env
MODELPORT_OIDC_ISSUER=https://identity.example.com/realms/modelport
MODELPORT_OIDC_CLIENT_ID=modelport
MODELPORT_OIDC_REDIRECT_URI=https://modelport.example.com/admin/auth/oidc/callback

# Set for a confidential client. Leave unset only when the provider accepts a
# public-client authorization-code exchange with PKCE.
MODELPORT_OIDC_CLIENT_SECRET=replace-with-client-secret

# Optional presentation and claim mapping.
MODELPORT_OIDC_LABEL=Company SSO
MODELPORT_OIDC_USERNAME_CLAIM=preferred_username
MODELPORT_OIDC_EMAIL_CLAIM=email

# Disabled by default. When disabled, an administrator must create the user in
# ModelPort before the first OIDC sign-in. Enabling it creates ordinary `user`
# identities only; it never grants administrator access.
MODELPORT_OIDC_AUTO_PROVISION=0

# Local development only. This is accepted only when both the issuer endpoints
# and callback host are loopback addresses.
# MODELPORT_OIDC_ALLOW_INSECURE_HTTP=1
```

Set the normal browser protections as well:

```env
MODELPORT_ADMIN_COOKIE_SECURE=1
MODELPORT_ALLOWED_ORIGINS=https://modelport.example.com
```

The issuer must provide standard OIDC discovery metadata. Remote issuer,
authorization, token, and JWKS endpoints must use HTTPS. Loopback HTTP is only
appropriate for an explicitly local development provider.

The initial account-link and automatic-provision paths require the standard
`email` claim together with `email_verified=true`. A verification assertion for
the standard claim is never transferred to a differently named custom claim;
keep `MODELPORT_OIDC_EMAIL_CLAIM=email` for initial linking and JIT in this
preview.

## Sign-In Flow

1. The login page reads `GET /admin/auth/methods` to determine whether OIDC is
   enabled.
2. `GET /admin/auth/oidc/start` creates bounded, single-use state, nonce, PKCE,
   and a short-lived HttpOnly browser-flow cookie, then redirects to the
   identity provider.
3. The provider redirects to `GET /admin/auth/oidc/callback` with an
   authorization code.
4. ModelPort requires the callback to carry both the state and the browser-flow
   cookie, consumes them once, exchanges the code, validates the ID token
   issuer, audience, signature, expiry, and nonce, then creates the normal
   HttpOnly ModelPort console session. Binding state to the initiating browser
   prevents login CSRF in which another user is tricked into completing an
   attacker's sign-in.

Only a local relative `returnTo` path is accepted. External return URLs and
protocol-relative paths are rejected so the login flow cannot be used as an
open redirect.

## Account Linking And Provisioning

An OIDC identity is permanently identified by the `(issuer, subject)` pair.
ModelPort first looks for that binding. For a previously unbound local,
non-administrator user it can bind only a unique matching email address when
the provider explicitly marks that address as verified. A username claim is
never used for implicit account linking. A subject already bound to one user
cannot be rebound to another user, and an existing administrator is never
linked implicitly.

With automatic provisioning disabled, create an ordinary `active` user with
the same unique, verified email address before their first SSO login. This is
the recommended initial deployment mode. Automatic provisioning, when
explicitly enabled, requires a verified email and a valid, globally unique
username claim and creates only an ordinary `user` role. Username/email
collisions fail closed instead of creating a shadow account. Administrator
access remains a separate, audited ModelPort operation.

OIDC users still need a ModelPort data-plane API key for SDK or API requests.
The console session cookie is intentionally not accepted by `/v1/messages` or
`/v1/chat/completions`. An administrator can issue a scoped API key and apply
team, model, Provider, IP, expiry, quota, and spend policy before handing it to
the user or to a server-side BFF.

## Operational Notes

- OIDC authorization state and ModelPort console sessions are process-local in
  the current release. A restart invalidates in-progress login flows and active
  sessions.
- Starting a second OIDC flow in the same browser replaces its short-lived flow
  cookie; finish the newest flow or start again.
- ModelPort logout clears only the local console session. RP-initiated logout
  and identity-provider single logout are not implemented in this preview.
- Rotate the OIDC client secret at the identity provider and in the ModelPort
  process environment together, then restart the service.
- OIDC settings are startup configuration; the dashboard config-reload action
  does not replace the active issuer, client, metadata cache, or pending flows.
- Do not log authorization codes, ID tokens, access tokens, client secrets, or
  full callback query strings. Configure every reverse proxy and load balancer
  in front of ModelPort to log only the callback path, not the raw request
  target or Referer. The bundled Nginx configuration already does this.
- Keep Provider API keys in the ModelPort server environment or an external
  secret manager. Never expose them to the browser.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| SSO button is absent | Required `MODELPORT_OIDC_*` values and configuration validation. |
| Provider rejects the callback | The registered redirect URI must match exactly, including scheme, host, port, and path. |
| Login returns to the page with an error | Issuer/audience/nonce validation, user status, and whether automatic provisioning is enabled. |
| Existing user is not linked | The standard email claim must uniquely match an active non-admin local user and the provider must assert `email_verified=true`. |
| Login stops working after restart | Start a new login; pending state and sessions are intentionally process-local. |
