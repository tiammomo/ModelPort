import assert from 'node:assert/strict'
import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { test } from 'node:test'
import { adminSession, body, createUser, databaseURL, evidence, gateway, json, secret, server } from './support.mjs'

async function identityProvider(t) {
  const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 })
  const wrongKey = generateKeyPairSync('rsa', { modulusLength: 2048 }).privateKey
  const jwk = { ...publicKey.export({ format: 'jwk' }), kid: 'assurance', alg: 'RS256', use: 'sig' }
  const grants = new Map()
  const clientSecret = secret()
  let selected = {}
  let tokenCalls = 0
  const idp = await server(async (req, res) => {
    const url = new URL(req.url, idp.url)
    if (url.pathname === '/.well-known/openid-configuration') return json(res, {
      issuer: idp.url, authorization_endpoint: `${idp.url}/authorize`, token_endpoint: `${idp.url}/token`,
      jwks_uri: `${idp.url}/jwks`, response_types_supported: ['code'], subject_types_supported: ['public'],
      id_token_signing_alg_values_supported: ['RS256'], token_endpoint_auth_methods_supported: ['client_secret_basic'],
    })
    if (url.pathname === '/jwks') return json(res, { keys: [jwk] })
    if (url.pathname === '/authorize') {
      assert.equal(url.searchParams.get('code_challenge_method'), 'S256')
      assert.equal(url.searchParams.get('client_id'), 'modelport-assurance')
      const code = secret()
      grants.set(code, { params: url.searchParams, selected: { ...selected } })
      const redirect = new URL(url.searchParams.get('redirect_uri'))
      redirect.searchParams.set('code', code)
      redirect.searchParams.set('state', url.searchParams.get('state'))
      res.writeHead(302, { location: redirect.href }); return res.end()
    }
    if (url.pathname === '/token') {
      tokenCalls++
      assert.ok(req.headers.authorization === `Basic ${Buffer.from(`modelport-assurance:${clientSecret}`).toString('base64')}`, 'confidential client authenticates at the token endpoint')
      const input = new URLSearchParams(await body(req))
      const grant = grants.get(input.get('code'))
      grants.delete(input.get('code'))
      assert.ok(grant, 'authorization codes are single use')
      assert.equal(input.get('grant_type'), 'authorization_code')
      assert.equal(input.get('redirect_uri'), grant.params.get('redirect_uri'))
      assert.equal(createHash('sha256').update(input.get('code_verifier')).digest('base64url'), grant.params.get('code_challenge'))
      const now = Math.floor(Date.now() / 1000)
      const claims = {
        iss: idp.url, aud: 'modelport-assurance', sub: 'assurance-subject', iat: now, exp: now + 300,
        nonce: grant.params.get('nonce'), email_verified: true,
        acr: 'urn:modelport:assurance:mfa', ...grant.selected.claims,
      }
      const accessToken = secret()
      claims.at_hash = createHash('sha256').update(accessToken).digest().subarray(0, 16).toString('base64url')
      if (grant.selected.badAccessHash) claims.at_hash = 'incorrect-token-hash'
      const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url')
      const unsigned = `${encode({ alg: 'RS256', kid: 'assurance', typ: 'JWT' })}.${encode(claims)}`
      const signature = sign('RSA-SHA256', Buffer.from(unsigned), grant.selected.badSignature ? wrongKey : privateKey).toString('base64url')
      return json(res, { access_token: accessToken, token_type: 'Bearer', expires_in: 300, id_token: `${unsigned}.${signature}` })
    }
    res.writeHead(404); res.end()
  })
  t.after(() => idp.close())
  return { ...idp, clientSecret, select: value => { selected = value }, tokenCalls: () => tokenCalls }
}

async function begin(app) {
  const start = await app.request('/admin/auth/oidc/start?returnTo=/models')
  assert.equal(start.status, 302)
  const authorize = start.headers.get('location')
  const cookie = start.headers.getSetCookie()[0].split(';')[0]
  const response = await fetch(authorize, { redirect: 'manual', signal: AbortSignal.timeout(5000) })
  assert.equal(response.status, 302)
  return { callback: new URL(response.headers.get('location')), cookie, authorize: new URL(authorize) }
}

