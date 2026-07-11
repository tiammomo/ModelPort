import { formatNumber, parseDate } from '@/lib/utils'
import type { DashboardRange, DashboardStatsParams } from '@/services/dashboard.service'
import type { DashboardStats, RequestLog } from '@/types'

export const DAY_MS = 24 * 60 * 60 * 1000

export const TREND_RANGES: Array<{ value: DashboardRange; label: string }> = [
  { value: '1d', label: '近1天' },
  { value: '3d', label: '近3天' },
  { value: '7d', label: '近7天' },
  { value: 'custom', label: '自定义' },
]

const RANGE_LABELS: Record<DashboardRange, string> = {
  '1d': '24小时',
  '3d': '3天',
  '7d': '7天',
  custom: '自定义',
}

const RANGE_MS: Record<Exclude<DashboardRange, 'custom'>, number> = {
  '1d': DAY_MS,
  '3d': 3 * DAY_MS,
  '7d': 7 * DAY_MS,
}

export interface ModelUsageRow {
  model: string
  provider: string
  requests: number
  tokens: number
  cost: number
}

export interface TokenTrendPoint {
  time: string
  input: number
  output: number
  cacheWrite: number
  cacheRead: number
  cacheHitRate: number
}

export function computeTrend(series: { value: number }[]): number {
  if (series.length < 2) return 0
  const mid = Math.floor(series.length / 2)
  const firstHalf = series.slice(0, mid).reduce((sum, point) => sum + point.value, 0)
  const secondHalf = series.slice(mid).reduce((sum, point) => sum + point.value, 0)
  if (firstHalf === 0) return secondHalf > 0 ? 100 : 0
  return Math.round(((secondHalf - firstHalf) / firstHalf) * 1_000) / 10
}

