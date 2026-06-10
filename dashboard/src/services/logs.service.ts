import type { RequestLog, LogFilters, LatencyStats } from '@/types'
import { api } from '@/lib/api-client'

export const logsService = {
  getLogs: async (
    filters?: LogFilters,
    page = 1,
    pageSize = 20
  ): Promise<{ logs: RequestLog[]; total: number }> => {
    const data = await api.get<{ logs: RequestLog[]; total: number }>('/admin/logs')
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
    return { logs: filtered.slice(start, start + pageSize), total }
  },

  getLogById: async (id: string): Promise<RequestLog> => {
    const data = await api.get<{ logs: RequestLog[]; total: number }>('/admin/logs')
    const log = data.logs.find((item) => item.id === id)
    if (!log) throw new Error('日志不存在')
    return log
  },

  getLatencyStats: (): Promise<LatencyStats> => api.get('/admin/latency'),
}
