import { useQuery } from '@tanstack/react-query'
import { logsService } from '@/services/logs.service'
import { queryKeys } from './use-dashboard'
import type { LogFilters } from '@/types'

export function useLogs(filters?: LogFilters, page = 1, pageSize = 20) {
  return useQuery({
    queryKey: queryKeys.logs({ ...filters, page, pageSize }),
    queryFn: () => logsService.getLogs(filters, page, pageSize),
  })
}

export function useLogById(id: string) {
  return useQuery({
    queryKey: queryKeys.logById(id),
    queryFn: () => logsService.getLogById(id),
    enabled: !!id,
  })
}

export function useLatencyStats() {
  return useQuery({
    queryKey: queryKeys.latencyStats,
    queryFn: () => logsService.getLatencyStats(),
  })
}
