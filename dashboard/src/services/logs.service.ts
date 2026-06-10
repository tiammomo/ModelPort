import type { RequestLog, LogFilters, LatencyStats } from '@/types'
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

export const logsService = {
  getLogs: async (
    filters?: LogFilters,
    page = 1,
    pageSize = 20
  ): Promise<{ logs: RequestLog[]; total: number }> => {
    const data = isMockMode
      ? { logs: mockLogs, total: mockLogs.length }
      : await api.get<{ logs: RequestLog[]; total: number }>('/admin/logs')
    let filtered = data.logs

    if (filters?.userId) filtered = filtered.filter((log) => log.userId === filters.userId)
    if (filters?.model) filtered = filtered.filter((log) => log.model.includes(filters.model!))
    if (filters?.provider) filtered = filtered.filter((log) => log.provider === filters.provider)
    if (filters?.status) filtered = filtered.filter((log) => log.status === filters.status)
    if (filters?.stream) filtered = filtered.filter((log) => log.stream === filters.stream)
    if (filters?.search) {
      const search = filters.search.toLowerCase()
      filtered = filtered.filter(
        (log) =>
          log.id.toLowerCase().includes(search) ||
          log.username.toLowerCase().includes(search) ||
          log.model.toLowerCase().includes(search)
      )
    }

    const total = filtered.length
    const start = (page - 1) * pageSize
    const result = { logs: filtered.slice(start, start + pageSize), total }
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
