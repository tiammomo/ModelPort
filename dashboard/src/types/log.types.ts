import type { ProviderProtocol } from './model.types'

export type RequestStatus = 'success' | 'error' | 'timeout'
export type StreamMode = 'stream' | 'non-stream'

export interface RequestLog {
  id: string
  timestamp: string
  userId: string
  username: string
  model: string
  resolvedModel: string
  provider: string
  protocol: ProviderProtocol
  stream: StreamMode
  status: RequestStatus
  statusCode: number
  inputTokens: number
  outputTokens: number
  latencyMs: number
  errorMessage: string | null
}

export interface LogFilters {
  userId?: string
  model?: string
  provider?: string
  status?: RequestStatus
  stream?: StreamMode
  dateFrom?: string
  dateTo?: string
  search?: string
}

export interface LatencyStats {
  p50: number
  p90: number
  p95: number
  p99: number
  avg: number
  max: number
  byModel: Record<string, { p50: number; p95: number; avg: number }>
  byProvider: Record<string, { p50: number; p95: number; avg: number }>
}
