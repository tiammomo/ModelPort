import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { useDashboard, useLogs } from '@/hooks'
import { MetricCard } from '@/components/shared/MetricCard'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { ErrorState } from '@/components/shared/ErrorState'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { cn, formatNumber, formatRelativeTime, parseDate, formatLatency } from '@/lib/utils'
import type { DashboardRange, DashboardStatsParams } from '@/services/dashboard.service'
import type { DashboardStats, RequestLog } from '@/types'
import {
  Activity,
  ArrowRight,
  Box,
  Clock,
  Database,
  Gauge,
  KeyRound,
  Layers,
  ScrollText,
  TrendingUp,
  WalletCards,
  Zap,
} from 'lucide-react'
import {
  AreaChart,
  Area,
  ComposedChart,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
  Legend,
  Line,
} from 'recharts'

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DAY_MS = 24 * 60 * 60 * 1000

const TREND_RANGES: Array<{ value: DashboardRange; label: string }> = [
  { value: '1d', label: '近1天' },
  { value: '3d', label: '近3天' },
  { value: '7d', label: '近7天' },
  { value: 'custom', label: '自定义' },
]

const RANGE_LABELS: Record<DashboardRange, string> = {
  '1d': '24小时',
  '3d': '3天',
  '7d': '7天',
  custom: '自定义',
}

const RANGE_MS: Record<Exclude<DashboardRange, 'custom'>, number> = {
  '1d': DAY_MS,
  '3d': 3 * DAY_MS,
  '7d': 7 * DAY_MS,
}

const PIE_COLORS = [
  'hsl(212 86% 48%)',
  'hsl(162 72% 38%)',
  'hsl(38 92% 50%)',
  'hsl(347 77% 56%)',
  'hsl(262 83% 62%)',
  'hsl(190 84% 42%)',
  'hsl(24 90% 54%)',
  'hsl(225 70% 58%)',
]

