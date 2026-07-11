import type { Quota, QuotaPeriod, QuotaType } from '@/types'

export type QuotaRisk = 'healthy' | 'warning' | 'exhausted'
export type QuotaRiskFilter = 'all' | QuotaRisk

export interface QuotaFilters {
  search: string
  quotaType: 'all' | QuotaType
  period: 'all' | QuotaPeriod
  risk: QuotaRiskFilter
}

export function quotaUsagePercent(used: number, limit: number): number {
  if (limit <= 0) return 0
  return Math.max(0, Math.round((used / limit) * 100))
}

export function quotaRisk(used: number, limit: number): QuotaRisk {
  if (limit <= 0) return 'exhausted'
  const ratio = Math.max(0, used / limit)
  if (ratio >= 1) return 'exhausted'
  if (ratio >= 0.8) return 'warning'
  return 'healthy'
}

export function filterQuotas(quotas: readonly Quota[], filters: QuotaFilters): Quota[] {
  const query = filters.search.trim().toLocaleLowerCase()

  return quotas.filter((quota) => {
    const matchesQuery = !query || [quota.username, quota.userId]
      .join(' ')
      .toLocaleLowerCase()
      .includes(query)
    return matchesQuery
      && (filters.quotaType === 'all' || quota.quotaType === filters.quotaType)
      && (filters.period === 'all' || quota.period === filters.period)
      && (filters.risk === 'all' || quotaRisk(quota.used, quota.limit) === filters.risk)
  })
}

export function isQuotaFilterActive(filters: QuotaFilters): boolean {
  return Boolean(filters.search.trim())
    || filters.quotaType !== 'all'
    || filters.period !== 'all'
    || filters.risk !== 'all'
}

export function isQuotaLimitValid(limit: number, quotaType: QuotaType): boolean {
  return Number.isFinite(limit) && limit >= 0 && (quotaType === 'cost' || Number.isInteger(limit))
}

export function quotaPeriodRange(period: QuotaPeriod, now = new Date()): {
  periodStart: string
  periodEnd: string
  resetAt: string
} {
  const year = now.getUTCFullYear()
  const month = now.getUTCMonth()
  const date = now.getUTCDate()
  let start: Date
  let end: Date

  if (period === 'daily') {
    start = new Date(Date.UTC(year, month, date))
    end = new Date(Date.UTC(year, month, date + 1))
  } else if (period === 'weekly') {
    const daysSinceMonday = (now.getUTCDay() + 6) % 7
    start = new Date(Date.UTC(year, month, date - daysSinceMonday))
    end = new Date(Date.UTC(year, month, date - daysSinceMonday + 7))
  } else {
    start = new Date(Date.UTC(year, month, 1))
    end = new Date(Date.UTC(year, month + 1, 1))
  }

  return {
    periodStart: start.toISOString(),
    periodEnd: end.toISOString(),
    resetAt: end.toISOString(),
  }
}
