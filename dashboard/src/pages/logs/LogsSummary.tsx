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
    sky: 'bg-sky-50 text-sky-700 ring-sky-100 dark:bg-sky-950/40 dark:text-sky-300 dark:ring-sky-900',
    emerald: 'bg-emerald-50 text-emerald-700 ring-emerald-100 dark:bg-emerald-950/40 dark:text-emerald-300 dark:ring-emerald-900',
    amber: 'bg-amber-50 text-amber-700 ring-amber-100 dark:bg-amber-950/40 dark:text-amber-300 dark:ring-amber-900',
    rose: 'bg-rose-50 text-rose-700 ring-rose-100 dark:bg-rose-950/40 dark:text-rose-300 dark:ring-rose-900',
  }

  return (
    <div className="rounded-lg border bg-card p-3 shadow-sm sm:p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm text-muted-foreground">{label}</p>
          <p className="mt-1 truncate font-mono text-xl font-semibold tracking-tight sm:text-2xl">{value}</p>
        </div>
        <div className={cn('flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ring-1 sm:h-10 sm:w-10', tones[tone])}>
          <Icon className="h-4 w-4 sm:h-5 sm:w-5" />
        </div>
      </div>
      <p className="mt-2 truncate text-xs text-muted-foreground sm:mt-3">{helper}</p>
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
    <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
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
        label="Token"
        value={formatInteger(summary?.totalTokens || 0)}
        helper={`TPM ${formatInteger(summary?.tpm || 0)} · RPM ${(summary?.rpm || 0).toFixed(2)}`}
        icon={Gauge}
        tone="amber"
      />
      <SummaryMetric
        label="缓存 Token"
        value={formatInteger(cacheTokens)}
        helper={`读 ${formatInteger(summary?.totalCacheReadTokens || 0)} / 写 ${formatInteger(summary?.totalCacheWriteTokens || 0)}`}
        icon={DatabaseZap}
        tone="rose"
      />
    </div>
  )
}
