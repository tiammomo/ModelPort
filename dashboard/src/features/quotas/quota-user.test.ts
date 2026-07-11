import { describe, expect, it } from 'vitest'

import type { User } from '@/types'
import { resolveQuotaUser } from './quota-user'

const users = [
  {
    id: 'usr_real',
    username: 'real-user',
    email: 'real@example.com',
    role: 'user',
    status: 'active',
  },
] as User[]

describe('quota user selection', () => {
  it('derives the submitted id and username from a real user record', () => {
    expect(resolveQuotaUser(users, 'usr_real')).toEqual({
      userId: 'usr_real',
      username: 'real-user',
    })
  })

  it('does not manufacture a quota owner for an unknown id', () => {
    expect(resolveQuotaUser(users, 'usr_forged')).toBeNull()
  })
})
