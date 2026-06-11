import type { RequestLog, LogFilters, LatencyStats, LogSummary } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockLogs } from '@/mock'

const latencyValues = mockLogs.map((log) => log.latencyMs).sort((a, b) => a - b)

function percentile(values: number[], p: number) {
  if (values.length === 0) return 0
  const index = Math.min(values.length - 1, Math.floor(values.length * p))
  return values[index]
}

function mockLatencyStats(): LatencyStats {
  const avg = latencyValues.reduce((sum, value) => sum + value, 0) / Math.max(latencyValues.length, 1)
  const byProvider: LatencyStats['byProvider'] = {}
  const byModel: LatencyStats['byModel'] = {}

  for (const log of mockLogs) {
    for (const [key, bucket] of [[log.provider, byProvider], [log.model, byModel]] as const) {
      const current = bucket[key] || { p50: 0, p95: 0, avg: 0 }
      current.avg = Math.round((current.avg + log.latencyMs) / (current.avg === 0 ? 1 : 2))
      current.p50 = Math.max(current.p50, Math.round(log.latencyMs * 0.7))
      current.p95 = Math.max(current.p95, log.latencyMs)
      bucket[key] = current
    }
  }

  return {
    p50: percentile(latencyValues, 0.5),
    p90: percentile(latencyValues, 0.9),
    p95: percentile(latencyValues, 0.95),
    p99: percentile(latencyValues, 0.99),
    avg: Math.round(avg),
    max: latencyValues.at(-1) || 0,
    byModel,
    byProvider,
  }
}

function logTime(log: RequestLog) {
  const timestamp = Number(log.timestamp)
  return Number.isFinite(timestamp) ? timestamp : new Date(log.timestamp).getTime()
}

function summarizeLogs(logs: RequestLog[]): LogSummary {
  const totalInputTokens = logs.reduce((sum, log) => sum + log.inputTokens, 0)
  const totalOutputTokens = logs.reduce((sum, log) => sum + log.outputTokens, 0)
  const totalCacheWriteTokens = logs.reduce((sum, log) => sum + (log.cacheWriteTokens || 0), 0)
  const totalCacheReadTokens = logs.reduce((sum, log) => sum + (log.cacheReadTokens || 0), 0)
  const totalTokens = totalInputTokens + totalOutputTokens + totalCacheWriteTokens + totalCacheReadTokens
  const totalCostEstimate = logs.reduce((sum, log) => sum + (log.costEstimate || 0), 0)
  const timestamps = logs.map(logTime).filter(Number.isFinite).sort((a, b) => a - b)
  const minutes = timestamps.length > 1
    ? Math.max((timestamps[timestamps.length - 1] - timestamps[0]) / 60000, 1)
    : 1

  return {
    totalRequests: logs.length,
    successRequests: logs.filter((log) => log.status === 'success').length,
    totalInputTokens,
    totalOutputTokens,
    totalCacheWriteTokens,
    totalCacheReadTokens,
    totalTokens,
    totalCostEstimate,
    rpm: logs.length / minutes,
    tpm: totalTokens / minutes,
  }
}

export const logsService = {
  getLogs: async (
    filters?: LogFilters,
    page = 1,
    pageSize = 20
  ): Promise<{ logs: RequestLog[]; total: number; summary: LogSummary }> => {
    const data = isMockMode
      ? { logs: mockLogs, total: mockLogs.length }
      : await api.get<{ logs: RequestLog[]; total: number }>('/admin/logs')
    let filtered = data.logs

    if (filters?.userId) filtered = filtered.filter((log) => log.userId === filters.userId)
    if (filters?.model) filtered = filtered.filter((log) => log.model.includes(filters.model!))
    if (filters?.provider) filtered = filtered.filter((log) => log.provider === filters.provider)
    if (filters?.group) filtered = filtered.filter((log) => (log.group || log.apiKeyGroup || '').includes(filters.group!))
    if (filters?.username) filtered = filtered.filter((log) => log.username.includes(filters.username!))
    if (filters?.status) filtered = filtered.filter((log) => log.status === filters.status)
    if (filters?.stream) filtered = filtered.filter((log) => log.stream === filters.stream)
    if (filters?.dateFrom) {
      const from = new Date(filters.dateFrom).getTime()
      if (Number.isFinite(from)) filtered = filtered.filter((log) => logTime(log) >= from)
    }
    if (filters?.dateTo) {
      const to = new Date(filters.dateTo).getTime()
      if (Number.isFinite(to)) filtered = filtered.filter((log) => logTime(log) <= to)
    }
    if (filters?.search) {
      const search = filters.search.toLowerCase()
      filtered = filtered.filter(
        (log) =>
          log.id.toLowerCase().includes(search) ||
          log.username.toLowerCase().includes(search) ||
          log.model.toLowerCase().includes(search) ||
          log.resolvedModel.toLowerCase().includes(search) ||
          log.provider.toLowerCase().includes(search) ||
          (log.apiKeyName || '').toLowerCase().includes(search) ||
          (log.group || log.apiKeyGroup || '').toLowerCase().includes(search)
      )
    }

    const total = filtered.length
    const summary = summarizeLogs(filtered)
    const start = (page - 1) * pageSize
    const result = { logs: filtered.slice(start, start + pageSize), total, summary }
    return isMockMode ? mockDelay(result) : result
  },

  getLogById: async (id: string): Promise<RequestLog> => {
    const data = isMockMode
      ? { logs: mockLogs, total: mockLogs.length }
      : await api.get<{ logs: RequestLog[]; total: number }>('/admin/logs')
    const log = data.logs.find((item) => item.id === id)
    if (!log) throw new Error('日志不存在')
    return isMockMode ? mockDelay(log) : log
  },

  getLatencyStats: (): Promise<LatencyStats> =>
    isMockMode ? mockDelay(mockLatencyStats()) : api.get('/admin/latency'),
}
