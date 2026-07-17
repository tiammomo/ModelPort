import type { User } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockUsers } from '@/mock'

interface LoginResponse {
  user: User
  expiresAt: string
}

export interface AuthMethods {
  passwordEnabled: boolean
  oidc: {
    enabled: boolean
    label: string
    startUrl: string
  }
}

const MOCK_AUTH_METHODS: AuthMethods = {
  passwordEnabled: true,
  oidc: {
    enabled: false,
    label: '企业单点登录',
    startUrl: '/admin/auth/oidc/start',
  },
}

const MOCK_SESSION_KEY = 'modelport_mock_session'

export const authService = {
  getMethods: (): Promise<AuthMethods> => {
    if (!isMockMode) return api.get('/admin/auth/methods')
    return mockDelay(MOCK_AUTH_METHODS)
  },

  login: async (username: string, password: string): Promise<User> => {
    if (!username.trim() || !password) {
      throw new Error('无效的账号或密码')
    }

    if (isMockMode) {
      if (username.trim() !== 'admin' || password !== 'admin') {
        throw new Error('mock 模式账号密码为 admin / admin')
      }
      const user = mockUsers.find((item) => item.username === 'admin') || mockUsers[0]
      window.localStorage.setItem(MOCK_SESSION_KEY, user.id)
      return mockDelay(user)
    }

    const response = await api.post<LoginResponse>('/admin/auth/login', {
      username: username.trim(),
      password,
    })
    return response.user
  },

  logout: (): Promise<{ ok: boolean }> => {
    if (!isMockMode) return api.post('/admin/auth/logout')
    window.localStorage.removeItem(MOCK_SESSION_KEY)
    return mockDelay({ ok: true })
  },

  getCurrentUser: (): Promise<User> => {
    if (!isMockMode) return api.get('/admin/auth/me')
    const userId = window.localStorage.getItem(MOCK_SESSION_KEY)
    const user = mockUsers.find((item) => item.id === userId)
    if (!user) return Promise.reject(new Error('Unauthorized'))
    return mockDelay(user)
  },
}
