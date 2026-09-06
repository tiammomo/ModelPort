import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, mkdirSync, mkdtempSync, readdirSync, rmSync, utimesSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

const library = fileURLToPath(new URL('../../scripts/lib.sh', import.meta.url))

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'modelport-freshness-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  for (const path of [
    'src/main.rs', 'crates/ops-protocol/src/lib.rs', 'resources/catalog/models.json',
    'resources/schemas/adapter.json', 'migrations/0001.sql', 'Cargo.toml',
    'Cargo.lock', 'rust-toolchain.toml', 'target/release/model-port',
  ]) {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    writeFileSync(join(root, path), '')
  }
  function age(path) {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name)
      if (entry.isDirectory()) age(child)
      utimesSync(child, new Date('2020-01-01'), new Date('2020-01-01'))
    }
  }
  age(root)
  const binary = join(root, 'target/release/model-port')
  chmodSync(binary, 0o755)
  utimesSync(binary, new Date('2025-01-01'), new Date('2025-01-01'))
  const isFresh = () => {
    const result = spawnSync('bash', ['-c',
      'source "$1"; ROOT_DIR="$2"; RELEASE_BIN="$3"; release_is_fresh',
      'freshness-test', library, root, binary,
    ], { encoding: 'utf8' })
    assert.ifError(result.error)
    assert.ok(result.status === 0 || result.status === 1, result.stderr)
    return result.status === 0
  }
  return { root, binary, isFresh }
}

test('an unchanged checkout reuses the executable; a missing executable rebuilds', t => {
  const { binary, isFresh } = fixture(t)
  assert.equal(isFresh(), true)
  rmSync(binary)
  assert.equal(isFresh(), false)
})

for (const path of ['resources/catalog/models.json', 'resources/schemas/adapter.json',
  'crates/ops-protocol/src/lib.rs', 'migrations/0001.sql', 'src/main.rs']) {
  test(`changing ${path} rebuilds the executable`, t => {
    const { root, isFresh } = fixture(t)
    assert.equal(isFresh(), true)
    utimesSync(join(root, path), new Date('2030-01-01'), new Date('2030-01-01'))
    assert.equal(isFresh(), false)
  })
}

test('removing a resource invalidates the executable through its parent directory', t => {
  const { root, isFresh } = fixture(t)
  rmSync(join(root, 'resources/catalog/models.json'))
  assert.equal(isFresh(), false)
})

test('an incomplete checkout cannot be treated as a fresh build', t => {
  const { root, isFresh } = fixture(t)
  rmSync(join(root, 'resources'), { recursive: true })
  assert.equal(isFresh(), false)
})
