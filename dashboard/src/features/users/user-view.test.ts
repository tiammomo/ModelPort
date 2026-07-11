import { describe, expect, it } from 'vitest'
import type { User } from '@/types'
import { filterUsers, isCreateUserFormValid, isUserEmailValid, isUserFilterActive } from './user-view'

const users: User[] = [
  {
    id: 'usr_admin', username: 'Alice', email: 'alice@example.com', role: 'admin', status: 'active',
    createdAt: '', lastLoginAt: null, apiKeyCount: 2, requestCount24h: 10,
  },
  {
    id: 'usr_viewer', username: 'Bob', email: 'bob@example.com', role: 'viewer', status: 'disabled',
    createdAt: '', lastLoginAt: null, apiKeyCount: 0, requestCount24h: 0,
  },
]

describe('filterUsers', () => {
  it('combines text, role and status filters', () => {
    expect(filterUsers(users, { search: 'EXAMPLE', role: 'admin', status: 'active' })).toEqual([users[0]])
    expect(filterUsers(users, { search: 'usr_viewer', role: 'all', status: 'disabled' })).toEqual([users[1]])
  })
})

it('only marks meaningful user filters as active', () => {
  expect(isUserFilterActive({ search: ' ', role: 'all', status: 'all' })).toBe(false)
  expect(isUserFilterActive({ search: '', role: 'viewer', status: 'all' })).toBe(true)
})

describe('isCreateUserFormValid', () => {
  it('requires a username, valid-looking email and 12-character password', () => {
    expect(isCreateUserFormValid({ username: 'alice', email: 'alice@example.com', password: 'long-password' })).toBe(true)
    expect(isCreateUserFormValid({ username: 'alice', email: 'alice@localhost', password: 'long-password' })).toBe(true)
    expect(isCreateUserFormValid({ username: '', email: 'alice@example.com', password: 'long-password' })).toBe(false)
    expect(isCreateUserFormValid({ username: 'alice', email: 'not-email', password: 'long-password' })).toBe(false)
    expect(isCreateUserFormValid({ username: 'alice', email: 'alice@example.com', password: 'short' })).toBe(false)
  })
})

it('accepts local-domain emails while rejecting missing address parts', () => {
  expect(isUserEmailValid('alice@localhost')).toBe(true)
  expect(isUserEmailValid('@localhost')).toBe(false)
  expect(isUserEmailValid('alice@')).toBe(false)
})
