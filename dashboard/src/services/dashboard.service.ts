import type { DashboardStats } from '@/types'
import { api } from '@/lib/api-client'

export const dashboardService = {
  getStats: (): Promise<DashboardStats> => api.get('/admin/dashboard'),
}
