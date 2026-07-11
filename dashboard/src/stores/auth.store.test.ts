import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { User } from '@/types'
import { queryClient } from '@/lib/query-client'

vi.mock('@/services/auth.service', () => ({
  authService: {
    login: vi.fn(),
    logout: vi.fn(),
    getCurrentUser: vi.fn(),
  },
}))

import { authService } from '@/services/auth.service'
import { useAuthStore } from './auth.store'

const user = {
  id: 'usr_next',
  username: 'next-user',
  email: 'next@example.test',
  role: 'user',
  status: 'active',
} as User

describe('auth query isolation', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    queryClient.clear()
    useAuthStore.setState({
      currentUser: null,
      isAuthenticated: false,
      isInitializing: false,
    })
  })

  it('clears cached private data before exposing a newly logged-in principal', async () => {
    vi.mocked(authService.login).mockResolvedValue(user)
    queryClient.setQueryData(['private', 'users'], [{ id: 'from-previous-account' }])

    await useAuthStore.getState().login('next-user', 'password')

    expect(queryClient.getQueryData(['private', 'users'])).toBeUndefined()
    expect(useAuthStore.getState().currentUser?.id).toBe(user.id)
  })

  it('clears cached private data on logout even if the server logout fails', async () => {
    vi.mocked(authService.logout).mockRejectedValue(new Error('network unavailable'))
    useAuthStore.setState({ currentUser: user, isAuthenticated: true })
    queryClient.setQueryData(['private', 'logs'], [{ id: 'sensitive-log' }])

    await useAuthStore.getState().logout()

    expect(queryClient.getQueryData(['private', 'logs'])).toBeUndefined()
    expect(useAuthStore.getState().isAuthenticated).toBe(false)
  })
})