const TOOLTIP_STYLE = {
  contentStyle: {
    backgroundColor: 'var(--card)',
    border: '1px solid var(--border)',
    borderRadius: '8px',
    fontSize: '12px',
    boxShadow: '0 4px 12px rgba(0,0,0,0.1)',
  },
  labelStyle: { fontWeight: 600 },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function computeTrend(series: { value: number }[]): number {
  if (series.length < 2) return 0
  const mid = Math.floor(series.length / 2)
  const firstHalf = series.slice(0, mid).reduce((s, p) => s + p.value, 0)
  const secondHalf = series.slice(mid).reduce((s, p) => s + p.value, 0)
  if (firstHalf === 0) return secondHalf > 0 ? 100 : 0
  return Math.round(((secondHalf - firstHalf) / firstHalf) * 100 * 10) / 10
}

function formatChartTime(timestamp: string, bucketMs?: number): string {
  const date = parseDate(timestamp)
  if (Number.isNaN(date.getTime())) return '--:--'
  if (bucketMs && bucketMs > 60 * 60 * 1000) {
    return date.toLocaleString('zh-CN', {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
    })
  }
  return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}

function rangeLabel(range?: DashboardRange): string {
  return RANGE_LABELS[range ?? '1d'] ?? '24小时'
}

function dashboardTrendParams(
  range: DashboardRange,
  from: string,
  to: string,
): DashboardStatsParams {
  if (range !== 'custom') return { range }
  const fromMs = dateTimeLocalToMillis(from)
  const toMs = dateTimeLocalToMillis(to)
  if (!fromMs || !toMs || Number(fromMs) >= Number(toMs)) return { range: '1d' }
  return { range, from: fromMs, to: toMs }
}

function toDateTimeLocal(timestamp: number): string {
  const date = new Date(timestamp)
  const localDate = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return localDate.toISOString().slice(0, 16)
}

function dateTimeLocalToMillis(value: string): string | undefined {
  const timestamp = new Date(value).getTime()
  return Number.isFinite(timestamp) ? String(timestamp) : undefined
}

function timestampMs(value: string): number {
  const numeric = Number(value)
  if (Number.isFinite(numeric)) return numeric
  const parsed = parseDate(value).getTime()
  return Number.isFinite(parsed) ? parsed : 0
}

function formatUsd(value: number, digits = 4): string {
  return `$${value.toFixed(digits)}`
}

function formatPercentValue(value: number): string {
  return `${value.toFixed(1)}%`
}

function formatRate(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0'
  if (value >= 1000) return formatNumber(value)
  if (value >= 100) return value.toFixed(0)
  if (value >= 10) return value.toFixed(1).replace(/\.0$/, '')
  if (value >= 1) return value.toFixed(2).replace(/\.?0+$/, '')
  return value.toFixed(2)
}

function tokenBreakdownDescription(
  inputTokens: number,
  outputTokens: number,
  cacheWriteTokens: number,
  cacheReadTokens: number,
): string {
  return `入 ${formatNumber(inputTokens)} / 出 ${formatNumber(outputTokens)} / Cache ${formatNumber(cacheWriteTokens + cacheReadTokens)}`
}

function currentLogFilters(range: DashboardRange, customFrom: string, customTo: string) {
  if (range === 'custom') {
    return { dateFrom: customFrom || undefined, dateTo: customTo || undefined }
  }
  const now = Date.now()
  return {
    dateFrom: toDateTimeLocal(now - RANGE_MS[range]),
    dateTo: toDateTimeLocal(now),
  }
}

interface ModelUsageRow {
  model: string
  provider: string
  requests: number
  tokens: number
  cost: number
}

interface TokenTrendPoint {
  time: string
  input: number
  output: number
  cacheWrite: number
  cacheRead: number
  cacheHitRate: number
}

function buildModelUsageRows(logs: RequestLog[], fallback: DashboardStats['topModels']): ModelUsageRow[] {
  if (logs.length === 0) {
    return fallback.slice(0, 6).map((item) => ({
      model: item.model,
      provider: item.provider,
      requests: item.requests,
      tokens: 0,
      cost: 0,
    }))
  }

  const rows = new Map<string, ModelUsageRow>()
  for (const log of logs) {
    const key = `${log.provider}:${log.resolvedModel || log.model}`
    const current = rows.get(key) || {
      model: log.resolvedModel || log.model,
      provider: log.provider,
      requests: 0,
      tokens: 0,
      cost: 0,
    }
    current.requests += 1
    current.tokens += log.totalTokens ?? (log.inputTokens + log.outputTokens + (log.cacheWriteTokens || 0) + (log.cacheReadTokens || 0))
    current.cost += log.costEstimate || 0
    rows.set(key, current)
  }

  return Array.from(rows.values())
    .sort((a, b) => b.tokens - a.tokens || b.requests - a.requests)
    .slice(0, 6)
}

function buildTokenTrend(logs: RequestLog[], startMs: number, endMs: number, bucketMs: number): TokenTrendPoint[] {
  const safeStart = Number.isFinite(startMs) ? startMs : 0
  const safeEnd = Number.isFinite(endMs) && endMs > safeStart ? endMs : safeStart + DAY_MS
  const safeBucket = Math.max(bucketMs || 60 * 60 * 1000, 30 * 60 * 1000)
  const bucketCount = Math.min(48, Math.max(1, Math.ceil((safeEnd - safeStart) / safeBucket)))
  const buckets = Array.from({ length: bucketCount }, (_, index) => {
    const bucketStart = safeStart + index * safeBucket
    return {
      start: bucketStart,
      end: index === bucketCount - 1 ? safeEnd + 1 : bucketStart + safeBucket,
      time: formatChartTime(String(bucketStart), safeBucket),
      input: 0,
      output: 0,
      cacheWrite: 0,
      cacheRead: 0,
      cacheHitRate: 0,
    }
  })

  for (const log of logs) {
    const time = timestampMs(log.timestamp)
    const bucket = buckets.find((item) => time >= item.start && time < item.end)
    if (!bucket) continue
    bucket.input += log.inputTokens
    bucket.output += log.outputTokens
    bucket.cacheWrite += log.cacheWriteTokens || 0
    bucket.cacheRead += log.cacheReadTokens || 0
  }

  return buckets.map((bucket) => {
    const billedInput = bucket.input + bucket.cacheWrite + bucket.cacheRead
    return {
      time: bucket.time,
      input: bucket.input,
      output: bucket.output,
      cacheWrite: bucket.cacheWrite,
      cacheRead: bucket.cacheRead,
      cacheHitRate: billedInput > 0 ? Math.round((bucket.cacheRead / billedInput) * 1000) / 10 : 0,
    }
  })
}

function providerTokens(provider: DashboardStats['providerHealth'][number]): number {
  return (
    (provider.inputTokensTotal || 0) +
    (provider.outputTokensTotal || 0) +
    (provider.cacheWriteTokensTotal || 0) +
    (provider.cacheReadTokensTotal || 0)
  )
}

function statusText(status: DashboardStats['providerHealth'][number]['status']): string {
  if (status === 'healthy') return '健康'
  if (status === 'degraded') return '降级'
  if (status === 'cooldown') return '冷却'
  return '不可用'
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function RangeSelector({
  value,
  onChange,
  customFrom,
  customTo,
  onCustomFromChange,
  onCustomToChange,
}: {
  value: DashboardRange
  onChange: (r: DashboardRange) => void
  customFrom: string
  customTo: string
  onCustomFromChange: (v: string) => void
  onCustomToChange: (v: string) => void
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      {TREND_RANGES.map((option) => (
        <Button
          key={option.value}
          type="button"
          size="sm"
          variant={value === option.value ? 'default' : 'outline'}
          className={cn(
            'h-8 rounded-lg text-xs',
            value === option.value && 'shadow-sm',
          )}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </Button>
      ))}
      {value === 'custom' && (
        <div className="flex items-center gap-2 ml-1">
          <Input
            aria-label="开始时间"
            type="datetime-local"
            value={customFrom}
            onChange={(e) => onCustomFromChange(e.target.value)}
            className="h-8 w-[160px] text-xs"
          />
          <span className="text-muted-foreground text-xs">至</span>
          <Input
            aria-label="结束时间"
            type="datetime-local"
            value={customTo}
            onChange={(e) => onCustomToChange(e.target.value)}
            className="h-8 w-[160px] text-xs"
          />
        </div>
      )}
    </div>
  )
}

function ProviderBreakdown({
  providers,
}: {
  providers: DashboardStats['providerHealth']
}) {
  const activeProviders = providers.filter((provider) => provider.status === 'healthy' || provider.status === 'degraded')

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-3">
        <div>
          <CardTitle className="text-base">按平台拆分</CardTitle>
          <p className="mt-1 text-xs text-muted-foreground">请求 · Token · 费用 · 健康状态</p>
        </div>
        <span className="rounded-full bg-muted px-2.5 py-1 text-xs text-muted-foreground">
          {activeProviders.length} 个可用
        </span>
      </CardHeader>
      <CardContent className="pt-0">
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {providers.slice(0, 6).map((provider) => (
            <div key={provider.providerId} className="rounded-lg border bg-muted/20 p-3">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <p className="truncate text-sm font-semibold">{provider.displayName}</p>
                  <p className="mt-1 text-xs text-muted-foreground">{provider.providerId}</p>
                </div>
                <span
                  className={cn(
                    'rounded-full px-2 py-0.5 text-xs font-medium',
                    provider.status === 'healthy' && 'bg-emerald-500/10 text-emerald-600',
                    provider.status === 'degraded' && 'bg-amber-500/10 text-amber-600',
                    provider.status === 'cooldown' && 'bg-violet-500/10 text-violet-600',
                    provider.status === 'down' && 'bg-rose-500/10 text-rose-600',
                  )}
                >
                  {statusText(provider.status)}
                </span>
              </div>
              <div className="mt-4 grid grid-cols-3 gap-3 text-xs">
                <div>
                  <p className="text-muted-foreground">请求</p>
                  <p className="mt-1 font-mono font-semibold">{formatNumber(provider.requestsTotal)}</p>
                </div>
                <div>
                  <p className="text-muted-foreground">Token</p>
                  <p className="mt-1 font-mono font-semibold">{formatNumber(providerTokens(provider))}</p>
                </div>
                <div>
                  <p className="text-muted-foreground">费用</p>
                  <p className="mt-1 font-mono font-semibold">{formatUsd(provider.costEstimateUsdTotal || 0, 4)}</p>
                </div>
              </div>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  )
}

function ModelDistributionCard({
  rows,
  pieData,
}: {
  rows: ModelUsageRow[]
  pieData: Array<{ name: string; value: number }>
}) {
  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <CardTitle className="text-base">模型分布</CardTitle>
      </CardHeader>
      <CardContent className="pt-0">
        <div className="grid gap-4 xl:grid-cols-[240px_1fr]">
          <div className="flex min-h-[220px] items-center justify-center">
            {pieData.length > 0 ? (
              <ResponsiveContainer width="100%" height={220}>
                <PieChart>
                  <Pie
                    data={pieData}
                    cx="50%"
                    cy="50%"
                    innerRadius={54}
                    outerRadius={86}
                    paddingAngle={1.2}
                    minAngle={pieData.length > 1 ? 4 : 0}
                    cornerRadius={2}
                    dataKey="value"
                    stroke="var(--card)"
                    strokeWidth={2}
                  >
                    {pieData.map((_, idx) => (
                      <Cell key={idx} fill={PIE_COLORS[idx % PIE_COLORS.length]} />
                    ))}
                  </Pie>
                  <Tooltip {...TOOLTIP_STYLE} />
                </PieChart>
              </ResponsiveContainer>
            ) : (
              <div className="flex h-[220px] items-center justify-center text-sm text-muted-foreground">
                暂无数据
              </div>
            )}
          </div>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>模型</TableHead>
                <TableHead className="text-right">请求</TableHead>
                <TableHead className="text-right">Token</TableHead>
                <TableHead className="text-right">费用</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row, idx) => (
                <TableRow key={`${row.provider}:${row.model}`}>
                  <TableCell>
                    <div className="flex min-w-0 items-start gap-2">
                      <span
                        className="mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full ring-2 ring-background"
                        style={{ backgroundColor: PIE_COLORS[idx % PIE_COLORS.length] }}
                      />
                      <div className="min-w-0">
                        <div className="max-w-[220px] truncate font-mono text-xs font-medium">{row.model}</div>
                        <div className="text-xs text-muted-foreground">{row.provider}</div>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell className="text-right font-mono text-sm">{formatNumber(row.requests)}</TableCell>
                  <TableCell className="text-right font-mono text-sm">{formatNumber(row.tokens)}</TableCell>
                  <TableCell className="text-right font-mono text-sm">{formatUsd(row.cost, 4)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  )
}

function TokenTrendCard({ data }: { data: TokenTrendPoint[] }) {
  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Token 使用趋势</CardTitle>
      </CardHeader>
      <CardContent className="pt-0">
        <ResponsiveContainer width="100%" height={280}>
          <ComposedChart data={data}>
            <CartesianGrid strokeDasharray="3 3" className="stroke-muted" vertical={false} />
            <XAxis dataKey="time" tick={{ fontSize: 11 }} tickLine={false} axisLine={false} />
            <YAxis yAxisId="tokens" tick={{ fontSize: 11 }} tickLine={false} axisLine={false} width={54} />
            <YAxis yAxisId="rate" orientation="right" domain={[0, 100]} tick={{ fontSize: 11 }} tickLine={false} axisLine={false} width={42} />
            <Tooltip {...TOOLTIP_STYLE} />
            <Legend iconType="circle" iconSize={8} wrapperStyle={{ fontSize: 12 }} />
            <Area yAxisId="tokens" type="monotone" dataKey="input" name="Input" stroke="hsl(217 91% 60%)" fill="hsl(217 91% 60% / 0.16)" dot={false} />
            <Area yAxisId="tokens" type="monotone" dataKey="output" name="Output" stroke="hsl(142 71% 45%)" fill="hsl(142 71% 45% / 0.12)" dot={false} />
            <Area yAxisId="tokens" type="monotone" dataKey="cacheWrite" name="Cache Creation" stroke="hsl(38 92% 50%)" fill="hsl(38 92% 50% / 0.12)" dot={false} />
            <Area yAxisId="tokens" type="monotone" dataKey="cacheRead" name="Cache Read" stroke="hsl(190 70% 50%)" fill="hsl(190 70% 50% / 0.12)" dot={false} />
            <Line yAxisId="rate" type="monotone" dataKey="cacheHitRate" name="Cache Hit Rate" stroke="hsl(260 90% 65%)" strokeDasharray="5 4" strokeWidth={2} dot={false} />
          </ComposedChart>
        </ResponsiveContainer>
      </CardContent>
    </Card>
  )
}

function RecentUsageCard({ logs }: { logs: RequestLog[] }) {
  return (
    <Card className="h-full">
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
        <CardTitle className="text-base">最近使用</CardTitle>
        <span className="rounded-full bg-muted px-2.5 py-1 text-xs text-muted-foreground">近 {logs.length} 条</span>
      </CardHeader>
      <CardContent className="space-y-3 pt-0">
        {logs.length === 0 ? (
          <div className="flex h-40 items-center justify-center text-sm text-muted-foreground">暂无请求记录</div>
        ) : (
          logs.slice(0, 5).map((log) => (
            <div key={log.id} className="flex items-center justify-between gap-4 rounded-lg bg-muted/35 p-3">
              <div className="flex min-w-0 items-center gap-3">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-emerald-500/10 text-emerald-600">
                  <Box className="h-4 w-4" />
                </div>
                <div className="min-w-0">
                  <p className="truncate font-mono text-sm font-medium">{log.resolvedModel || log.model}</p>
                  <p className="text-xs text-muted-foreground">{formatRelativeTime(log.timestamp)}</p>
                </div>
              </div>
              <div className="shrink-0 text-right">
                <p className="font-mono text-sm font-semibold text-emerald-600">{formatUsd(log.costEstimate || 0, 4)}</p>
                <p className="text-xs text-muted-foreground">{formatNumber(log.totalTokens || 0)} tokens</p>
              </div>
            </div>
          ))
        )}
        <Button asChild variant="ghost" className="w-full text-primary">
          <Link to="/logs">
            查看全部
            <ArrowRight className="h-4 w-4" />
          </Link>
        </Button>
      </CardContent>
    </Card>
  )
}

function QuickActionsCard() {
  const actions = [
    { to: '/api-keys', icon: KeyRound, title: '创建 API 密钥', desc: '生成新的 API 密钥' },
    { to: '/logs', icon: ScrollText, title: '查看使用记录', desc: '排查错误、成本和延迟' },
    { to: '/models', icon: Layers, title: '管理模型路由', desc: '检查供应商和别名' },
  ]

  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <CardTitle className="text-base">快捷操作</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 pt-0">
        {actions.map((action) => {
          const Icon = action.icon
          return (
            <Link
              key={action.to}
              to={action.to}
              className="flex items-center justify-between gap-4 rounded-lg bg-muted/35 p-4 transition-colors hover:bg-muted"
            >
              <div className="flex min-w-0 items-center gap-3">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                  <Icon className="h-5 w-5" />
                </div>
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">{action.title}</p>
                  <p className="text-xs text-muted-foreground">{action.desc}</p>
                </div>
              </div>
              <ArrowRight className="h-4 w-4 shrink-0 text-muted-foreground" />
            </Link>
          )
        })}
      </CardContent>
    </Card>
  )
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export function DashboardPage() {
  const [trendRange, setTrendRange] = useState<DashboardRange>('1d')
  const [customFrom, setCustomFrom] = useState(() => toDateTimeLocal(Date.now() - DAY_MS))
  const [customTo, setCustomTo] = useState(() => toDateTimeLocal(Date.now()))

  const dashboardParams = useMemo(
    () => dashboardTrendParams(trendRange, customFrom, customTo),
    [customFrom, customTo, trendRange],
  )
  const { data: stats, isLoading, error, refetch } = useDashboard(dashboardParams)
  const logFilters = useMemo(
    () => currentLogFilters(trendRange, customFrom, customTo),
    [customFrom, customTo, trendRange],
  )
  const { data: logsData } = useLogs(logFilters, 1, 500)

  // ---- Derived / memoized data ----

  const requestTrend = useMemo(
    () => (stats ? computeTrend(stats.requestTimeSeries) : 0),
    [stats],
  )

  const chartData = useMemo(() => {
    if (!stats) return []
    return stats.requestTimeSeries.map((p, i) => ({
      time: formatChartTime(p.timestamp, stats.trendRange?.bucketMs),
      requests: p.value,
      errors: stats.errorTimeSeries[i]?.value ?? 0,
    }))
  }, [stats])

  const sparklineRequests = useMemo(
    () => stats?.requestTimeSeries.map((p) => p.value) ?? [],
    [stats],
  )

  const successSparkline = useMemo(() => {
    if (!stats) return []
    return stats.requestTimeSeries.map((p, i) => {
      const err = stats.errorTimeSeries[i]?.value ?? 0
      return p.value === 0 ? 100 : Math.round(((p.value - err) / p.value) * 100)
    })
  }, [stats])

  const modelUsageRows = useMemo(
    () => buildModelUsageRows(logsData?.logs ?? [], stats?.topModels ?? []),
    [logsData?.logs, stats?.topModels],
  )

  const modelPieData = useMemo(
    () => modelUsageRows.map((row) => ({
      name: row.model,
      value: row.tokens || row.requests,
    })),
    [modelUsageRows],
  )

  const tokenTrendData = useMemo(() => {
    if (!stats) return []
    const fallbackStartMs = Number(dateTimeLocalToMillis(customFrom))
    const fallbackEndMs = Number(dateTimeLocalToMillis(customTo))
    const startMs = Number(stats.trendRange?.from ?? fallbackStartMs)
    const endMs = Number(stats.trendRange?.to ?? fallbackEndMs)
    return buildTokenTrend(logsData?.logs ?? [], startMs, endMs, stats.trendRange?.bucketMs ?? 60 * 60 * 1000)
  }, [customFrom, customTo, logsData?.logs, stats])

  // ---- Loading / Error ----

  if (error && !stats) {
    return (
      <ErrorState
        message="仪表盘数据加载失败，请检查网络后重试。"
        onRetry={() => refetch()}
      />
    )
  }

  if (isLoading || !stats) {
    return <LoadingPage />
  }

  const totalTokens =
    (stats.todayInputTokens ?? 0) +
    (stats.todayOutputTokens ?? 0) +
    (stats.todayCacheWriteTokens ?? 0) +
    (stats.todayCacheReadTokens ?? 0)
  const summary = logsData?.summary
  const summaryInputTokens = summary?.totalInputTokens ?? stats.todayInputTokens ?? 0
  const summaryOutputTokens = summary?.totalOutputTokens ?? stats.todayOutputTokens ?? 0
  const summaryCacheWriteTokens = summary?.totalCacheWriteTokens ?? stats.todayCacheWriteTokens ?? 0
  const summaryCacheReadTokens = summary?.totalCacheReadTokens ?? stats.todayCacheReadTokens ?? 0
  const summaryTokens = summary?.totalTokens ?? totalTokens
  const summaryCost = summary?.totalCostEstimate ?? stats.todayCostEstimate ?? 0
  const billedInputTokens = summaryInputTokens + summaryCacheWriteTokens + summaryCacheReadTokens
  const cacheHitRate = billedInputTokens > 0 ? (summaryCacheReadTokens / billedInputTokens) * 100 : 0

  const todayTokenDesc = tokenBreakdownDescription(
    stats.todayInputTokens ?? 0,
    stats.todayOutputTokens ?? 0,
    stats.todayCacheWriteTokens ?? 0,
    stats.todayCacheReadTokens ?? 0,
  )
  const summaryTokenDesc = tokenBreakdownDescription(
    summaryInputTokens,
    summaryOutputTokens,
    summaryCacheWriteTokens,
    summaryCacheReadTokens,
  )

  const rangeName = rangeLabel(stats.trendRange?.range ?? trendRange)

  // ---- Render ----

  return (
    <div className="space-y-6">
      {/* ---------------------------------------------------------------- */}
      {/* Header                                                           */}
      {/* ---------------------------------------------------------------- */}
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">仪表盘</h1>
          <p className="text-sm text-muted-foreground">
            实时监控 API 调用、模型使用与系统健康状态
          </p>
        </div>
        <RangeSelector
          value={trendRange}
          onChange={setTrendRange}
          customFrom={customFrom}
          customTo={customTo}
          onCustomFromChange={setCustomFrom}
          onCustomToChange={setCustomTo}
        />
      </div>

      {/* ---------------------------------------------------------------- */}
      {/* Metric Cards                                                     */}
      {/* ---------------------------------------------------------------- */}
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <MetricCard
          title="API 密钥"
          value={`${formatNumber(stats.apiKeysActive ?? 0)}/${formatNumber(stats.apiKeysTotal ?? 0)}`}
          icon={KeyRound}
          description={`${formatNumber(stats.apiKeysActive ?? 0)} 启用`}
        />
        <MetricCard
          title="今日请求量"
          value={formatNumber(stats.todayRequests ?? stats.totalRequests)}
          icon={Activity}
          sparkline={sparklineRequests}
          trend={{ value: requestTrend, label: `总计 ${formatNumber(stats.totalRequests)}` }}
        />
        <MetricCard
          title="今日消耗"
          value={formatUsd(stats.todayCostEstimate ?? 0, 4)}
          icon={WalletCards}
          sparkline={sparklineRequests}
          description={`当前范围 ${formatUsd(summaryCost, 4)}`}
        />
        <MetricCard
          title="今日 Token"
          value={formatNumber(totalTokens)}
          icon={Clock}
          sparkline={sparklineRequests}
          description={todayTokenDesc}
        />
        <MetricCard
          title="累计 Token"
          value={formatNumber(summaryTokens)}
          icon={Database}
          sparkline={sparklineRequests}
          description={`当前范围 · ${summaryTokenDesc}`}
        />
        <MetricCard
          title="成功率"
          value={`${stats.successRate.toFixed(1)}%`}
          icon={Gauge}
          sparkline={successSparkline}
          trend={{
            value: Math.round(stats.successRate) >= 99 ? 0 : -1,
            label: '目标 99%',
          }}
        />
        <MetricCard
          title="性能指标"
          value={`${formatRate(summary?.rpm ?? 0)} RPM`}
          icon={Zap}
          sparkline={sparklineRequests}
          description={`${formatRate(summary?.tpm ?? 0)} TPM`}
        />
        <MetricCard
          title="平均响应"
          value={formatLatency(stats.avgLatencyMs)}
          icon={TrendingUp}
          sparkline={sparklineRequests}
          description={`缓存命中 ${formatPercentValue(cacheHitRate)}`}
        />
      </div>

      {/* ---------------------------------------------------------------- */}
      {/* Full-width request volume chart                                  */}
      {/* ---------------------------------------------------------------- */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">
            请求量趋势（{rangeName}）
          </CardTitle>
        </CardHeader>
        <CardContent>
          <ResponsiveContainer width="100%" height={320}>
            <AreaChart data={chartData}>
              <defs>
                <linearGradient id="requestGradient" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--primary)" stopOpacity={0.3} />
                  <stop offset="100%" stopColor="var(--primary)" stopOpacity={0.02} />
                </linearGradient>
                <linearGradient id="errorGradient" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--destructive)" stopOpacity={0.25} />
                  <stop offset="100%" stopColor="var(--destructive)" stopOpacity={0.02} />
                </linearGradient>
              </defs>
              <CartesianGrid strokeDasharray="3 3" className="stroke-muted" vertical={false} />
              <XAxis
                dataKey="time"
                className="text-xs"
                tick={{ fontSize: 11 }}
                tickLine={false}
                axisLine={false}
              />
              <YAxis
                className="text-xs"
                tick={{ fontSize: 11 }}
                tickLine={false}
                axisLine={false}
                width={50}
              />
              <Tooltip {...TOOLTIP_STYLE} />
              <Area
                type="monotone"
                dataKey="requests"
                name="请求"
                stroke="var(--primary)"
                strokeWidth={2}
                fill="url(#requestGradient)"
                dot={false}
                activeDot={{ r: 4, strokeWidth: 2 }}
              />
              <Area
                type="monotone"
                dataKey="errors"
                name="错误"
                stroke="var(--destructive)"
                strokeWidth={1.5}
                fill="url(#errorGradient)"
                dot={false}
                activeDot={{ r: 3, strokeWidth: 2 }}
              />
            </AreaChart>
          </ResponsiveContainer>
        </CardContent>
      </Card>

      <ProviderBreakdown providers={stats.providerHealth} />

      <div className="grid gap-4 lg:grid-cols-2">
        <ModelDistributionCard rows={modelUsageRows} pieData={modelPieData} />
        <TokenTrendCard data={tokenTrendData} />
      </div>

      <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
        <RecentUsageCard logs={logsData?.logs ?? []} />
        <QuickActionsCard />
      </div>
    </div>
  )
}
