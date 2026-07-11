import { describe, expect, it } from 'vitest'
import type { ApiKey } from '@/types'
import { apiKeyExpiryState, filterApiKeys, isApiKeyFilterActive } from './api-key-view'

const NOW = Date.UTC(2026, 6, 11, 12)

function key(overrides: Partial<ApiKey> = {}): ApiKey {
  return {
    id: 'key_1',
    userId: 'usr_1',
    username: 'alice',
    name: 'Claude Code',
    keyPrefix: 'mp_live_1234',
    group: '研发',
    teamName: '平台团队',
    createdAt: new Date(NOW).toISOString(),
    lastUsedAt: null,
    expiresAt: null,
    status: 'active',
    ...overrides,
  }
}

describe('apiKeyExpiryState', () => {
  it('distinguishes expired, expiring, valid and non-expiring keys', () => {
    expect(apiKeyExpiryState(key(), NOW)).toBe('never')
    expect(apiKeyExpiryState(key({ expiresAt: String(NOW - 1) }), NOW)).toBe('expired')
    expect(apiKeyExpiryState(key({ expiresAt: String(NOW + 2 * 86_400_000) }), NOW)).toBe('expiring')
    expect(apiKeyExpiryState(key({ expiresAt: String(NOW + 8 * 86_400_000) }), NOW)).toBe('valid')
  })
})

describe('filterApiKeys', () => {
  const keys = [
    key(),
    key({ id: 'key_2', name: 'CI', username: 'bob', group: null, teamName: null, status: 'revoked' }),
    key({ id: 'key_3', name: 'Release', expiresAt: String(NOW + 86_400_000) }),
  ]

  const baseFilters = {
    search: '',
    status: 'all' as const,
    group: '__all__',
    allGroupValue: '__all__',
    noGroupValue: '__none__',
  }

  it('searches identity and ownership fields case-insensitively', () => {
    expect(filterApiKeys(keys, { ...baseFilters, search: 'BOB' }, NOW).map((item) => item.id)).toEqual(['key_2'])
    expect(filterApiKeys(keys, { ...baseFilters, search: '平台团队' }, NOW)).toHaveLength(2)
  })

  it('supports no-group and expiry risk filters', () => {
    expect(filterApiKeys(keys, { ...baseFilters, group: '__none__' }, NOW).map((item) => item.id)).toEqual(['key_2'])
    expect(filterApiKeys(keys, { ...baseFilters, status: 'expiring' }, NOW).map((item) => item.id)).toEqual(['key_3'])
  })
})

it('detects active filters without treating whitespace as a query', () => {
  expect(isApiKeyFilterActive({ search: '  ', status: 'all', group: '__all__', allGroupValue: '__all__' })).toBe(false)
  expect(isApiKeyFilterActive({ search: '', status: 'revoked', group: '__all__', allGroupValue: '__all__' })).toBe(true)
})
