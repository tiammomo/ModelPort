import type { User, CreateUserInput, ApiKey } from '@/types'
import { api } from '@/lib/api-client'

export interface CreateApiKeyInput {
  userId: string
  username?: string
  name: string
  group?: string
}

export const usersService = {
  getUsers: (): Promise<User[]> => api.get('/admin/users'),

  getUser: async (id: string): Promise<User> => {
    const users = await api.get<User[]>('/admin/users')
    const user = users.find((item) => item.id === id)
    if (!user) throw new Error('用户不存在')
    return user
  },

  createUser: (data: CreateUserInput): Promise<User> => api.post('/admin/users', data),

  updateUser: async (id: string, data: Partial<User>): Promise<User> => {
    void id
    const users = await api.get<User[]>('/admin/users')
    return { ...users[0], ...data }
  },

  deleteUser: (id: string): Promise<void> => api.delete(`/admin/users/${encodeURIComponent(id)}`),

  getUserApiKeys: (userId: string): Promise<ApiKey[]> =>
    api.get(`/admin/users/${encodeURIComponent(userId)}/api-keys`),

  getApiKeys: (): Promise<ApiKey[]> => api.get('/admin/api-keys'),

  createApiKey: (data: CreateApiKeyInput): Promise<ApiKey> =>
    api.post('/admin/api-keys', data),

  revokeApiKey: (keyId: string): Promise<void> =>
    api.post(`/admin/api-keys/${encodeURIComponent(keyId)}/disable`),

  deleteApiKey: (keyId: string): Promise<void> =>
    api.delete(`/admin/api-keys/${encodeURIComponent(keyId)}`),
}
