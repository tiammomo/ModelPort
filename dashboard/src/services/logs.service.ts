import type { RequestLog, LogFilters, LatencyStats, LogSummary } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockLogs } from '@/mock'

type LogsResponse = { logs: RequestLog[]; total: number; summary: LogSummary }

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
    toolUseRequests: logs.filter((log) => log.toolUseRequested).length,
    toolUseSuccessRequests: logs.filter((log) => log.toolUseRequested && log.status === 'success').length,
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

function appendFilter(params: URLSearchParams, name: string, value?: string) {
  const normalized = value?.trim()
  if (normalized) params.set(name, normalized)
}

function appendEpochMillis(params: URLSearchParams, name: string, value?: string) {
  if (!value) return
  const timestamp = new Date(value).getTime()
  if (Number.isFinite(timestamp)) params.set(name, String(timestamp))
}

function logsPath(filters: LogFilters | undefined, page: number, pageSize: number) {
  const params = new URLSearchParams()
  params.set('page', String(Math.max(1, Math.trunc(page))))
  params.set('pageSize', String(Math.min(500, Math.max(1, Math.trunc(pageSize)))))
  appendFilter(params, 'status', filters?.status)
  appendFilter(params, 'provider', filters?.provider)
  appendFilter(params, 'model', filters?.model)
  appendFilter(params, 'userId', filters?.userId)
  appendFilter(params, 'apiKeyId', filters?.apiKeyId)
  appendEpochMillis(params, 'dateFrom', filters?.dateFrom)
  appendEpochMillis(params, 'dateTo', filters?.dateTo)
  appendFilter(params, 'search', filters?.search)
  appendFilter(params, 'username', filters?.username)
  appendFilter(params, 'group', filters?.group)
  appendFilter(params, 'stream', filters?.stream)
  appendFilter(params, 'toolUse', filters?.toolUse)
  return `/admin/logs?${params.toString()}`
}

export const logsService = {
  getLogs: async (
    filters?: LogFilters,
    page = 1,
    pageSize = 20
  ): Promise<LogsResponse> => {
    if (!isMockMode) {
      return api.get<LogsResponse>(logsPath(filters, page, pageSize))
    }

    let filtered = mockLogs

    if (filters?.userId) filtered = filtered.filter((log) => log.userId === filters.userId)
    if (filters?.apiKeyId) filtered = filtered.filter((log) => log.apiKeyId === filters.apiKeyId)
    if (filters?.model) {
      const model = filters.model.toLowerCase()
      filtered = filtered.filter(
        (log) => log.model.toLowerCase().includes(model) || log.resolvedModel.toLowerCase().includes(model),
      )
    }
    if (filters?.provider) filtered = filtered.filter((log) => log.provider === filters.provider)
    if (filters?.group) filtered = filtered.filter((log) => (log.group || log.apiKeyGroup || '').includes(filters.group!))
    if (filters?.username) filtered = filtered.filter((log) => log.username.includes(filters.username!))
    if (filters?.status) filtered = filtered.filter((log) => log.status === filters.status)
    if (filters?.stream) filtered = filtered.filter((log) => log.stream === filters.stream)
    if (filters?.toolUse) {
      const requested = filters.toolUse === 'requested'
      filtered = filtered.filter((log) => Boolean(log.toolUseRequested) === requested)
    }
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
          (log.requestId || '').toLowerCase().includes(search) ||
          log.userId.toLowerCase().includes(search) ||
          log.username.toLowerCase().includes(search) ||
          log.model.toLowerCase().includes(search) ||
          log.resolvedModel.toLowerCase().includes(search) ||
          log.provider.toLowerCase().includes(search) ||
          (log.apiKeyId || '').toLowerCase().includes(search) ||
          (log.apiKeyName || '').toLowerCase().includes(search) ||
          (log.group || log.apiKeyGroup || '').toLowerCase().includes(search) ||
          (log.errorMessage || '').toLowerCase().includes(search) ||
          (log.detail || '').toLowerCase().includes(search)
      )
    }

    const total = filtered.length
    const summary = summarizeLogs(filtered)
    const start = (page - 1) * pageSize
    const result = { logs: filtered.slice(start, start + pageSize), total, summary }
    return mockDelay(result)
  },

  getLogById: async (id: string): Promise<RequestLog> => {
    if (!isMockMode) {
      return api.get<RequestLog>(`/admin/logs/${encodeURIComponent(id)}`)
    }

    const log = mockLogs.find((item) => item.id === id)
    if (!log) throw new Error('日志不存在')
    return mockDelay(log)
  },

  getLatencyStats: (): Promise<LatencyStats> =>
    isMockMode ? mockDelay(mockLatencyStats()) : api.get('/admin/latency'),
}
