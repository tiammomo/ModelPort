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

export interface CreateUserInput {
  username: string
  email: string
  password: string
  role: UserRole
  status: 'active' | 'disabled' | 'suspended'
}

export interface ApiKey {
  id: string
  userId: string
  username?: string
  name: string
  keyPrefix: string
  keyPreview?: string
  key?: string
  group?: string | null
  createdAt: string
  lastUsedAt: string | null
  expiresAt: string | null
  status: 'active' | 'revoked'
  requestsToday?: number
  tokensToday?: number
}
