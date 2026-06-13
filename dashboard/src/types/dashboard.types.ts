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
  apiKeysTotal?: number
  apiKeysActive?: number
  todayRequests?: number
  todayInputTokens?: number
  todayOutputTokens?: number
  todayCacheWriteTokens?: number
  todayCacheReadTokens?: number
  todayCostEstimate?: number
  trendRange?: {
    range: '1d' | '3d' | '7d' | 'custom'
    from: string
    to: string
    bucketMs: number
  }
  requestTimeSeries: TimeSeriesPoint[]
  errorTimeSeries: TimeSeriesPoint[]
  topModels: Array<{ model: string; provider: string; requests: number }>
  providerHealth: Array<{
    providerId: string
    displayName: string
    status: 'healthy' | 'degraded' | 'down' | 'cooldown'
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
