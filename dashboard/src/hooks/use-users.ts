import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { usersService } from '@/services/users.service'
import { queryKeys } from './use-dashboard'
import type { User } from '@/types'

export function useUsers() {
  return useQuery({
    queryKey: queryKeys.users,
    queryFn: () => usersService.getUsers(),
  })
}

export function useUser(id: string) {
  return useQuery({
    queryKey: queryKeys.user(id),
    queryFn: () => usersService.getUser(id),
    enabled: !!id,
  })
}

export function useUserApiKeys(userId: string) {
  return useQuery({
    queryKey: queryKeys.userApiKeys(userId),
    queryFn: () => usersService.getUserApiKeys(userId),
    enabled: !!userId,
  })
}

export function useCreateUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: Omit<User, 'id' | 'createdAt' | 'lastLoginAt' | 'apiKeyCount' | 'requestCount24h'>) =>
      usersService.createUser(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.users }),
  })
}

export function useUpdateUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<User> }) => usersService.updateUser(id, data),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.users }),
  })
}

export function useDeleteUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => usersService.deleteUser(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.users }),
  })
}

export function useCreateApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ userId, name }: { userId: string; name: string }) => usersService.createApiKey(userId, name),
    onSuccess: (_data, vars) => qc.invalidateQueries({ queryKey: queryKeys.userApiKeys(vars.userId) }),
  })
}

export function useRevokeApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (keyId: string) => usersService.revokeApiKey(keyId),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['users'] }),
  })
}
