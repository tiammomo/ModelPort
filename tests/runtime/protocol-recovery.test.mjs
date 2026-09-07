import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { promisify } from 'node:util'
import { test } from 'node:test'
import { adminSession, createUser, databaseURL, evidence, gateway } from './support.mjs'
import { upstream } from './upstream.mjs'

const execute = promisify(execFile)
const model = 'custom:assurance-model'
const message = { model, max_tokens: 32, messages: [{ role: 'user', content: 'Synthetic acceptance: reply OK.' }] }

test('gateway protocol, bounded concurrent load, cancellation, database outage and restore', { skip: !databaseURL, timeout: 180_000 }, async t => {
  const mock = await upstream(t)
  const app = await gateway(t, { CUSTOM_OPENAI_BASE_URL: `${mock.url}/v1`, MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS: '2', MODELPORT_DATABASE_ACQUIRE_TIMEOUT_SECS: '2' }, `
[providers.custom]
protocol = "openai-compat"
base_url = "${mock.url}/v1"
api_key_env = "CUSTOM_OPENAI_API_KEY"
default_model = "assurance-model"
models = ["assurance-model"]
[providers.custom.model_profile_defaults]
reasoning_replay = "none"
[providers.custom.tool_use]
supported = true
response_validation = "strict"
`)
  let activeDatabaseURL = databaseURL
  let cookie = await adminSession(app)
  const user = await createUser(app, cookie, 'protocol')
  const response = await app.request('/admin/api-keys', { headers: { cookie, 'x-modelport-csrf': '1' },
    data: { userId: user.id, name: 'assurance-client', allowedProviders: ['custom'], allowedModels: ['assurance-model'] } })
  assert.equal(response.status, 200)
  const key = await response.json()
  const apiKey = key.key
  assert.ok(apiKey, 'one-time API key must be returned')
  const headers = { 'x-api-key': apiKey, 'x-modelport-traffic-class': 'synthetic' }
  const writeHeaders = () => ({ cookie, 'x-modelport-csrf': '1' })
  for (const path of ['/v1/messages', '/v1/chat/completions']) {
    await t.test(`${path}: text, live streaming and tool-result round trip`, async () => {
      let response = await app.request(path, { headers, data: message })
      assert.equal(response.status, 200, 'synthetic text request')
      const text = await response.json()
      assert.equal(path === '/v1/messages' ? text.content[0].text : text.choices[0].message.content, 'OK')
      response = await app.request(path, { headers, data: { ...message, stream: true } })
      assert.equal(response.status, 200)
      const stream = await response.text()
      assert.match(stream, /OK/)
      assert.match(stream, path === '/v1/messages' ? /event: message_stop/ : /data: \[DONE\]/)
      const definition = { name: 'weather', description: 'Synthetic weather', parameters: { type: 'object', properties: { city: { type: 'string' } }, required: ['city'] } }
      const tools = path === '/v1/messages'
        ? [{ name: definition.name, description: definition.description, input_schema: definition.parameters }]
        : [{ type: 'function', function: definition }]
      response = await app.request(path, { headers, data: { ...message, tools } })
      assert.equal(response.status, 200)
      const completion = await response.json()
      let messages
      if (path === '/v1/messages') {
        const call = completion.content.find(block => block.type === 'tool_use')
        assert.deepEqual(call.input, { city: 'Shanghai' })
        messages = [...message.messages, { role: 'assistant', content: completion.content }, { role: 'user', content: [{ type: 'tool_result', tool_use_id: call.id, content: 'Sunny' }] }]
      } else {
        const assistant = completion.choices[0].message
        assert.deepEqual(JSON.parse(assistant.tool_calls[0].function.arguments), { city: 'Shanghai' })
        messages = [...message.messages, assistant, { role: 'tool', tool_call_id: assistant.tool_calls[0].id, content: 'Sunny' }]
      }
      response = await app.request(path, { headers, data: { ...message, tools, messages } })
      assert.equal(response.status, 200, (await response.clone().json()).error?.message)
      const result = await response.json()
      assert.equal(path === '/v1/messages' ? result.content[0].text : result.choices[0].message.content, 'OK')
      response = await app.request(path, { headers, data: { ...message, tools, stream: true } })
      assert.equal(response.status, 200)
      const toolStream = await response.text()
      assert.match(toolStream, /Shanghai/)
      assert.match(toolStream, path === '/v1/messages' ? /event: message_stop/ : /data: \[DONE\]/)
    })
  }
  await t.test('rejects forged identity, data-plane keys on the console and writes without CSRF', async () => {
    const calls = mock.calls()
    assert.equal((await app.request('/v1/models', { headers: { 'x-api-key': 'incorrect-key' } })).status, 401)
    assert.equal((await app.request('/admin/users', { headers })).status, 401)
    assert.equal((await app.request(`/admin/users/${user.id}`, { method: 'PUT', headers: { cookie }, data: { email: user.email } })).status, 403)
    assert.equal(mock.calls(), calls)
  })
  await t.test('40 distinct users sustain synthetic streaming load with bounded outcomes', async () => {
    const durationSeconds = Number(process.env.MODELPORT_ASSURANCE_LOAD_SECONDS || 10)
    assert.ok(Number.isInteger(durationSeconds) && durationSeconds >= 1 && durationSeconds <= 120)
    const clients = []
    for (let i = 0; i < 40; i++) {
      const owner = await createUser(app, cookie, `load${i}`)
      const response = await app.request('/admin/api-keys', { headers: writeHeaders(),
        data: { userId: owner.id, name: `assurance-load-${i}`, allowedProviders: ['custom'], allowedModels: ['assurance-model'] } })
      assert.equal(response.status, 200)
      clients.push((await response.json()).key)
    }
    const started = performance.now()
    const results = []
    // Ten paced waves keep the request budget fixed even for a longer soak.
    // Each request goes through real auth, policy, PostgreSQL and SSE handling.
    for (let wave = 0; wave < 10; wave++) {
      const batch = await Promise.all(clients.map(async (token, index) => {
        const start = performance.now()
        const response = await app.request(index % 2 ? '/v1/messages' : '/v1/chat/completions', {
          headers: { ...headers, 'x-api-key': token }, data: { ...message, stream: true },
        })
        const content = await response.text()
        assert.ok([200, 429].includes(response.status), 'load must succeed or reject admission explicitly')
        if (response.status === 200) assert.match(content, index % 2 ? /event: message_stop/ : /data: \[DONE\]/)
        else assert.ok(Number(response.headers.get('retry-after')) > 0)
        return { status: response.status, ms: performance.now() - start }
      }))
      results.push(...batch)
      const untilNext = started + (wave + 1) * durationSeconds * 100 - performance.now()
      if (untilNext > 0) await delay(untilNext)
    }
    assert.ok(results.some(result => result.status === 200))
    const sorted = results.map(result => result.ms).sort((a, b) => a - b)
    const accepted = results.filter(result => result.status === 200).map(result => result.ms).sort((a, b) => a - b)
    await evidence('synthetic-load', { clients: 40, distinctUsers: 40, requests: results.length,
      elapsedMs: performance.now() - started, success: results.filter(result => result.status === 200).length,
      rejected: results.filter(result => result.status === 429).length,
      p50Ms: sorted[Math.ceil(sorted.length * 0.50) - 1], p95Ms: sorted[Math.ceil(sorted.length * 0.95) - 1],
      successP50Ms: accepted[Math.ceil(accepted.length * 0.50) - 1], successP95Ms: accepted[Math.ceil(accepted.length * 0.95) - 1],
      latencyScope: 'whole-response-including-stream', localExecutionSlots: 1,
      upstreamPeak: mock.peak(), realProvider: false })
  })
  await t.test('truncated SSE is an error and client cancellation releases the upstream stream', async () => {
    mock.setMode('truncated')
    const response = await app.request('/v1/messages', { headers, data: { ...message, stream: true } })
    assert.equal(response.status, 200)
    const truncated = await response.text()
    assert.match(truncated, /event: error/)
    assert.doesNotMatch(truncated, /event: message_stop/)
    mock.setMode('hold')
    const abort = new AbortController()
    const held = await app.request('/v1/messages', { headers, data: { ...message, stream: true }, signal: abort.signal })
    const reader = held.body.getReader()
    await reader.read()
    abort.abort()
    await reader.cancel().catch(() => {})
    for (let i = 0; i < 50 && mock.active(); i++) await delay(100)
    assert.equal(mock.active(), 0, 'cancelled stream releases its upstream connection')
    mock.setMode('normal')
  })
  await t.test('forced process restart invalidates sessions and preserves scoped keys', async () => {
    await app.stop('SIGKILL')
    await app.start()
    assert.equal((await app.request('/admin/auth/me', { headers: { cookie } })).status, 401)
    assert.equal((await app.request('/v1/models', { headers })).status, 200)
    cookie = await adminSession(app)
  })
  const container = process.env.MODELPORT_RUNTIME_TEST_POSTGRES_CONTAINER
  await t.test('database outage fails closed, then backup/restore recovers auth and ledger state', { skip: !container, timeout: 90_000 }, async () => {
    assert.match(container, /^modelport-assurance-[0-9]+-[0-9]+$/)
    const inspect = await execute('docker', ['inspect', '--format', '{{index .Config.Labels "io.modelport.test"}}', container])
    assert.equal(inspect.stdout.trim(), 'assurance')
    const docker = args => execute('docker', ['exec', container, ...args], { maxBuffer: 8 * 1024 * 1024 })
    const sql = (query, database = 'modelport_assurance') => docker(['psql', '-U', 'modelport', '-d', database, '-Atc', query])
    const calls = mock.calls()
    await execute('docker', ['stop', '--time', '5', container])
    try {
      assert.equal((await app.request('/readyz', { headers: { 'x-api-key': app.token } })).status, 503)
      const failed = await app.request('/v1/messages', { headers, data: message })
      assert.ok(failed.status >= 500)
      assert.equal(mock.calls(), calls, 'storage outage must prevent unrecorded model egress')
    } finally { await execute('docker', ['start', container]) }
    for (let i = 0; i < 100; i++) {
      try { await sql('SELECT 1'); break } catch { await delay(100) }
    }
    assert.equal((await app.request('/readyz', { headers: { 'x-api-key': app.token } })).status, 200)
    await app.stop()
    const fingerprintQuery = "SELECT namespace || ':' || md5(document::text) FROM modelport_state ORDER BY namespace"
    const fingerprint = (await sql(fingerprintQuery)).stdout
    const ledger = (await sql('SELECT count(*) FROM modelport_gateway_requests')).stdout
    const dump = await execute('docker', ['exec', container, 'pg_dump', '-U', 'modelport', '-d', 'modelport_assurance', '-Fc', '--no-owner', '--no-privileges'], { encoding: 'buffer', maxBuffer: 16 * 1024 * 1024 })
    const temporary = await mkdtemp(join(tmpdir(), 'modelport-restore-'))
    try {
      const archive = join(temporary, 'postgres.dump')
      await writeFile(archive, dump.stdout, { mode: 0o600 })
      await docker(['createdb', '-U', 'modelport', 'modelport_assurance_restore'])
      await execute('docker', ['cp', archive, `${container}:/tmp/assurance.dump`])
      await docker(['pg_restore', '-U', 'modelport', '-d', 'modelport_assurance_restore', '--exit-on-error', '--no-owner', '--no-privileges', '/tmp/assurance.dump'])
      assert.equal((await sql(fingerprintQuery, 'modelport_assurance_restore')).stdout, fingerprint)
      assert.equal((await sql('SELECT count(*) FROM modelport_gateway_requests', 'modelport_assurance_restore')).stdout, ledger)
      const restored = new URL(databaseURL); restored.pathname = '/modelport_assurance_restore'
      activeDatabaseURL = restored.href
      await app.start({ MODELPORT_DATABASE_URL: restored.href })
      cookie = await adminSession(app)
      assert.equal((await app.request('/v1/models', { headers })).status, 200)
      if (process.env.MODELPORT_ROLLBACK_TEST_BINARY) {
        await app.stop()
        await app.start({ MODELPORT_DATABASE_URL: restored.href }, process.env.MODELPORT_ROLLBACK_TEST_BINARY)
        assert.equal((await app.request('/readyz', { headers: { 'x-api-key': app.token } })).status, 200)
        assert.equal((await app.request('/v1/models', { headers })).status, 200)
      }
      await evidence('recovery', { databaseOutageRejectedBeforeEgress: true, stateFingerprintMatched: true, ledgerRows: Number(ledger.trim()), restoredLoginPassed: true,
        rollbackBinaryPassed: Boolean(process.env.MODELPORT_ROLLBACK_TEST_BINARY), productionRtoRpoVerified: false })
    } finally { await rm(temporary, { recursive: true, force: true }) }
  })
  await t.test('revoking a client key remains effective after restart', async () => {
    cookie = await adminSession(app)
    const revoked = await app.request(`/admin/api-keys/${key.id}`, { method: 'DELETE', headers: writeHeaders() })
    assert.equal(revoked.status, 200)
    await app.stop()
    await app.start({ MODELPORT_DATABASE_URL: activeDatabaseURL })
    assert.equal((await app.request('/v1/models', { headers })).status, 401)
  })
})
