import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { settingsService } from '@/services/settings.service'
import { queryKeys } from './use-dashboard'
import type { SystemSettings } from '@/types'

export function useSettings() {
  return useQuery({
    queryKey: queryKeys.settings,
    queryFn: () => settingsService.getSettings(),
  })
}

export function useUpdateSettings() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (settings: Partial<SystemSettings>) => settingsService.updateSettings(settings),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.settings }),
  })
}

export function useTestProviderConnection() {
  return useMutation({
    mutationFn: (providerId: string) => settingsService.testProviderConnection(providerId),
  })
}
