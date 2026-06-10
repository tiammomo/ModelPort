import { useQuery } from '@tanstack/react-query'
import { dashboardService } from '@/services/dashboard.service'

export const queryKeys = {
  dashboard: ['dashboard'] as const,
  users: ['users'] as const,
  user: (id: string) => ['users', id] as const,
  userApiKeys: (userId: string) => ['users', userId, 'api-keys'] as const,
  quotas: ['quotas'] as const,
  providers: ['providers'] as const,
  provider: (id: string) => ['providers', id] as const,
  aliases: ['aliases'] as const,
  logs: (filters: unknown) => ['logs', filters] as const,
  logById: (id: string) => ['logs', id] as const,
  latencyStats: ['latency-stats'] as const,
  settings: ['settings'] as const,
} as const

export function useDashboard() {
  return useQuery({
    queryKey: queryKeys.dashboard,
    queryFn: () => dashboardService.getStats(),
    refetchInterval: 30000,
  })
}
