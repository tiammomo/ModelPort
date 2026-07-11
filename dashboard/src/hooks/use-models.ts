import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { modelsService } from '@/services/models.service'
import { withDefaultProvider } from '@/features/models/default-provider'
import { queryKeys } from './use-dashboard'
import type { Provider, ProviderCredentialPoolMode, ProviderCredentialWritePayload, ProviderWritePayload, SystemSettings } from '@/types'

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
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useBulkToggleModels() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: async ({ providerId, models, enabled }: { providerId: string; models: string[]; enabled: boolean }) => {
      const results = await Promise.allSettled(
        models.map((model) => modelsService.toggleModel(providerId, model, enabled)),
      )
      const failures = results.filter((result) => result.status === 'rejected')
      if (failures.length > 0) {
        throw new Error(`${failures.length} 个模型更新失败`)
      }
      return { updated: results.length }
    },
    onSettled: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useUpdateDefaultModel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, model }: { providerId: string; model: string }) =>
      modelsService.updateDefaultModel(providerId, model),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useCreateProvider() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: ProviderWritePayload) => modelsService.createProvider(data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useUpdateProvider() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, data }: { providerId: string; data: ProviderWritePayload }) =>
      modelsService.updateProvider(providerId, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useSetProviderDisabled() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, disabled }: { providerId: string; disabled: boolean }) =>
      modelsService.setProviderDisabled(providerId, disabled),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useCreateProviderCredential() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, data }: { providerId: string; data: ProviderCredentialWritePayload }) =>
      modelsService.createProviderCredential(providerId, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useUpdateProviderCredential() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, credentialId, data }: { providerId: string; credentialId: string; data: ProviderCredentialWritePayload }) =>
      modelsService.updateProviderCredential(providerId, credentialId, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useSelectProviderCredential() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, credentialId }: { providerId: string; credentialId: string }) =>
      modelsService.selectProviderCredential(providerId, credentialId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useUpdateProviderCredentialPoolMode() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, mode }: { providerId: string; mode: ProviderCredentialPoolMode }) =>
      modelsService.updateProviderCredentialPoolMode(providerId, mode),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useDeleteProviderCredential() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, credentialId }: { providerId: string; credentialId: string }) =>
      modelsService.deleteProviderCredential(providerId, credentialId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
  })
}

export function useDeleteProvider() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ providerId, force = false }: { providerId: string; force?: boolean }) =>
      modelsService.deleteProvider(providerId, force),
    onSuccess: (_result, variables) => {
      qc.setQueryData<Provider[]>(queryKeys.providers, (current) =>
        current?.filter((provider) => provider.id !== variables.providerId),
      )
      qc.invalidateQueries({ queryKey: queryKeys.providers })
      qc.invalidateQueries({ queryKey: queryKeys.settings })
      qc.invalidateQueries({ queryKey: queryKeys.aliases })
      qc.invalidateQueries({ queryKey: queryKeys.dashboard })
    },
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
    onMutate: async (providerId) => {
      await qc.cancelQueries({ queryKey: queryKeys.settings })
      const previousSettings = qc.getQueryData<SystemSettings>(queryKeys.settings)
      qc.setQueryData<SystemSettings>(queryKeys.settings, (current) =>
        current ? withDefaultProvider(current, providerId) : current,
      )
      return { previousSettings }
    },
    onError: (_error, _providerId, context) => {
      if (context?.previousSettings) {
        qc.setQueryData(queryKeys.settings, context.previousSettings)
      }
    },
    onSettled: async () => {
      await Promise.all([
        qc.invalidateQueries({ queryKey: queryKeys.providers }),
        qc.invalidateQueries({ queryKey: queryKeys.settings }),
      ])
    },
  })
}
