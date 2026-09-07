import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

const scripts = fileURLToPath(new URL('../../scripts', import.meta.url))

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'modelport lifecycle '))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  cpSync(scripts, join(root, 'scripts'), { recursive: true })
  mkdirSync(join(root, '.modelport'))
  mkdirSync(join(root, 'bin'))
  const commands = join(root, 'commands.log')
  const env = { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`,
    MODELPORT_ENV_FILE: join(root, '.env'), MODELPORT_RUNTIME_DIR: join(root, '.modelport'),
    MODELPORT_PID_FILE: join(root, '.modelport/model-port.pid'),
    MODELPORT_TEST_COMMAND_LOG: commands,
  }
  writeFileSync(env.MODELPORT_ENV_FILE, 'MODELPORT_BIND=127.0.0.1:38082\nMODELPORT_AUTH_TOKEN=local-test-token\n')
  function executable(name, source) {
    writeFileSync(join(root, 'bin', name), `#!/usr/bin/env bash\n${source}\n`, { mode: 0o755 })
  }
  executable('curl', 'exit 1')
  executable('cargo', 'printf "%s|%s\\n" "$PWD" "$*" >> "$MODELPORT_TEST_COMMAND_LOG"')
  function run(args, legacy) {
    const result = spawnSync('bash', [join(root, 'scripts', legacy || 'dev.sh'), ...args], {
      cwd: tmpdir(), env, encoding: 'utf8', timeout: 5000,
    })
    assert.ifError(result.error)
    return result
  }
  return { root, env, run, executable, commands }
}

async function nativeProcess(t, root) {
  const binary = join(root, 'target/release/model-port')
  mkdirSync(dirname(binary), { recursive: true })
  copyFileSync('/bin/sleep', binary)
  chmodSync(binary, 0o755)
  const child = spawn(binary, ['120'], { stdio: 'ignore' })
  await once(child, 'spawn')
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, 'exit')
      child.kill('SIGKILL')
      await exited
    }
  })
  return { child, binary }
}

function alive(pid) {
  try { process.kill(pid, 0); return true } catch { return false }
}

test('help and invalid commands do not need configuration or launch Cargo', t => {
  const { run, env, commands } = fixture(t)
  rmSync(env.MODELPORT_ENV_FILE)
  assert.equal(run(['help']).status, 0)
  assert.equal(run(['statuz']).status, 1)
  assert.equal(run(['start', '--unexpected']).status, 1)
  assert.equal(existsSync(commands), false)
})

test('foreground, validation and Rust checks run in this checkout from another directory', t => {
  const { run, root, commands, env } = fixture(t)
  assert.equal(run([]).status, 0)
  env.MODELPORT_FORCE_BUILD = '1'
  assert.equal(run(['validate']).status, 0)
  assert.equal(run(['check', '--backend']).status, 0)
  assert.deepEqual(readFileSync(commands, 'utf8').trim().split('\n'), [
    `${root}|run --locked --bin model-port`,
    `${root}|run --locked --bin model-port -- config validate`,
    `${root}|fmt --all -- --check`,
    `${root}|test --locked --all-targets`,
    `${root}|clippy --locked --all-targets --all-features -- -D warnings`,
  ])
})

test('stop owns its checkout even when a stale PID and listener point to another checkout', async t => {
  const current = fixture(t)
  const other = fixture(t)
  const foreign = await nativeProcess(t, other.root)
  const own = await nativeProcess(t, current.root)
  writeFileSync(current.env.MODELPORT_PID_FILE, String(foreign.child.pid))
  current.executable('ss', `echo 'LISTEN 0 128 127.0.0.1:38082 0.0.0.0:* users:(("model-port",pid=${foreign.child.pid},fd=3))'`)
  const ownExit = once(own.child, 'exit')
  const result = current.run(['stop'])
  assert.equal(result.status, 0, result.stderr)
  await ownExit
  assert.equal(alive(foreign.child.pid), true)
  assert.equal(existsSync(current.env.MODELPORT_PID_FILE), false)
  assert.match(result.stdout, /does not belong to this checkout/)
})

test('legacy stop leaves a foreign listener and its process untouched', async t => {
  const current = fixture(t)
  const other = fixture(t)
  const foreign = await nativeProcess(t, other.root)
  current.executable('ss', `echo 'LISTEN 0 128 127.0.0.1:38082 0.0.0.0:* users:(("model-port",pid=${foreign.child.pid},fd=3))'`)
  assert.equal(current.run([], 'stop.sh').status, 0)
  assert.equal(alive(foreign.child.pid), true)
})

test('a replaced binary can still be stopped by executable ownership', async t => {
  const current = fixture(t)
  const own = await nativeProcess(t, current.root)
  rmSync(own.binary)
  const exited = once(own.child, 'exit')
  assert.equal(current.run(['stop']).status, 0)
  await exited
})

test('start refuses a second native process when the existing one is unhealthy', async t => {
  const current = fixture(t)
  const own = await nativeProcess(t, current.root)
  writeFileSync(current.env.MODELPORT_PID_FILE, String(own.child.pid))
  const result = current.run([], 'start.sh')
  assert.equal(result.status, 1)
  assert.match(result.stderr, /already has a running gateway/)
  assert.equal(alive(own.child.pid), true)
  assert.equal(existsSync(current.commands), false)
})

test('start identifies a responding endpoint outside this checkout without building', t => {
  const current = fixture(t)
  current.executable('curl', 'exit 0')
  const result = current.run(['start'])
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not managed by this checkout/)
  assert.equal(existsSync(current.commands), false)
})
