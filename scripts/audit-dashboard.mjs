#!/usr/bin/env node

import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const dashboard = resolve(root, 'dashboard')
const policyPath = resolve(dashboard, 'npm-audit-exceptions.json')
const policy = JSON.parse(readFileSync(policyPath, 'utf8'))

if (policy.schemaVersion !== 1 || !Array.isArray(policy.exceptions)) {
  throw new Error('dashboard/npm-audit-exceptions.json has an unsupported schema')
}

const audit = spawnSync(
  'npm',
  ['audit', '--json', '--audit-level=low', '--registry=https://registry.npmjs.org'],
  { cwd: dashboard, encoding: 'utf8' },
)

if (audit.error) {
  throw audit.error
}

let report
try {
  report = JSON.parse(audit.stdout)
} catch {
  process.stderr.write(audit.stderr)
  process.stderr.write(audit.stdout)
  throw new Error('npm audit did not return valid JSON')
}

const today = new Date().toISOString().slice(0, 10)
const observed = new Map()

for (const [packageName, vulnerability] of Object.entries(report.vulnerabilities ?? {})) {
  for (const via of vulnerability.via ?? []) {
    if (typeof via !== 'object' || typeof via.url !== 'string') continue
    const id = via.url.split('/').at(-1)
    observed.set(`${packageName}:${id}`, {
      id,
      package: packageName,
      severity: via.severity,
      title: via.title,
    })
  }
}

const allowed = new Map(
  policy.exceptions.map((exception) => [`${exception.package}:${exception.id}`, exception]),
)
const failures = []

for (const [key, finding] of observed) {
  const exception = allowed.get(key)
  if (!exception) {
    failures.push(`${finding.package} ${finding.id} (${finding.severity}): ${finding.title}`)
    continue
  }
  if (exception.expires < today) {
    failures.push(
      `${finding.package} ${finding.id}: exception expired on ${exception.expires}`,
    )
    continue
  }
  process.stdout.write(
    `accepted ${finding.package} ${finding.id} through ${exception.expires}: ${exception.rationale}\n`,
  )
}

for (const [key, exception] of allowed) {
  if (!observed.has(key)) {
    failures.push(
      `${exception.package} ${exception.id}: stale exception is not present in the audit report`,
    )
  }
}

if (failures.length > 0) {
  process.stderr.write(`dashboard dependency audit failed:\n- ${failures.join('\n- ')}\n`)
  process.exit(1)
}

const total = report.metadata?.vulnerabilities?.total ?? observed.size
process.stdout.write(
  `dashboard dependency audit passed (${total} reported vulnerability entries, all explicitly reviewed)\n`,
)
