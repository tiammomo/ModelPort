import type { User } from '@/types'

export interface QuotaUserSelection {
  userId: string
  username: string
}

export function resolveQuotaUser(
  users: readonly User[],
  userId: string,
): QuotaUserSelection | null {
  const user = users.find((candidate) => candidate.id === userId)
  if (!user) return null
  return {
    userId: user.id,
    username: user.username,
  }
}
