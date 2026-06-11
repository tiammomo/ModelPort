import type { User, CreateUserInput, UpdateUserInput, ApiKey } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay, nextMockId } from '@/lib/mock-mode'
import { mockApiKeys, mockUsers } from '@/mock'

export interface CreateApiKeyInput {
  userId: string
  username?: string
  name: string
  group?: string
}

export interface UpdateApiKeyInput {
  name?: string
  group?: string
  expiresAt?: string
  status?: ApiKey['status']
  ipRestricted?: boolean
  spendLimitUsd?: number
  rateLimited?: boolean
  fiveHourLimitUsd?: number
  dailyLimitUsd?: number
  weeklyLimitUsd?: number
  monthlyLimitUsd?: number
}

let mockUserStore = [...mockUsers]
let mockApiKeyStore = [...mockApiKeys]

function withApiKeyCounts(users: User[]) {
  return users.map((user) => ({
    ...user,
    apiKeyCount: mockApiKeyStore.filter((key) => key.userId === user.id && key.status === 'active').length,
  }))
}

export const usersService = {
  getUsers: (): Promise<User[]> =>
    isMockMode ? mockDelay(withApiKeyCounts(mockUserStore)) : api.get('/admin/users'),

  getUser: async (id: string): Promise<User> => {
    const users = isMockMode ? withApiKeyCounts(mockUserStore) : await api.get<User[]>('/admin/users')
    const user = users.find((item) => item.id === id)
    if (!user) throw new Error('用户不存在')
    return isMockMode ? mockDelay(user) : user
  },

  createUser: (data: CreateUserInput): Promise<User> => {
    if (!isMockMode) return api.post('/admin/users', data)
    const user: User = {
      id: nextMockId('usr'),
      username: data.username,
      email: data.email,
      role: data.role,
      status: data.status,
      createdAt: new Date().toISOString(),
      lastLoginAt: null,
      apiKeyCount: 0,
      requestCount24h: 0,
    }
    mockUserStore = [user, ...mockUserStore]
    return mockDelay(user)
  },

  updateUser: async (id: string, data: UpdateUserInput): Promise<User> => {
    if (isMockMode) {
      const user = mockUserStore.find((item) => item.id === id)
      if (!user) throw new Error('用户不存在')
      const next = { ...user, ...data }
      mockUserStore = mockUserStore.map((item) => item.id === id ? next : item)
      return mockDelay(next)
    }
    return api.put(`/admin/users/${encodeURIComponent(id)}`, data)
  },

  deleteUser: (id: string): Promise<void> => {
    if (!isMockMode) return api.delete(`/admin/users/${encodeURIComponent(id)}`)
    mockUserStore = mockUserStore.filter((user) => user.id !== id)
    mockApiKeyStore = mockApiKeyStore.filter((key) => key.userId !== id)
    return mockDelay(undefined)
  },

  getUserApiKeys: (userId: string): Promise<ApiKey[]> =>
    isMockMode
      ? mockDelay(mockApiKeyStore.filter((key) => key.userId === userId))
      : api.get(`/admin/users/${encodeURIComponent(userId)}/api-keys`),

  getApiKeys: (): Promise<ApiKey[]> =>
    isMockMode ? mockDelay(mockApiKeyStore) : api.get('/admin/api-keys'),

  createApiKey: (data: CreateApiKeyInput): Promise<ApiKey> => {
    if (!isMockMode) return api.post('/admin/api-keys', data)
    const key = `sk-mp-demo-${Math.random().toString(36).slice(2, 18)}`
    const row: ApiKey = {
      id: nextMockId('key'),
      userId: data.userId,
      username: data.username,
      name: data.name,
      keyPrefix: `${key.slice(0, 12)}...`,
      keyPreview: `${key.slice(0, 12)}...${key.slice(-4)}`,
      key,
      group: data.group || null,
      createdAt: new Date().toISOString(),
      lastUsedAt: null,
      expiresAt: null,
      status: 'active',
      requestsToday: 0,
      tokensToday: 0,
    }
    mockApiKeyStore = [row, ...mockApiKeyStore]
    return mockDelay(row)
  },

  revokeApiKey: (keyId: string): Promise<void> => {
    if (!isMockMode) return api.post(`/admin/api-keys/${encodeURIComponent(keyId)}/disable`)
    mockApiKeyStore = mockApiKeyStore.map((key) => key.id === keyId ? { ...key, status: 'revoked' } : key)
    return mockDelay(undefined)
  },

  updateApiKey: (keyId: string, data: UpdateApiKeyInput): Promise<ApiKey> => {
    if (!isMockMode) return api.put(`/admin/api-keys/${encodeURIComponent(keyId)}`, data)
    const key = mockApiKeyStore.find((item) => item.id === keyId)
    if (!key) throw new Error('API 密钥不存在')
    const next: ApiKey = {
      ...key,
      ...(data.name !== undefined ? { name: data.name } : {}),
      ...(data.group !== undefined ? { group: data.group.trim() || null } : {}),
      ...(data.expiresAt !== undefined ? { expiresAt: data.expiresAt || null } : {}),
      ...(data.status !== undefined ? { status: data.status } : {}),
      ...(data.ipRestricted !== undefined ? { ipRestricted: data.ipRestricted } : {}),
      ...(data.spendLimitUsd !== undefined ? { spendLimitUsd: data.spendLimitUsd } : {}),
      ...(data.rateLimited !== undefined ? { rateLimited: data.rateLimited } : {}),
      ...(data.fiveHourLimitUsd !== undefined ? { fiveHourLimitUsd: data.fiveHourLimitUsd } : {}),
      ...(data.dailyLimitUsd !== undefined ? { dailyLimitUsd: data.dailyLimitUsd } : {}),
      ...(data.weeklyLimitUsd !== undefined ? { weeklyLimitUsd: data.weeklyLimitUsd } : {}),
      ...(data.monthlyLimitUsd !== undefined ? { monthlyLimitUsd: data.monthlyLimitUsd } : {}),
    }
    mockApiKeyStore = mockApiKeyStore.map((item) => item.id === keyId ? next : item)
    return mockDelay(next)
  },

  deleteApiKey: (keyId: string): Promise<void> => {
    if (!isMockMode) return api.delete(`/admin/api-keys/${encodeURIComponent(keyId)}`)
    mockApiKeyStore = mockApiKeyStore.filter((key) => key.id !== keyId)
    return mockDelay(undefined)
  },
}
