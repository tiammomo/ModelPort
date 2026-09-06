import type { User, CreateUserInput, UpdateUserInput, ApiKey, Team, UpsertTeamInput } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay, nextMockId } from '@/lib/mock-mode'
import { mockApiKeys, mockUsers } from '@/mock'

export interface CreateApiKeyInput {
  userId: string
  username?: string
  name: string
  principalType?: 'user' | 'service_account'
  purpose?: string
  group?: string
  teamId?: string
  allowedModels?: string[]
  allowedProviders?: string[]
  expiresAt?: string
}

export interface UpdateApiKeyInput {
  name?: string
  group?: string
  teamId?: string
  allowedModels?: string[]
  allowedProviders?: string[]
  expiresAt?: string
  status?: ApiKey['status']
  ipRestricted?: boolean
  allowedIps?: string[]
  spendLimitUsd?: number
  rateLimited?: boolean
  fiveHourLimitUsd?: number
  dailyLimitUsd?: number
  weeklyLimitUsd?: number
  monthlyLimitUsd?: number
}

export type RevealedApiKey = ApiKey & { key: string }
export type PreparedApiKeyRotation = Omit<ApiKey, 'status'> & {
  status: 'pending_rotation'
  key: string
}

let mockUserStore = isMockMode ? [...mockUsers] : []
let mockApiKeyStore = isMockMode ? [...mockApiKeys] : []
const mockPendingApiKeyRotations = new Map<string, { sourceKeyId: string; replacement: PreparedApiKeyRotation }>()
let mockTeamStore: Team[] = [
  {
    id: 'team_default',
    name: '默认项目',
    slug: 'default',
    description: '小团队默认项目',
    status: 'active',
    dailyLimitUsd: 20,
    monthlyLimitUsd: 300,
    dailySpendUsd: 0.34,
    monthlySpendUsd: 8.2,
    allowedModels: [],
    allowedProviders: [],
    activeApiKeys: 0,
    requestsToday: 0,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  },
]

function withApiKeyCounts(users: User[]) {
  return users.map((user) => ({
    ...user,
    apiKeyCount: mockApiKeyStore.filter((key) => key.userId === user.id && key.status === 'active').length,
  }))
}

