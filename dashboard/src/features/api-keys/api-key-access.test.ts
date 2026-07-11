import { describe, expect, it } from 'vitest'

import { apiKeyAccessForRole, apiKeySelfServiceUpdate } from './api-key-access'

describe('API key role access', () => {
  it('grants administrators the complete management surface', () => {
    expect(apiKeyAccessForRole('admin')).toMatchObject({
      isAdmin: true,
      canCreate: true,
      canManageTeams: true,
      canEdit: true,
      canManagePolicy: true,
      canRevoke: true,
      canRestore: true,
      canDelete: true,
    })
  })

  it('limits normal users to self-service key operations', () => {
    expect(apiKeyAccessForRole('user')).toEqual({
      isAdmin: false,
      canCreate: false,
      canManageTeams: false,
      canEdit: true,
      canManagePolicy: false,
      canRevoke: true,
      canRestore: false,
      canDelete: true,
    })
  })

  it('keeps viewers and missing sessions read-only', () => {
    expect(apiKeyAccessForRole('viewer')).toEqual(apiKeyAccessForRole(undefined))
    expect(Object.values(apiKeyAccessForRole('viewer')).some(Boolean)).toBe(false)
  })

  it('builds a self-service update without administrator policy fields', () => {
    expect(apiKeySelfServiceUpdate('  renamed  ', '  dev  ')).toEqual({
      name: 'renamed',
      group: 'dev',
    })
  })
})
