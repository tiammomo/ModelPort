const CHUNK_RELOAD_AT_KEY = 'modelport_chunk_reload_at'
const CHUNK_RELOAD_GUARD_MS = 30_000

const CHUNK_LOAD_ERROR_PATTERNS = [
  'failed to fetch dynamically imported module',
  'error loading dynamically imported module',
  'importing a module script failed',
  'unable to preload css',
]

export function isChunkLoadError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error || '')
  const normalized = message.toLowerCase()
  return CHUNK_LOAD_ERROR_PATTERNS.some((pattern) => normalized.includes(pattern))
}

export function reloadWithFreshAssets(): void {
  if (typeof window === 'undefined') return
  const url = new URL(window.location.href)
  url.searchParams.set('__modelport_reload', Date.now().toString(36))
  window.location.replace(url.toString())
}

export function installChunkRecovery(): void {
  if (typeof window === 'undefined') return

  window.addEventListener('vite:preloadError', (event) => {
    const now = Date.now()
    let lastReloadAt = 0
    try {
      lastReloadAt = Number(window.sessionStorage.getItem(CHUNK_RELOAD_AT_KEY) || 0)
    } catch {
      // Storage can be unavailable in hardened browser modes; one URL-busted
      // reload still gives the current deployment a chance to recover.
    }

    if (Number.isFinite(lastReloadAt) && now - lastReloadAt < CHUNK_RELOAD_GUARD_MS) return

    event.preventDefault()
    try {
      window.sessionStorage.setItem(CHUNK_RELOAD_AT_KEY, String(now))
    } catch {
      // See the read-side note above.
    }
    reloadWithFreshAssets()
  })
}
