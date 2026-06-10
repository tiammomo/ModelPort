import type { DashboardStats } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockDashboardStats } from '@/mock'

export const dashboardService = {
  getStats: (): Promise<DashboardStats> =>
    isMockMode ? mockDelay(mockDashboardStats) : api.get('/admin/dashboard'),
}
