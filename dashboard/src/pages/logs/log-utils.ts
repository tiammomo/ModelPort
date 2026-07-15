import type { LogFilters, RequestLog, RequestStatus, StreamMode } from '@/types'

// ── Display helpers ──────────────────────────────────────────────

export function compactDetail(log: RequestLog): string {
  const pricing = log.modelPricing
  if (!pricing) return `模型: ${log.resolvedModel} · 缓存创建: ${formatInteger(log.cacheWriteTokens || 0)}`
  return `模型: ${log.resolvedModel} · 缓存创建: ${pricing.cacheWritePerMillion}/1M · 缓存命中: ${pricing.cacheReadPerMillion}/1M`
}

export function costFormula(log: RequestLog): string {
  const pricing = log.modelPricing
  if (!pricing) return '无计价明细'
  return [
    `${formatInteger(log.inputTokens)} × $${pricing.inputPerMillion}/1M`,
    `${formatInteger(log.cacheWriteTokens || 0)} × $${pricing.cacheWritePerMillion}/1M`,
    `${formatInteger(log.cacheReadTokens || 0)} × $${pricing.cacheReadPerMillion}/1M`,
    `${formatInteger(log.outputTokens)} × $${pricing.outputPerMillion}/1M`,
    `= ${formatMoney(log.costEstimate || 0, 6)}`,
  ].join(' + ')
}

// ── Tone / style helpers ─────────────────────────────────────────

export function rowTone(status: RequestStatus): string {
  if (status === 'error') return 'border-l-rose-400 bg-rose-50/30 hover:bg-rose-50/60 dark:bg-rose-950/20 dark:hover:bg-rose-950/30'
  if (status === 'timeout') return 'border-l-amber-400 bg-amber-50/30 hover:bg-amber-50/60 dark:bg-amber-950/20 dark:hover:bg-amber-950/30'
  return 'border-l-transparent'
}

export function providerTone(provider: string): string {
  const key = provider.toLowerCase()
  if (key.includes('mimo')) return 'border-orange-200 bg-orange-50 text-orange-700 dark:border-orange-900 dark:bg-orange-950/40 dark:text-orange-300'
  if (key.includes('deepseek')) return 'border-cyan-200 bg-cyan-50 text-cyan-700 dark:border-cyan-900 dark:bg-cyan-950/40 dark:text-cyan-300'
  if (key.includes('openai')) return 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300'
  if (key.includes('anthropic')) return 'border-violet-200 bg-violet-50 text-violet-700 dark:border-violet-900 dark:bg-violet-950/40 dark:text-violet-300'
  if (key.includes('gemini')) return 'border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-900 dark:bg-blue-950/40 dark:text-blue-300'
  if (key.includes('dashscope')) return 'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-300'
  return 'border-slate-200 bg-slate-50 text-slate-700 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300'
}

export function latencyTone(value: number): string {
  if (value >= 6000) return 'bg-rose-500'
  if (value >= 2500) return 'bg-amber-500'
  return 'bg-emerald-500'
}

// ── Label helpers ────────────────────────────────────────────────

export function protocolLabel(value?: string): string {
  if (value === 'openai-compat') return 'OpenAI-compatible'
  if (value === 'anthropic') return 'Anthropic Messages'
  return value || 'default'
}

export function clientProtocolLabel(value?: RequestLog['clientProtocol']): string {
  if (value === 'openai-chat-completions') return 'OpenAI Chat Completions'
  if (value === 'anthropic-messages') return 'Anthropic Messages'
  return '客户端协议未记录'
}

export function billingModeLabel(value?: string): string {
  if (value === 'upstream-returned') return '上游返回'
  if (value === 'metrics-fallback') return '进程指标回退'
  return value || '本地估算'
}

// ── Parsing helpers ──────────────────────────────────────────────

export function parseLogDate(value: string): Date | null {
  const date = /^\d+$/.test(value) ? new Date(Number(value)) : new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return date
}

export function shortId(value: string): string {
  if (value.length <= 24) return value
  return `${value.slice(0, 18)}...${value.slice(-4)}`
}

// ── Number formatting ────────────────────────────────────────────

export function formatInteger(value: number): string {
  return Math.round(value).toLocaleString('en-US')
}

export function formatCompactTokenCount(value: number): string {
  if (value >= 1_000_000) return `${trimFixed(value / 1_000_000)}M`
  if (value >= 1_000) return `${trimFixed(value / 1_000)}K`
  return formatInteger(value)
}

export function trimFixed(value: number): string {
  return value.toFixed(1).replace(/\.0$/, '')
}

export function formatMoney(value: number, digits: number): string {
  return `$${value.toFixed(digits)}`
}

export function formatPercent(value: number): string {
  return `${value.toFixed(1)}%`
}

// ── Time range helpers ───────────────────────────────────────────

export type TimeRange = '1h' | '6h' | '24h' | '7d'

