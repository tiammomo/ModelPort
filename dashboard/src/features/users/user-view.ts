import type { CreateUserInput, User } from '@/types'

export type UserRoleFilter = 'all' | User['role']
export type UserStatusFilter = 'all' | User['status']

export interface UserFilters {
  search: string
  role: UserRoleFilter
  status: UserStatusFilter
}

export function filterUsers(users: readonly User[], filters: UserFilters): User[] {
  const query = filters.search.trim().toLocaleLowerCase()

  return users.filter((user) => {
    const matchesQuery = !query || [user.username, user.email, user.id]
      .join(' ')
      .toLocaleLowerCase()
      .includes(query)
    return matchesQuery
      && (filters.role === 'all' || user.role === filters.role)
      && (filters.status === 'all' || user.status === filters.status)
  })
}

export function isUserFilterActive(filters: UserFilters): boolean {
  return Boolean(filters.search.trim()) || filters.role !== 'all' || filters.status !== 'all'
}

export function isCreateUserFormValid(form: Pick<CreateUserInput, 'username' | 'email' | 'password'>): boolean {
  return form.username.trim().length > 0
    && isUserEmailValid(form.email)
    && form.password.length >= 12
}

export function isUserEmailValid(email: string): boolean {
  return /^[^\s@]+@[^\s@]+$/.test(email.trim())
}
