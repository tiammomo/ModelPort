export type QuotaType = 'tokens' | 'requests' | 'cost'
export type QuotaPeriod = 'daily' | 'weekly' | 'monthly'

export interface Quota {
  id: string
  userId: string
  username: string
  quotaType: QuotaType
  limit: number
  used: number
  period: QuotaPeriod
  periodStart: string
  periodEnd: string
  resetAt: string
}

export interface UsageRecord {
  id: string
  userId: string
  model: string
  provider: string
  inputTokens: number
  outputTokens: number
  timestamp: string
  costEstimate: number | null
}

export interface TimeSeriesPoint {
  timestamp: string
  value: number
  label?: string
}

export interface UsageSummary {
  totalRequests: number
  totalInputTokens: number
  totalOutputTokens: number
  totalCostEstimate: number
  byModel: Record<string, { requests: number; tokens: number }>
  byProvider: Record<string, { requests: number; tokens: number }>
  byUser: Record<string, { requests: number; tokens: number }>
  timeSeries: TimeSeriesPoint[]
}
