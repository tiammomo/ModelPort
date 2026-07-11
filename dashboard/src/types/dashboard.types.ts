import type { TimeSeriesPoint } from './quota.types'
import type { LogSummary } from './log.types'

export interface DashboardModelUsage {
  model: string
  provider: string
  requests: number
  tokens: number
  cost: number
}

export interface DashboardTokenTimePoint {
  timestamp: string
  inputTokens: number
  outputTokens: number
  cacheWriteTokens: number
  cacheReadTokens: number
  cacheHitRate: number
}

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
  modelUsage: DashboardModelUsage[]
  tokenTimeSeries: DashboardTokenTimePoint[]
  rangeSummary: LogSummary
  rangeDataSource: 'persisted-usage' | 'process-metrics-estimate' | 'empty'
  rangeDataEstimated: boolean
  rangeDataAtRetentionLimit: boolean
  providerHealth: Array<{
    providerId: string
    displayName: string
    status: 'healthy' | 'degraded' | 'down' | 'cooldown'
    requestsTotal: number
    successRate: number
    avgLatencyMs: number
    inputTokensTotal?: number
    outputTokensTotal?: number
    cacheWriteTokensTotal?: number
    cacheReadTokensTotal?: number
    costEstimateUsdTotal?: number
    accountIssue?: 'none' | 'insufficient_balance' | 'auth'
    rechargeRequired?: boolean
    rechargeBadge?: string | null
  }>
  recentActivity: Array<{
    id: string
    timestamp: string
    type: 'request' | 'error' | 'config_change' | 'auto_governance' | 'account_issue' | string
    message: string
    severity: 'info' | 'warning' | 'error'
  }>
}
