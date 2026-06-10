import { create } from 'zustand'
import type { User } from '@/types'
import { authService } from '@/services/auth.service'

interface AuthState {
  token: string | null
  currentUser: User | null
  isAuthenticated: boolean
  login: (token: string) => Promise<void>
  logout: () => void
  initialize: () => void
}

export const useAuthStore = create<AuthState>((set) => ({
  token: null,
  currentUser: null,
  isAuthenticated: false,

  login: async (token: string) => {
    const adminUser = await authService.login(token)
    set({ token, currentUser: adminUser, isAuthenticated: true })
  },

  logout: () => {
    localStorage.removeItem('modelport_token')
    set({ token: null, currentUser: null, isAuthenticated: false })
  },

  initialize: () => {
    const token = localStorage.getItem('modelport_token')
    if (token) {
      set({ token, isAuthenticated: true })
      authService
        .getCurrentUser()
        .then((currentUser) => set({ currentUser }))
        .catch(() => {
          localStorage.removeItem('modelport_token')
          set({ token: null, currentUser: null, isAuthenticated: false })
        })
    }
  },
}))
