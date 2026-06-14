import type { RequestLog, RequestStatus } from '@/types'

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
  if (status === 'error') return 'border-l-rose-400 bg-rose-50/30 hover:bg-rose-50/60'
  if (status === 'timeout') return 'border-l-amber-400 bg-amber-50/30 hover:bg-amber-50/60'
  return 'border-l-transparent'
}

export function providerTone(provider: string): string {
  const key = provider.toLowerCase()
  if (key.includes('mimo')) return 'border-orange-200 bg-orange-50 text-orange-700'
  if (key.includes('deepseek')) return 'border-cyan-200 bg-cyan-50 text-cyan-700'
  if (key.includes('openai')) return 'border-emerald-200 bg-emerald-50 text-emerald-700'
  if (key.includes('anthropic')) return 'border-violet-200 bg-violet-50 text-violet-700'
  if (key.includes('gemini')) return 'border-blue-200 bg-blue-50 text-blue-700'
  if (key.includes('dashscope')) return 'border-amber-200 bg-amber-50 text-amber-700'
  return 'border-slate-200 bg-slate-50 text-slate-700'
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

export function timeRangeToDates(range: TimeRange): { dateFrom: string; dateTo: string } {
  const now = new Date()
  const from = new Date(now.getTime() - TIME_RANGE_MS[range])
  return {
    dateFrom: from.toISOString().slice(0, 16),
    dateTo: now.toISOString().slice(0, 16),
  }
}

// ── Provider extraction ──────────────────────────────────────────

export function extractProviders(logs: RequestLog[]): string[] {
  const seen = new Set<string>()
  for (const log of logs) {
    if (log.provider) seen.add(log.provider.toLowerCase())
  }
  return Array.from(seen).sort()
}
