import type { User } from '@/types'
import { api } from '@/lib/api-client'

export const authService = {
  login: async (token: string): Promise<User> => {
    if (!token.trim()) {
      throw new Error('无效的认证令牌')
    }

    localStorage.setItem('modelport_token', token)
    try {
      const users = await api.get<User[]>('/admin/users')
      return users[0]
    } catch (error) {
      localStorage.removeItem('modelport_token')
      throw error
    }
  },

  getCurrentUser: async (): Promise<User> => {
    const users = await api.get<User[]>('/admin/users')
    return users[0]
  },
}
