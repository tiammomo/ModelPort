import type { UserRole } from '@/types'

export interface ApiKeyAccess {
  isAdmin: boolean
  canCreate: boolean
  canManageTeams: boolean
  canEdit: boolean
  canManagePolicy: boolean
  canRevoke: boolean
  canRestore: boolean
  canDelete: boolean
}

const READ_ONLY_ACCESS: ApiKeyAccess = {
  isAdmin: false,
  canCreate: false,
  canManageTeams: false,
  canEdit: false,
  canManagePolicy: false,
  canRevoke: false,
  canRestore: false,
  canDelete: false,
}

export function apiKeyAccessForRole(role?: UserRole): ApiKeyAccess {
  if (role === 'admin') {
    return {
      isAdmin: true,
      canCreate: true,
      canManageTeams: true,
      canEdit: true,
      canManagePolicy: true,
      canRevoke: true,
      canRestore: true,
      canDelete: true,
    }
  }
  if (role === 'user') {
    return {
      ...READ_ONLY_ACCESS,
      canEdit: true,
      canRevoke: true,
      canDelete: true,
    }
  }
  return READ_ONLY_ACCESS
}

export function apiKeySelfServiceUpdate(name: string, group: string) {
  return {
    name: name.trim(),
    group: group.trim(),
  }
}
