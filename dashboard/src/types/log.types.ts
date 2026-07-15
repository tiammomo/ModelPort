import type { ProviderProtocol } from './model.types'

export type RequestStatus = 'success' | 'error' | 'timeout'
export type StreamMode = 'stream' | 'non-stream'

export interface RequestLog {
  id: string
  requestId?: string | null
  attemptId?: string | null
  timestamp: string
  userId: string
  username: string
  apiKeyId?: string | null
  apiKeyName?: string | null
  apiKeyGroup?: string | null
  tokenName?: string | null
  group?: string | null
  channelId?: string
  channelName?: string
  model: string
  resolvedModel: string
  provider: string
  protocol: ProviderProtocol
  clientProtocol?: 'anthropic-messages' | 'openai-chat-completions'
  requestType?: 'consume' | 'error'
  stream: StreamMode
  status: RequestStatus
  statusCode: number
  terminalReason?: string
  inputTokens: number
  outputTokens: number
  cacheWriteTokens?: number
  cacheReadTokens?: number
  billedInputTokens?: number
  totalTokens?: number
  cacheHitRate?: number
  costEstimate?: number
  modelPricing?: {
    inputPerMillion: number
    outputPerMillion: number
    cacheWritePerMillion: number
    cacheReadPerMillion: number
  }
  costBreakdown?: {
    inputCost: number
    outputCost: number
    cacheWriteCost: number
    cacheReadCost: number
    totalCost: number
  }
  latencyMs: number
  firstByteLatencyMs?: number
  retryCount?: number
  clientIp?: string | null
  requestPath?: string
  billingMode?: string
  detail?: string
  errorMessage: string | null
}

export interface LogFilters {
  userId?: string
  apiKeyId?: string
  model?: string
  provider?: string
  group?: string
  username?: string
  status?: RequestStatus
  stream?: StreamMode
  dateFrom?: string
  dateTo?: string
  search?: string
}

export interface LogSummary {
  totalRequests: number
  successRequests: number
  totalInputTokens: number
  totalOutputTokens: number
  totalCacheWriteTokens: number
  totalCacheReadTokens: number
  totalTokens: number
  totalCostEstimate: number
  rpm: number
  tpm: number
}

export interface LatencyStats {
  p50: number
  p90: number
  p95: number
  p99: number
  avg: number
  max: number
  sampleCount?: number
  percentilesEstimated?: boolean
  byModel: Record<string, { p50: number; p95: number; avg: number }>
  byProvider: Record<string, { p50: number; p95: number; avg: number }>
}
