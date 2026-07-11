import type { ApiKey } from '@/types'

export type ApiKeyStatusFilter = 'all' | ApiKey['status'] | 'expiring' | 'expired'

export interface ApiKeyFilters {
  search: string
  status: ApiKeyStatusFilter
  group: string
  allGroupValue: string
  noGroupValue: string
}

export type ApiKeyExpiryState = 'never' | 'valid' | 'expiring' | 'expired'

const EXPIRING_WINDOW_MS = 7 * 24 * 60 * 60 * 1000

export function apiKeyExpiryState(apiKey: Pick<ApiKey, 'expiresAt'>, now = Date.now()): ApiKeyExpiryState {
  if (!apiKey.expiresAt) return 'never'

  const expiresAt = /^\d+$/.test(apiKey.expiresAt)
    ? Number(apiKey.expiresAt)
    : new Date(apiKey.expiresAt).getTime()
  if (!Number.isFinite(expiresAt)) return 'never'
  if (expiresAt <= now) return 'expired'
  if (expiresAt - now <= EXPIRING_WINDOW_MS) return 'expiring'
  return 'valid'
}

export function filterApiKeys(
  apiKeys: readonly ApiKey[],
  filters: ApiKeyFilters,
  now = Date.now(),
): ApiKey[] {
  const query = filters.search.trim().toLocaleLowerCase()

  return apiKeys.filter((apiKey) => {
    const haystack = [
      apiKey.name,
      apiKey.username,
      apiKey.userId,
      apiKey.keyPreview,
      apiKey.keyPrefix,
      apiKey.group,
      apiKey.teamName,
    ].filter(Boolean).join(' ').toLocaleLowerCase()

    if (query && !haystack.includes(query)) return false
    if (filters.group === filters.noGroupValue && apiKey.group) return false
    if (filters.group !== filters.allGroupValue
      && filters.group !== filters.noGroupValue
      && apiKey.group !== filters.group) return false

    if (filters.status === 'all') return true
    if (filters.status === 'expired' || filters.status === 'expiring') {
      return apiKey.status === 'active' && apiKeyExpiryState(apiKey, now) === filters.status
    }
    return apiKey.status === filters.status
  })
}

export function isApiKeyFilterActive(filters: Pick<ApiKeyFilters, 'search' | 'status' | 'group' | 'allGroupValue'>): boolean {
  return Boolean(filters.search.trim()) || filters.status !== 'all' || filters.group !== filters.allGroupValue
}
