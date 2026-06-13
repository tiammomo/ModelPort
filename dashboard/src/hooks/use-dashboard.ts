import { useQuery } from '@tanstack/react-query'
import { dashboardService, type DashboardStatsParams } from '@/services/dashboard.service'

export const queryKeys = {
  dashboard: ['dashboard'] as const,
  dashboardStats: (params: DashboardStatsParams) => ['dashboard', params] as const,
  users: ['users'] as const,
  user: (id: string) => ['users', id] as const,
  apiKeys: ['api-keys'] as const,
  teams: ['teams'] as const,
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

export function useDashboard(params: DashboardStatsParams = {}) {
  return useQuery({
    queryKey: queryKeys.dashboardStats(params),
    queryFn: () => dashboardService.getStats(params),
    refetchInterval: 30000,
  })
}
