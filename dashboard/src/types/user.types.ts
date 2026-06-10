export type UserRole = 'admin' | 'user' | 'viewer'

export interface User {
  id: string
  username: string
  email: string
  role: UserRole
  status: 'active' | 'disabled' | 'suspended'
  createdAt: string
  lastLoginAt: string | null
  apiKeyCount: number
  requestCount24h: number
}

export interface ApiKey {
  id: string
  userId: string
  name: string
  keyPrefix: string
  createdAt: string
  lastUsedAt: string | null
  expiresAt: string | null
  status: 'active' | 'revoked'
}