function withTeamCounts(teams: Team[]) {
  return teams.map((team) => ({
    ...team,
    activeApiKeys: mockApiKeyStore.filter((key) => key.teamId === team.id && key.status === 'active').length,
    requestsToday: mockApiKeyStore
      .filter((key) => key.teamId === team.id)
      .reduce((sum, key) => sum + (key.requestsToday || 0), 0),
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

  getTeams: (): Promise<Team[]> =>
    isMockMode ? mockDelay(withTeamCounts(mockTeamStore)) : api.get('/admin/teams'),

  upsertTeam: (data: UpsertTeamInput): Promise<Team> => {
    if (!isMockMode) {
      return data.id
        ? api.put(`/admin/teams/${encodeURIComponent(data.id)}`, data)
        : api.post('/admin/teams', data)
    }
    const now = new Date().toISOString()
    const existing = data.id ? mockTeamStore.find((team) => team.id === data.id) : undefined
    const row: Team = {
      id: existing?.id || nextMockId('team'),
      name: data.name,
      slug: data.slug || data.name.toLowerCase().replace(/\s+/g, '-'),
      description: data.description || null,
      status: data.status || existing?.status || 'active',
      dailyLimitUsd: data.dailyLimitUsd ?? existing?.dailyLimitUsd ?? 0,
      monthlyLimitUsd: data.monthlyLimitUsd ?? existing?.monthlyLimitUsd ?? 0,
      dailySpendUsd: existing?.dailySpendUsd ?? 0,
      monthlySpendUsd: existing?.monthlySpendUsd ?? 0,
      allowedModels: data.allowedModels ?? existing?.allowedModels ?? [],
      allowedProviders: data.allowedProviders ?? existing?.allowedProviders ?? [],
      activeApiKeys: existing?.activeApiKeys ?? 0,
      requestsToday: existing?.requestsToday ?? 0,
      createdAt: existing?.createdAt || now,
      updatedAt: now,
    }
    mockTeamStore = existing
      ? mockTeamStore.map((team) => team.id === row.id ? row : team)
      : [row, ...mockTeamStore]
    return mockDelay(row)
  },

  deleteTeam: (teamId: string): Promise<void> => {
    if (!isMockMode) return api.delete(`/admin/teams/${encodeURIComponent(teamId)}`)
    mockTeamStore = mockTeamStore.filter((team) => team.id !== teamId)
    mockApiKeyStore = mockApiKeyStore.map((key) => key.teamId === teamId ? { ...key, teamId: null, teamName: null } : key)
    return mockDelay(undefined)
  },

  createApiKey: (data: CreateApiKeyInput): Promise<RevealedApiKey> => {
    if (!isMockMode) return api.post('/admin/api-keys', data)
    const key = `sk-mp-demo-${Math.random().toString(36).slice(2, 18)}`
    const row: RevealedApiKey = {
      id: nextMockId('key'),
      userId: data.userId,
      username: data.username,
      name: data.name,
      principalType: data.principalType || 'user',
      purpose: data.purpose || null,
      keyPrefix: `${key.slice(0, 12)}...`,
      keyPreview: `${key.slice(0, 12)}...${key.slice(-4)}`,
      key,
      group: data.group || null,
      teamId: data.teamId || null,
      teamName: mockTeamStore.find((team) => team.id === data.teamId)?.name || null,
      allowedModels: data.allowedModels || [],
      allowedProviders: data.allowedProviders || [],
      createdAt: new Date().toISOString(),
      lastUsedAt: null,
      expiresAt: data.expiresAt || null,
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

  rotateApiKey: (keyId: string): Promise<PreparedApiKeyRotation> => {
    if (!isMockMode) return api.post(`/admin/api-keys/${encodeURIComponent(keyId)}/rotate`)
    const current = mockApiKeyStore.find((item) => item.id === keyId)
    if (!current || current.status !== 'active') {
      return Promise.reject(new Error('只有启用的 API 密钥可以轮换'))
    }
    const key = `sk-mp-demo-${Math.random().toString(36).slice(2, 18)}`
    const replacement: PreparedApiKeyRotation = {
      ...current,
      id: nextMockId('key'),
      key,
      keyPrefix: `${key.slice(0, 12)}...`,
      keyPreview: `${key.slice(0, 12)}...${key.slice(-4)}`,
      createdAt: new Date().toISOString(),
      lastUsedAt: null,
      status: 'pending_rotation',
      requestsToday: 0,
      tokensToday: 0,
    }
    mockPendingApiKeyRotations.set(replacement.id, { sourceKeyId: keyId, replacement })
    return mockDelay(replacement)
  },

  confirmApiKeyRotation: (keyId: string, replacementId: string): Promise<ApiKey> => {
    if (!isMockMode) {
      return api.post(`/admin/api-keys/${encodeURIComponent(keyId)}/rotate/${encodeURIComponent(replacementId)}`)
    }
    const pending = mockPendingApiKeyRotations.get(replacementId)
    if (!pending || pending.sourceKeyId !== keyId) {
      return Promise.reject(new Error('待确认的密钥轮换不存在'))
    }
    const replacement: ApiKey = { ...pending.replacement, key: undefined, status: 'active' }
    mockApiKeyStore = [
      replacement,
      ...mockApiKeyStore.map((item) => item.id === keyId
        ? { ...item, status: 'revoked' as const, supersededByKeyId: replacement.id }
        : item),
    ]
    mockPendingApiKeyRotations.delete(replacementId)
    return mockDelay(replacement)
  },

  cancelApiKeyRotation: (keyId: string, replacementId: string): Promise<void> => {
    if (!isMockMode) {
      return api.delete(`/admin/api-keys/${encodeURIComponent(keyId)}/rotate/${encodeURIComponent(replacementId)}`)
    }
    const pending = mockPendingApiKeyRotations.get(replacementId)
    if (!pending || pending.sourceKeyId !== keyId) {
      return Promise.reject(new Error('待确认的密钥轮换不存在'))
    }
    mockPendingApiKeyRotations.delete(replacementId)
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
      ...(data.teamId !== undefined ? {
        teamId: data.teamId || null,
        teamName: mockTeamStore.find((team) => team.id === data.teamId)?.name || null,
      } : {}),
      ...(data.allowedModels !== undefined ? { allowedModels: data.allowedModels } : {}),
      ...(data.allowedProviders !== undefined ? { allowedProviders: data.allowedProviders } : {}),
      ...(data.expiresAt !== undefined ? { expiresAt: data.expiresAt || null } : {}),
      ...(data.status !== undefined ? { status: data.status } : {}),
      ...(data.ipRestricted !== undefined ? { ipRestricted: data.ipRestricted } : {}),
      ...(data.allowedIps !== undefined ? { allowedIps: data.allowedIps } : {}),
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