export function formatChartTime(timestamp: string, bucketMs?: number): string {
  const date = parseDate(timestamp)
  if (Number.isNaN(date.getTime())) return '--:--'
  if (bucketMs && bucketMs > 60 * 60 * 1_000) {
    return date.toLocaleString('zh-CN', {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
    })
  }
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

export function rangeLabel(range?: DashboardRange): string {
  return RANGE_LABELS[range ?? '1d'] ?? RANGE_LABELS['1d']
}

export function dashboardTrendParams(
  range: DashboardRange,
  from: string,
  to: string,
): DashboardStatsParams {
  if (range !== 'custom') return { range }
  const fromMs = dateTimeLocalToMillis(from)
  const toMs = dateTimeLocalToMillis(to)
  if (
    !fromMs
    || !toMs
    || Number(fromMs) >= Number(toMs)
    || Number(toMs) - Number(fromMs) > 90 * DAY_MS
  ) return { range: '1d' }
  return { range, from: fromMs, to: toMs }
}

export function customRangeError(
  range: DashboardRange,
  from: string,
  to: string,
): string | null {
  if (range !== 'custom') return null
  const fromMs = dateTimeLocalToMillis(from)
  const toMs = dateTimeLocalToMillis(to)
  if (!fromMs || !toMs) return '请选择完整的开始和结束时间。数据暂按近 24 小时展示。'
  if (Number(fromMs) >= Number(toMs)) return '结束时间必须晚于开始时间。数据暂按近 24 小时展示。'
  if (Number(toMs) - Number(fromMs) > 90 * DAY_MS) return '自定义范围最多为 90 天。'
  return null
}

export function toDateTimeLocal(timestamp: number): string {
  const date = new Date(timestamp)
  const localDate = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return localDate.toISOString().slice(0, 16)
}

export function dateTimeLocalToMillis(value: string): string | undefined {
  const timestamp = new Date(value).getTime()
  return Number.isFinite(timestamp) ? String(timestamp) : undefined
}

export function timestampMs(value: string): number {
  const numeric = Number(value)
  if (Number.isFinite(numeric)) return numeric
  const parsed = parseDate(value).getTime()
  return Number.isFinite(parsed) ? parsed : 0
}

export function formatUsd(value: number, digits = 4): string {
  return `$${value.toFixed(digits)}`
}

export function formatPercentValue(value: number): string {
  return `${value.toFixed(1)}%`
}

export function formatRate(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0'
  if (value >= 1_000) return formatNumber(value)
  if (value >= 100) return value.toFixed(0)
  if (value >= 10) return value.toFixed(1).replace(/\.0$/, '')
  if (value >= 1) return value.toFixed(2).replace(/\.?0+$/, '')
  return value.toFixed(2)
}

export function tokenBreakdownDescription(
  inputTokens: number,
  outputTokens: number,
  cacheWriteTokens: number,
  cacheReadTokens: number,
): string {
  return `入 ${formatNumber(inputTokens)} / 出 ${formatNumber(outputTokens)} / Cache ${formatNumber(cacheWriteTokens + cacheReadTokens)}`
}

export function currentLogFilters(
  range: DashboardRange,
  customFrom: string,
  customTo: string,
  now = Date.now(),
) {
  if (range === 'custom') {
    return { dateFrom: customFrom || undefined, dateTo: customTo || undefined }
  }
  return {
    dateFrom: toDateTimeLocal(now - RANGE_MS[range]),
    dateTo: toDateTimeLocal(now),
  }
}

export function buildModelUsageRows(
  logs: RequestLog[],
  fallback: DashboardStats['topModels'],
): ModelUsageRow[] {
  if (logs.length === 0) {
    return fallback.slice(0, 6).map((item) => ({
      model: item.model,
      provider: item.provider,
      requests: item.requests,
      tokens: 0,
      cost: 0,
    }))
  }

  const rows = new Map<string, ModelUsageRow>()
  for (const log of logs) {
    const key = `${log.provider}:${log.resolvedModel || log.model}`
    const current = rows.get(key) ?? {
      model: log.resolvedModel || log.model,
      provider: log.provider,
      requests: 0,
      tokens: 0,
      cost: 0,
    }
    current.requests += 1
    current.tokens += log.totalTokens
      ?? log.inputTokens + log.outputTokens + (log.cacheWriteTokens || 0) + (log.cacheReadTokens || 0)
    current.cost += log.costEstimate || 0
    rows.set(key, current)
  }

  return compactModelUsageRows(Array.from(rows.values()))
}

export function compactModelUsageRows(
  rows: ModelUsageRow[],
  limit = 6,
): ModelUsageRow[] {
  const sorted = [...rows].sort(
    (left, right) => right.tokens - left.tokens || right.requests - left.requests,
  )
  if (limit <= 0) return []
  if (sorted.length <= limit) return sorted
  if (limit === 1) {
    return [aggregateModelUsageTail(sorted)]
  }
  const visible = sorted.slice(0, limit - 1)
  visible.push(aggregateModelUsageTail(sorted.slice(limit - 1)))
  return visible
}

function aggregateModelUsageTail(rows: ModelUsageRow[]): ModelUsageRow {
  return rows.reduce<ModelUsageRow>((total, row) => ({
    model: '其他模型',
    provider: 'multiple',
    requests: total.requests + row.requests,
    tokens: total.tokens + row.tokens,
    cost: total.cost + row.cost,
  }), {
    model: '其他模型',
    provider: 'multiple',
    requests: 0,
    tokens: 0,
    cost: 0,
  })
}

export function buildTokenTrend(
  logs: RequestLog[],
  startMs: number,
  endMs: number,
  bucketMs: number,
): TokenTrendPoint[] {
  const safeStart = Number.isFinite(startMs) ? startMs : 0
  const safeEnd = Number.isFinite(endMs) && endMs > safeStart ? endMs : safeStart + DAY_MS
  const safeBucket = Math.max(bucketMs || 60 * 60 * 1_000, 30 * 60 * 1_000)
  const bucketCount = Math.min(48, Math.max(1, Math.ceil((safeEnd - safeStart) / safeBucket)))
  const buckets = Array.from({ length: bucketCount }, (_, index) => {
    const bucketStart = safeStart + index * safeBucket
    return {
      start: bucketStart,
      end: index === bucketCount - 1 ? safeEnd + 1 : bucketStart + safeBucket,
      time: formatChartTime(String(bucketStart), safeBucket),
      input: 0,
      output: 0,
      cacheWrite: 0,
      cacheRead: 0,
    }
  })

  for (const log of logs) {
    const time = timestampMs(log.timestamp)
    const index = Math.floor((time - safeStart) / safeBucket)
    if (index < 0 || index >= buckets.length) continue
    const bucket = buckets[index]
    if (time < bucket.start || time >= bucket.end) continue
    bucket.input += log.inputTokens
    bucket.output += log.outputTokens
    bucket.cacheWrite += log.cacheWriteTokens || 0
    bucket.cacheRead += log.cacheReadTokens || 0
  }

  return buckets.map((bucket) => {
    const billedInput = bucket.input + bucket.cacheWrite + bucket.cacheRead
    return {
      time: bucket.time,
      input: bucket.input,
      output: bucket.output,
      cacheWrite: bucket.cacheWrite,
      cacheRead: bucket.cacheRead,
      cacheHitRate: billedInput > 0 ? Math.round((bucket.cacheRead / billedInput) * 1_000) / 10 : 0,
    }
  })
}

export function providerTokens(provider: DashboardStats['providerHealth'][number]): number {
  return (
    (provider.inputTokensTotal || 0)
    + (provider.outputTokensTotal || 0)
    + (provider.cacheWriteTokensTotal || 0)
    + (provider.cacheReadTokensTotal || 0)
  )
}

export function statusText(status: DashboardStats['providerHealth'][number]['status']): string {
  if (status === 'healthy') return '健康'
  if (status === 'degraded') return '降级'
  if (status === 'cooldown') return '冷却'
  return '不可用'
}