test('OIDC wire flow verifies signed claims, PKCE, browser binding, identity status and SSO policy', { skip: !databaseURL, timeout: 120_000 }, async t => {
  const idp = await identityProvider(t)
  const app = await gateway(t, {
    MODELPORT_OIDC_ISSUER: idp.url,
    MODELPORT_OIDC_CLIENT_ID: 'modelport-assurance',
    MODELPORT_OIDC_CLIENT_SECRET: idp.clientSecret,
    MODELPORT_OIDC_ALLOW_INSECURE_HTTP: '1',
  })
  let admin = await adminSession(app)
  let failures = 0
  const scenario = (name, fn) => t.test(name, async () => {
    try { await fn() } catch (error) { failures++; throw error }
  })
  const user = await createUser(app, admin, 'oidc')
  const baseClaims = { email: user.email, preferred_username: user.username }
  async function complete(grant, cookie = grant.cookie) {
    return app.request(`${grant.callback.pathname}${grant.callback.search}`, { headers: { cookie } })
  }
  await scenario('SSO-only startup refuses a deployment without a linked administrator', async () => {
    await app.stop()
    await assert.rejects(app.start({ MODELPORT_PASSWORD_LOGIN_ENABLED: '0' }), /isolated gateway must start/)
    assert.match(app.logs(), /active administrator already linked/)
    await app.start()
    admin = await adminSession(app)
  })
  await scenario('binds a verified identity, then rejects callback replay and cookie use on the data plane', async () => {
    idp.select({ claims: baseClaims })
    const flow = await begin(app)
    const response = await complete(flow)
    assert.equal(response.headers.get('location'), '/models')
    const cookie = response.headers.getSetCookie().find(value => value.startsWith('modelport_admin_session=')).split(';')[0]
    assert.equal((await app.request('/admin/auth/me', { headers: { cookie } })).status, 200)
    assert.equal((await app.request('/v1/models', { headers: { cookie } })).status, 401)
    assert.match((await complete(flow)).headers.get('location'), /oidc_error=invalid_state/)
  })
  await scenario('rejects another browser before attempting the token exchange', async () => {
    const flow = await begin(app)
    const calls = idp.tokenCalls()
    assert.match((await complete(flow, 'modelport_oidc_flow=wrong-browser')).headers.get('location'), /invalid_state/)
    assert.equal(idp.tokenCalls(), calls)
  })
  for (const [label, selected] of [
    ['wrong issuer', { claims: { iss: 'https://wrong.example.test' } }],
    ['wrong audience', { claims: { aud: 'another-client' } }],
    ['expired token', { claims: { exp: 1 } }],
    ['wrong nonce', { claims: { nonce: 'incorrect-nonce' } }],
    ['forged signature', { badSignature: true }],
    ['wrong access-token hash', { badAccessHash: true }],
  ]) await scenario(`rejects ${label}`, async () => {
    idp.select({ ...selected, claims: { ...baseClaims, ...selected.claims } })
    const response = await complete(await begin(app))
    assert.match(response.headers.get('location'), /oidc_error=token_invalid/)
    assert.ok(!response.headers.getSetCookie().some(value => value.startsWith('modelport_admin_session=')))
  })
  await scenario('requires signed assurance and rejects password fallback when SSO-only is configured', async () => {
    const promoted = await app.request(`/admin/users/${user.id}`, { method: 'PUT', headers: { cookie: admin, 'x-modelport-csrf': '1' }, data: { role: 'admin' } })
    assert.equal(promoted.status, 200)
    await app.stop()
    await app.start({ MODELPORT_PASSWORD_LOGIN_ENABLED: '0', MODELPORT_OIDC_REQUIRED_ACR: 'urn:modelport:assurance:mfa' })
    assert.equal((await (await app.request('/admin/auth/methods')).json()).passwordEnabled, false)
    assert.equal((await app.request('/admin/auth/login', { data: { username: 'assurance_admin', password: app.password } })).status, 403)
    for (const acr of [undefined, 'urn:lower-assurance']) {
      idp.select({ claims: { ...baseClaims, acr } })
      const flow = await begin(app)
      assert.equal(flow.authorize.searchParams.get('acr_values'), 'urn:modelport:assurance:mfa')
      assert.match((await complete(flow)).headers.get('location'), /oidc_error=token_invalid/)
    }
    idp.select({ claims: baseClaims })
    assert.equal((await complete(await begin(app))).headers.get('location'), '/models')
  })
  await scenario('a disabled linked account cannot obtain a new console session', async () => {
    await app.stop()
    await app.start()
    admin = await adminSession(app)
    const disabled = await app.request(`/admin/users/${user.id}`, { method: 'PUT', headers: { cookie: admin, 'x-modelport-csrf': '1' }, data: { status: 'disabled' } })
    assert.equal(disabled.status, 200)
    idp.select({ claims: baseClaims })
    assert.match((await complete(await begin(app))).headers.get('location'), /account_not_authorized/)
  })
  if (failures === 0) await evidence('oidc', { signingAlgorithm: 'RS256', pkce: 'S256', confidentialClient: true,
    issuerAudienceNonceExpirySignatureAndAccessHashChecked: true, ssoOnlyAndSignedAssuranceChecked: true,
    browserBindingReplayAndDisabledIdentityChecked: true, realIdentityProvider: false })
})
