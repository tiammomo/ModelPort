import type { User, ApiKey } from '@/types'
import { api } from '@/lib/api-client'

export const usersService = {
  getUsers: (): Promise<User[]> => api.get('/admin/users'),

  getUser: async (id: string): Promise<User> => {
    const users = await api.get<User[]>('/admin/users')
    const user = users.find((item) => item.id === id)
    if (!user) throw new Error('用户不存在')
    return user
  },

  createUser: (
    data: Omit<User, 'id' | 'createdAt' | 'lastLoginAt' | 'apiKeyCount' | 'requestCount24h'>
  ): Promise<User> => api.post('/admin/users', data),

  updateUser: async (id: string, data: Partial<User>): Promise<User> => {
    void id
    const users = await api.get<User[]>('/admin/users')
    return { ...users[0], ...data }
  },

  deleteUser: (id: string): Promise<void> => api.delete(`/admin/users/${encodeURIComponent(id)}`),

  getUserApiKeys: (userId: string): Promise<ApiKey[]> =>
    api.get(`/admin/users/${encodeURIComponent(userId)}/api-keys`),

  createApiKey: (userId: string, name: string): Promise<ApiKey> =>
    api.post(`/admin/users/${encodeURIComponent(userId)}/api-keys`, { userId, name }),

  revokeApiKey: (keyId: string): Promise<void> =>
    api.delete(`/admin/api-keys/${encodeURIComponent(keyId)}`),
}
