import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { once } from 'node:events'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

export const databaseURL = process.env.MODELPORT_RUNTIME_TEST_DATABASE_URL
export const root = resolve(import.meta.dirname, '../..')
export const secret = () => randomBytes(24).toString('base64url')

export async function server(handler) {
  const http = createServer((req, res) => {
    Promise.resolve(handler(req, res)).catch(() => {
      if (!res.headersSent) res.writeHead(500)
      res.end()
    })
  })
  http.listen(0, '127.0.0.1')
  await once(http, 'listening')
  return {
    url: `http://127.0.0.1:${http.address().port}`,
    async close() {
      const closed = once(http, 'close')
      http.close()
      http.closeAllConnections()
      await closed
    },
  }
}

export async function body(req) {
  const chunks = []
  let bytes = 0
  for await (const chunk of req) {
    bytes += chunk.length
    assert.ok(bytes < 1024 * 1024, 'fixture request is bounded')
    chunks.push(chunk)
  }
  return Buffer.concat(chunks).toString()
}

export function json(res, value, status = 200) {
  res.writeHead(status, { 'content-type': 'application/json' })
  res.end(JSON.stringify(value))
}

export async function gateway(t, overrides = {}, config) {
  assert.ok(databaseURL, 'MODELPORT_RUNTIME_TEST_DATABASE_URL is required')
  const database = new URL(databaseURL)
  assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(database.hostname), 'runtime tests require loopback PostgreSQL')
  assert.match(database.pathname, /^\/modelport_assurance(?:_[a-z0-9_]+)?$/, 'use a dedicated modelport_assurance database')
  const runtime = await mkdtemp(join(tmpdir(), 'modelport-assurance-'))
  const reservation = await server((_req, res) => res.end())
  const url = reservation.url
  await reservation.close()
  // Keep test credentials stable across restarts of this isolated database.
  const password = 'Assurance_local_7kP9wR4zM2'
  const token = secret()
  const env = {
    PATH: process.env.PATH,
    HOME: runtime,
    MODELPORT_ENV_FILE: join(runtime, 'absent.env'),
    MODELPORT_CONFIG: join(runtime, 'absent.toml'),
    MODELPORT_DATABASE_URL: databaseURL,
    MODELPORT_DATABASE_TLS_MODE: 'disable',
    MODELPORT_BIND: new URL(url).host,
    MODELPORT_ADMIN_USERNAME: 'assurance_admin',
    MODELPORT_ADMIN_PASSWORD: password,
    MODELPORT_AUTH_TOKEN: token,
    MODELPORT_DEFAULT_PROVIDER: 'custom',
    CUSTOM_OPENAI_BASE_URL: 'http://127.0.0.1:9/v1',
    CUSTOM_OPENAI_API_KEY: secret(),
    CUSTOM_OPENAI_MODEL: 'assurance-model',
    ...overrides,
  }
  if (config) {
    env.MODELPORT_CONFIG = join(runtime, 'config.toml')
    await writeFile(env.MODELPORT_CONFIG, config, { mode: 0o600 })
  }
  if (env.MODELPORT_OIDC_ISSUER) env.MODELPORT_OIDC_REDIRECT_URI = `${url}/admin/auth/oidc/callback`
  const binary = process.env.MODELPORT_TEST_BINARY || join(root, 'target/debug/model-port')
  let child
  let output = ''
  let launchError
  async function stop(signal = 'SIGTERM') {
    if (!child || child.exitCode !== null || child.signalCode !== null) return
    const exited = once(child, 'exit')
    child.kill(signal)
    const timeout = setTimeout(() => child.kill('SIGKILL'), 10_000)
    try { await exited } finally { clearTimeout(timeout) }
  }
  async function start(extra = {}, executable = binary) {
    output = ''
    child = spawn(executable, [], { cwd: root, env: { ...env, ...extra }, stdio: ['ignore', 'pipe', 'pipe'] })
    child.on('error', error => { launchError = error })
    for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { output = (output + chunk).slice(-20_000) })
    for (let i = 0; i < 160; i++) {
      if (launchError) throw launchError
      assert.equal(child.exitCode, null, 'isolated gateway must start; inspect its configuration')
      try {
        const response = await fetch(`${url}/livez`, { signal: AbortSignal.timeout(500) })
        if (response.ok) return
      } catch { /* wait for the listener */ }
      await delay(100)
    }
    throw new Error('isolated gateway did not become live')
  }
  t.after(async () => { await stop(); await rm(runtime, { recursive: true, force: true }) })
  await start()
  return {
    url, token, password, start, stop,
    logs: () => output,
    async request(path, { data, headers, method, ...options } = {}) {
      return fetch(`${url}${path}`, {
        method: method || (data ? 'POST' : 'GET'),
        headers: { ...(data ? { 'content-type': 'application/json' } : {}), ...headers },
        body: data ? JSON.stringify(data) : undefined,
        signal: AbortSignal.timeout(15_000),
        redirect: 'manual',
        ...options,
      })
    },
  }
}

export async function evidence(name, value) {
  const directory = process.env.MODELPORT_ASSURANCE_OUTPUT_DIR
  if (!directory) return
  await mkdir(directory, { recursive: true, mode: 0o700 })
  await writeFile(join(directory, `${name}.json`), JSON.stringify({
    schemaVersion: 1, scope: 'isolated-synthetic', generatedAt: new Date().toISOString(),
    commit: spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).stdout.trim(),
    sourceState: spawnSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).stdout.trim() ? 'dirty' : 'clean',
    ...value,
  }, null, 2) + '\n', { mode: 0o600 })
}

export async function adminSession(app) {
  const response = await app.request('/admin/auth/login', { data: { username: 'assurance_admin', password: app.password } })
  assert.equal(response.status, 200, 'fixture administrator login')
  return response.headers.getSetCookie()[0].split(';')[0]
}

export async function createUser(app, cookie, label) {
  const username = `assurance_${label}_${secret().slice(0, 8).toLowerCase().replace(/[^a-z0-9]/g, 'x')}`
  const response = await app.request('/admin/users', {
    headers: { cookie, 'x-modelport-csrf': '1' },
    data: { username, email: `${username}@example.test`, password: secret(), role: 'user', status: 'active' },
  })
  assert.equal(response.status, 200, 'create isolated user')
  return response.json()
}
