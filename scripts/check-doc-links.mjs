#!/usr/bin/env node

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, extname, isAbsolute, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const ignoredDirectories = new Set([
  '.git',
  'node_modules',
  'target',
  'dist',
  'playwright-report',
  'test-results',
])
const markdownFiles = []

function walk(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (!ignoredDirectories.has(entry.name)) walk(join(directory, entry.name))
      continue
    }
    if (entry.isFile() && extname(entry.name).toLowerCase() === '.md') {
      markdownFiles.push(join(directory, entry.name))
    }
  }
}

walk(root)

const failures = []
const linkPattern = /!?(?:\[[^\]]*\])\((<[^>]+>|[^\s)]+)(?:\s+['"][^'"]*['"])?\)/g

for (const markdownFile of markdownFiles) {
  const source = readFileSync(markdownFile, 'utf8')
  for (const match of source.matchAll(linkPattern)) {
    let target = match[1]
    if (target.startsWith('<') && target.endsWith('>')) target = target.slice(1, -1)
    if (/^(?:https?:|mailto:|tel:|data:|#)/i.test(target)) continue

    const pathPart = target.split('#', 1)[0].split('?', 1)[0]
    if (!pathPart) continue
    let decoded
    try {
      decoded = decodeURIComponent(pathPart)
    } catch {
      failures.push(`${markdownFile.slice(root.length + 1)}: invalid encoded link ${target}`)
      continue
    }
    const destination = isAbsolute(decoded)
      ? resolve(root, `.${decoded}`)
      : resolve(dirname(markdownFile), decoded)
    if (!existsSync(destination)) {
      failures.push(
        `${markdownFile.slice(root.length + 1)}: missing target ${target}`,
      )
      continue
    }
    // Explicitly touch metadata so broken symlinks and inaccessible targets
    // fail the same gate instead of silently passing existsSync on platforms
    // with unusual filesystem behavior.
    try {
      statSync(destination)
    } catch (error) {
      failures.push(
        `${markdownFile.slice(root.length + 1)}: unreadable target ${target} (${error.message})`,
      )
    }
  }
}

if (failures.length > 0) {
  for (const failure of failures) console.error(failure)
  process.exit(1)
}

console.log(`Markdown links valid in ${markdownFiles.length} file(s)`)