const TIME_RANGE_MS: Record<TimeRange, number> = {
  '1h': 60 * 60 * 1000,
  '6h': 6 * 60 * 60 * 1000,
  '24h': 24 * 60 * 60 * 1000,
  '7d': 7 * 24 * 60 * 60 * 1000,
}

export function timeRangeToDates(
  range: TimeRange,
  nowMs = Date.now(),
): { dateFrom: string; dateTo: string } {
  return {
    dateFrom: toLocalDateTimeInput(nowMs - TIME_RANGE_MS[range]),
    dateTo: toLocalDateTimeInput(nowMs),
  }
}

export function toLocalDateTimeInput(timestamp: number): string {
  const date = new Date(timestamp)
  const localDate = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return localDate.toISOString().slice(0, 16)
}

// ── Shareable view state ─────────────────────────────────────────

const LOG_STATUSES = new Set<RequestStatus>(['success', 'error', 'timeout'])
const STREAM_MODES = new Set<StreamMode>(['stream', 'non-stream'])
const LOG_PAGE_SIZES = new Set([20, 50, 100, 200])

export function logViewStateFromSearchParams(params: URLSearchParams): {
  filters: LogFilters
  page: number
  pageSize: number
} {
  const status = params.get('status') as RequestStatus | null
  const stream = params.get('stream') as StreamMode | null
  const page = positiveInteger(params.get('page')) ?? 1
  const requestedPageSize = positiveInteger(params.get('pageSize')) ?? 50

  return {
    filters: compactFilters({
      search: params.get('search') || undefined,
      provider: params.get('provider') || undefined,
      model: params.get('model') || undefined,
      userId: params.get('userId') || undefined,
      apiKeyId: params.get('apiKeyId') || undefined,
      username: params.get('username') || undefined,
      group: params.get('group') || undefined,
      status: status && LOG_STATUSES.has(status) ? status : undefined,
      stream: stream && STREAM_MODES.has(stream) ? stream : undefined,
      dateFrom: dateParamToLocalInput(params.get('dateFrom')),
      dateTo: dateParamToLocalInput(params.get('dateTo')),
    }),
    page,
    pageSize: LOG_PAGE_SIZES.has(requestedPageSize) ? requestedPageSize : 50,
  }
}

export function logViewSearchParams(filters: LogFilters, page: number, pageSize: number): URLSearchParams {
  const params = new URLSearchParams()
  const append = (name: string, value?: string) => {
    const normalized = value?.trim()
    if (normalized) params.set(name, normalized)
  }

  append('search', filters.search)
  append('provider', filters.provider)
  append('model', filters.model)
  append('userId', filters.userId)
  append('apiKeyId', filters.apiKeyId)
  append('username', filters.username)
  append('group', filters.group)
  append('status', filters.status)
  append('stream', filters.stream)
  appendDateParam(params, 'dateFrom', filters.dateFrom)
  appendDateParam(params, 'dateTo', filters.dateTo)
  if (page > 1) params.set('page', String(Math.trunc(page)))
  if (pageSize !== 50 && LOG_PAGE_SIZES.has(pageSize)) params.set('pageSize', String(pageSize))
  return params
}

function compactFilters(filters: LogFilters): LogFilters {
  return Object.fromEntries(
    Object.entries(filters).filter(([, value]) => value !== undefined && value !== ''),
  ) as LogFilters
}

function positiveInteger(value: string | null): number | null {
  if (!value) return null
  const parsed = Number(value)
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null
}

function dateParamToLocalInput(value: string | null): string | undefined {
  if (!value) return undefined
  const timestamp = /^\d+$/.test(value) ? Number(value) : new Date(value).getTime()
  return Number.isFinite(timestamp) ? toLocalDateTimeInput(timestamp) : undefined
}

function appendDateParam(params: URLSearchParams, name: string, value?: string) {
  if (!value) return
  const timestamp = new Date(value).getTime()
  if (Number.isFinite(timestamp)) params.set(name, String(timestamp))
}

// ── Provider extraction ──────────────────────────────────────────

export function extractProviders(logs: RequestLog[]): string[] {
  return mergeProviderOptions([], logs)
}

export function mergeProviderOptions(
  configuredProviderIds: string[],
  logs: RequestLog[],
  selectedProvider?: string,
): string[] {
  const providers = new Map<string, string>()
  const add = (value?: string) => {
    const provider = value?.trim()
    if (provider && !providers.has(provider.toLowerCase())) {
      providers.set(provider.toLowerCase(), provider)
    }
  }

  configuredProviderIds.forEach(add)
  logs.forEach((log) => add(log.provider))
  add(selectedProvider)
  return Array.from(providers.values()).sort((left, right) => left.localeCompare(right))
}

export function clampLogPage(page: number, totalPages: number): number {
  return Math.min(Math.max(page, 1), Math.max(totalPages, 1))
}
