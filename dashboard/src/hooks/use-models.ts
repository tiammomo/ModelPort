import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { modelsService } from '@/services/models.service'
import { queryKeys } from './use-dashboard'

export function useProviders() {
  return useQuery({
    queryKey: queryKeys.providers,
    queryFn: () => modelsService.getProviders(),
  })
}

export function useProvider(id: string) {
  return useQuery({
    queryKey: queryKeys.provider(id),
    queryFn: () => modelsService.getProvider(id),
    enabled: !!id,
  })
}

export function useAliases() {
  return useQuery({
    queryKey: queryKeys.aliases,
    queryFn: () => modelsService.getAliases(),
  })
}

export function useToggleModel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, model, enabled }: { providerId: string; model: string; enabled: boolean }) =>
      modelsService.toggleModel(providerId, model, enabled),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.providers }),
  })
}

export function useDiscoverProviderModels() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (providerId: string) => modelsService.discoverProviderModels(providerId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useCreateAlias() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: { alias: string; target: string }) => modelsService.createAlias(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.aliases }),
  })
}

export function useDeleteAlias() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (alias: string) => modelsService.deleteAlias(alias),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.aliases }),
  })
}

export function useUpdateDefaultProvider() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (providerId: string) => modelsService.updateDefaultProvider(providerId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
    },
  })
}
