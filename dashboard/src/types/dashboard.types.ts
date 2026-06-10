import type { TimeSeriesPoint } from './quota.types'

export interface DashboardStats {
  uptimeSeconds: number
  totalRequests: number
  successRate: number
  activeProviders: number
  totalProviders: number
  activeUsers: number
  totalModels: number
  avgLatencyMs: number
  requestTimeSeries: TimeSeriesPoint[]
  errorTimeSeries: TimeSeriesPoint[]
  topModels: Array<{ model: string; provider: string; requests: number }>
  providerHealth: Array<{
    providerId: string
    displayName: string
    status: 'healthy' | 'degraded' | 'down'
    requestsTotal: number
    successRate: number
    avgLatencyMs: number
  }>
  recentActivity: Array<{
    id: string
    timestamp: string
    type: 'request' | 'error' | 'config_change'
    message: string
    severity: 'info' | 'warning' | 'error'
  }>
}
