import type { ElementType } from 'react'
import type { LogSummary } from '@/types'
import { cn } from '@/lib/utils'
import { Activity, BadgeDollarSign, DatabaseZap, Gauge } from 'lucide-react'
import { formatInteger, formatMoney, formatPercent } from './log-utils'

// ── Summary metric card ──────────────────────────────────────────

export function SummaryMetric({
  label,
  value,
  helper,
  icon: Icon,
  tone,
}: {
  label: string
  value: string
  helper: string
  icon: ElementType
  tone: 'sky' | 'emerald' | 'amber' | 'rose'
}) {
  const tones = {
    sky: 'bg-sky-50 text-sky-700 ring-sky-100',
    emerald: 'bg-emerald-50 text-emerald-700 ring-emerald-100',
    amber: 'bg-amber-50 text-amber-700 ring-amber-100',
    rose: 'bg-rose-50 text-rose-700 ring-rose-100',
  }

  return (
    <div className="rounded-lg border bg-card p-4 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm text-muted-foreground">{label}</p>
          <p className="mt-1 truncate font-mono text-2xl font-semibold tracking-tight">{value}</p>
        </div>
        <div className={cn('flex h-10 w-10 shrink-0 items-center justify-center rounded-lg ring-1', tones[tone])}>
          <Icon className="h-5 w-5" />
        </div>
      </div>
      <p className="mt-3 truncate text-xs text-muted-foreground">{helper}</p>
    </div>
  )
}

// ── Summary cards grid ───────────────────────────────────────────

export function LogsSummaryGrid({
  summary,
}: {
  summary?: LogSummary
}) {
  const totalRequests = summary?.totalRequests || 0
  const successRequests = summary?.successRequests || 0
  const successRate = totalRequests > 0 ? (successRequests / totalRequests) * 100 : 0
  const cacheTokens = (summary?.totalCacheWriteTokens || 0) + (summary?.totalCacheReadTokens || 0)

  return (
    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <SummaryMetric
        label="消耗费用"
        value={formatMoney(summary?.totalCostEstimate || 0, 4)}
        helper={`${formatInteger(totalRequests)} 次调用`}
        icon={BadgeDollarSign}
        tone="sky"
      />
      <SummaryMetric
        label="成功率"
        value={formatPercent(successRate)}
        helper={`${formatInteger(successRequests)} 成功 / ${formatInteger(totalRequests)} 总计`}
        icon={Activity}
        tone="emerald"
      />
      <SummaryMetric
        label="Tokens"
        value={formatInteger(summary?.totalTokens || 0)}
        helper={`TPM ${formatInteger(summary?.tpm || 0)} · RPM ${(summary?.rpm || 0).toFixed(2)}`}
        icon={Gauge}
        tone="amber"
      />
      <SummaryMetric
        label="缓存"
        value={formatInteger(cacheTokens)}
        helper={`读 ${formatInteger(summary?.totalCacheReadTokens || 0)} / 写 ${formatInteger(summary?.totalCacheWriteTokens || 0)}`}
        icon={DatabaseZap}
        tone="rose"
      />
    </div>
  )
}
