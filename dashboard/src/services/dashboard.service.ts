import type { DashboardStats } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockDashboardStats } from '@/mock'

export type DashboardRange = '1d' | '3d' | '7d' | 'custom'

export interface DashboardStatsParams {
  range?: DashboardRange
  from?: string
  to?: string
}

export const dashboardService = {
  getStats: (params: DashboardStatsParams = {}): Promise<DashboardStats> =>
    isMockMode ? mockDelay(mockDashboardStats) : api.get(`/admin/dashboard${dashboardQuery(params)}`),
}

function dashboardQuery(params: DashboardStatsParams): string {
  const query = new URLSearchParams()
  if (params.range) query.set('range', params.range)
  if (params.from) query.set('from', params.from)
  if (params.to) query.set('to', params.to)
  const value = query.toString()
  return value ? `?${value}` : ''
}
