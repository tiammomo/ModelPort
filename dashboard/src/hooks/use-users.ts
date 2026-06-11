import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { usersService, type CreateApiKeyInput, type UpdateApiKeyInput } from '@/services/users.service'
import { queryKeys } from './use-dashboard'
import type { CreateUserInput, UpdateUserInput } from '@/types'

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

export function useApiKeys() {
  return useQuery({
    queryKey: queryKeys.apiKeys,
    queryFn: () => usersService.getApiKeys(),
  })
}

export function useCreateUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: CreateUserInput) => usersService.createUser(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.users }),
  })
}

export function useUpdateUser() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateUserInput }) => usersService.updateUser(id, data),
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
    mutationFn: (data: CreateApiKeyInput) => usersService.createApiKey(data),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: queryKeys.apiKeys })
      qc.invalidateQueries({ queryKey: queryKeys.userApiKeys(vars.userId) })
      qc.invalidateQueries({ queryKey: queryKeys.users })
    },
  })
}

export function useRevokeApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (keyId: string) => usersService.revokeApiKey(keyId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.apiKeys })
      qc.invalidateQueries({ queryKey: queryKeys.users })
    },
  })
}

export function useUpdateApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ keyId, data }: { keyId: string; data: UpdateApiKeyInput }) => usersService.updateApiKey(keyId, data),
    onSuccess: (apiKey) => {
      qc.invalidateQueries({ queryKey: queryKeys.apiKeys })
      qc.invalidateQueries({ queryKey: queryKeys.userApiKeys(apiKey.userId) })
      qc.invalidateQueries({ queryKey: queryKeys.users })
    },
  })
}

export function useDeleteApiKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (keyId: string) => usersService.deleteApiKey(keyId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.apiKeys })
      qc.invalidateQueries({ queryKey: queryKeys.users })
    },
  })
}
