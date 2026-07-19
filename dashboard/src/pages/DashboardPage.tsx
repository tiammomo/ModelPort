import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { useDashboard, useLogs } from '@/hooks'
import { useAuthStore } from '@/stores'
import { MetricCard } from '@/components/shared/MetricCard'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { ErrorState } from '@/components/shared/ErrorState'
import { EmptyState } from '@/components/shared/EmptyState'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { cn, formatNumber, formatRelativeTime, formatLatency } from '@/lib/utils'
import {
  DAY_MS,
  TREND_RANGES,
  compactModelUsageRows,
  computeTrend,
  customRangeError,
  dashboardTrendParams,
  formatChartTime,
  formatPercentValue,
  formatRate,
  formatUsd,
  providerTokens,
  rangeLabel,
  statusText,
  toDateTimeLocal,
  tokenBreakdownDescription,
  type ModelUsageRow,
  type TokenTrendPoint,
} from '@/features/dashboard/dashboard-data'
import type { DashboardRange } from '@/services/dashboard.service'
import type { DashboardStats, RequestLog } from '@/types'
import {
  Activity,
  ArrowRight,
  Box,
  CheckCircle2,
  Database,
  Gauge,
  KeyRound,
  Layers,
  ScrollText,
  TriangleAlert,
  WalletCards,
  Wrench,
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
// Sub-components
// ---------------------------------------------------------------------------

function RangeSelector({
  value,
  onChange,
  customFrom,
  customTo,
  onCustomFromChange,
  onCustomToChange,
  error,
}: {
  value: DashboardRange
  onChange: (r: DashboardRange) => void
  customFrom: string
  customTo: string
  onCustomFromChange: (v: string) => void
  onCustomToChange: (v: string) => void
  error?: string | null
}) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2" role="group" aria-label="仪表盘数据范围">
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
        <div className="flex w-full flex-col gap-2 pt-1 sm:ml-1 sm:w-auto sm:flex-row sm:items-center sm:pt-0">
          <Input
            aria-label="开始时间"
            type="datetime-local"
            value={customFrom}
            onChange={(e) => onCustomFromChange(e.target.value)}
            aria-invalid={!!error}
            className="h-8 w-full text-xs sm:w-[160px]"
          />
          <span className="hidden text-xs text-muted-foreground sm:inline">至</span>
          <Input
            aria-label="结束时间"
            type="datetime-local"
            value={customTo}
            onChange={(e) => onCustomToChange(e.target.value)}
            aria-invalid={!!error}
            className="h-8 w-full text-xs sm:w-[160px]"
          />
        </div>
      )}
      </div>
      {error && <p role="alert" className="max-w-lg text-xs text-destructive">{error}</p>}
    </div>
  )
}

function providerStatusClass(status: DashboardStats['providerHealth'][number]['status']): string {
  if (status === 'healthy') return 'bg-emerald-500'
  if (status === 'degraded') return 'bg-amber-500'
  if (status === 'cooldown') return 'bg-violet-500'
  return 'bg-rose-500'
}

function GatewayOperationsPanel({
  stats,
  logs,
  primaryModel,
}: {
  stats: DashboardStats
  logs: RequestLog[]
  primaryModel: string
}) {
  const isAdmin = useAuthStore((state) => state.currentUser?.role === 'admin')
  const primaryProvider = stats.providerHealth.find((provider) => provider.status === 'healthy')
    ?? stats.providerHealth[0]
  const streamCount = logs.filter((log) => log.stream === 'stream').length
  const errorCount = logs.filter((log) => log.status !== 'success').length
  const healthyProviders = stats.providerHealth.filter((provider) => provider.status === 'healthy').length
  const providerIssues = stats.providerHealth.filter(
    (provider) => provider.status !== 'healthy' || provider.rechargeRequired,
  )
  const allProvidersHealthy = stats.providerHealth.length > 0 && providerIssues.length === 0

  return (
    <div className="grid gap-4 xl:grid-cols-[1.35fr_1fr]">
      <Card className="overflow-hidden">
        <CardHeader className="border-b pb-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <CardTitle className="flex items-center gap-2 text-base">
                {allProvidersHealthy
                  ? <CheckCircle2 className="h-4 w-4 text-emerald-600" />
                  : <TriangleAlert className="h-4 w-4 text-amber-600" />}
                运行状态
              </CardTitle>
              <p className="mt-1 text-xs text-muted-foreground">
                基于 Provider 健康记录定位异常请求和上游账号问题。
              </p>
            </div>
            <Badge variant={allProvidersHealthy ? 'success' : providerIssues.length > 0 ? 'warning' : 'secondary'}>
              {allProvidersHealthy ? 'Provider 健康' : providerIssues.length > 0 ? `${providerIssues.length} 项需关注` : '等待 Provider'}
            </Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-4 p-4">
          <div className="grid overflow-hidden border-y bg-card lg:grid-cols-3 lg:divide-x">
            <div className="px-3 py-3">
              <p className="text-xs text-muted-foreground">可用 Provider</p>
              <p className="mt-1 font-mono text-lg font-semibold">{healthyProviders} / {stats.providerHealth.length}</p>
              <p className="mt-1 text-xs text-muted-foreground">基于当前运行健康记录</p>
            </div>
            <div className="border-t px-3 py-3 lg:border-t-0">
              <p className="text-xs text-muted-foreground">最近异常</p>
              <p className="mt-1 font-mono text-lg font-semibold">{formatNumber(errorCount)}</p>
              <p className="mt-1 text-xs text-muted-foreground">最近 {logs.length} 条记录；不等同于协议错误</p>
            </div>
            <div className="border-t px-3 py-3 lg:border-t-0">
              <p className="text-xs text-muted-foreground">当前首选路由</p>
              <p className="mt-1 truncate font-mono text-sm font-semibold" title={primaryModel}>{primaryModel || '未配置'}</p>
              <p className="mt-1 truncate text-xs text-muted-foreground">{primaryProvider?.displayName || '等待可用 Provider'}</p>
            </div>
          </div>

          {providerIssues.length > 0 ? (
            <div className="space-y-2 rounded-lg border border-amber-200 bg-amber-50/70 p-3 dark:border-amber-900 dark:bg-amber-950/20">
              {providerIssues.slice(0, 3).map((provider) => (
                <div key={provider.providerId} className="flex items-start justify-between gap-3 text-sm">
                  <div className="min-w-0">
                    <p className="truncate font-medium">{provider.displayName}</p>
                    <p className="text-xs text-muted-foreground">
                      {provider.rechargeRequired
                        ? '上游账号余额或额度需要处理'
                        : provider.status === 'degraded'
                          ? `调用成功率 ${provider.successRate.toFixed(1)}%，存在失败或客户端取消`
                          : provider.status === 'cooldown'
                            ? '路由暂时冷却，等待自动恢复'
                            : 'Provider 当前不可用，请检查配置或上游服务'}
                    </p>
                  </div>
                  {isAdmin && <Button asChild variant="ghost" size="sm"><Link to="/models">查看详情</Link></Button>}
                </div>
              ))}
            </div>
          ) : (
            <div className="flex items-start gap-2 rounded-lg border border-emerald-200 bg-emerald-50/70 p-3 text-sm dark:border-emerald-900 dark:bg-emerald-950/20">
              <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600" />
              <span>{stats.providerHealth.length > 0 ? '当前没有需要处理的 Provider 异常。' : '尚无 Provider 健康记录，请先完成上游接入。'}</span>
            </div>
          )}

          <div className="flex flex-wrap gap-2">
            <Button asChild variant="outline" size="sm"><Link to="/logs">查看请求日志</Link></Button>
            {isAdmin && <Button asChild variant="outline" size="sm"><Link to="/models">管理模型与渠道</Link></Button>}
            {streamCount > 0 && <Badge variant="outline" className="self-center">最近流式 {streamCount}</Badge>}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="border-b pb-4">
          <CardTitle className="flex items-center gap-2 text-base">
            <Wrench className="h-4 w-4 text-primary" />
            上游渠道状态
          </CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          <div className="divide-y">
            {stats.providerHealth.slice(0, 5).map((provider) => (
              <div key={provider.providerId} className="flex items-center justify-between gap-3 px-4 py-3">
                <div className="min-w-0">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className={cn('h-2 w-2 shrink-0 rounded-full', providerStatusClass(provider.status))} />
                    <p className="truncate text-sm font-medium">{provider.displayName}</p>
                    {provider.rechargeRequired && (
                      <Badge variant="warning" className="shrink-0 text-[10px]">
                        等待充值
                      </Badge>
                    )}
                  </div>
                  <p className="mt-1 truncate font-mono text-xs text-muted-foreground">{provider.providerId}</p>
                </div>
                <div className="shrink-0 text-right">
                  <p className="font-mono text-sm font-semibold">{provider.successRate.toFixed(1)}%</p>
                  <p className="text-xs text-muted-foreground">{formatLatency(provider.avgLatencyMs)}</p>
                </div>
              </div>
            ))}
            {stats.providerHealth.length === 0 && (
              <div className="px-4 py-8 text-center text-sm text-muted-foreground">
                暂无 Provider
              </div>
            )}
          </div>
        </CardContent>
      </Card>
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
      <CardContent className="p-0">
        {providers.length === 0 ? (
          <EmptyState
            icon={Database}
            title="暂无 Provider 运行数据"
            description="配置上游并完成请求后，这里会展示渠道健康、Token 与费用。"
            className="py-8"
          />
        ) : (
          <div className="grid gap-px border-y bg-border md:grid-cols-2 xl:grid-cols-3">
            {providers.slice(0, 6).map((provider) => (
              <div key={provider.providerId} className="min-w-0 bg-card px-4 py-4">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex min-w-0 flex-wrap items-center gap-2">
                      <p className="truncate text-sm font-semibold">{provider.displayName}</p>
                      {provider.rechargeRequired && (
                        <span className="rounded-full bg-amber-500 px-2 py-0.5 text-xs font-medium text-white">
                          等待充值
                        </span>
                      )}
                    </div>
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
        )}
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
        {rows.length === 0 ? (
          <EmptyState
            icon={Layers}
            title="暂无模型用量"
            description="完成首个模型请求后，这里会按真实保留用量展示模型分布。"
            className="min-h-[260px] py-8"
          />
        ) : (
          <div className="grid gap-4 2xl:grid-cols-[240px_1fr]">
            <div className="flex min-h-[220px] items-center justify-center">
              {pieData.length > 0 && (
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
        )}
      </CardContent>
    </Card>
  )
}

function TokenTrendCard({ data }: { data: TokenTrendPoint[] }) {
  const hasUsage = data.some((point) => point.input + point.output + point.cacheWrite + point.cacheRead > 0)
  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Token 使用趋势</CardTitle>
      </CardHeader>
      <CardContent className="pt-0">
        {hasUsage ? (
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
        ) : (
          <EmptyState
            icon={Database}
            title="暂无 Token 趋势"
            description="图表只使用后端返回的真实用量，不会用零值推断调用情况。"
            className="min-h-[280px] py-8"
          />
        )}
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
      <CardContent className="p-0">
        {logs.length === 0 ? (
          <div className="flex h-40 items-center justify-center text-sm text-muted-foreground">暂无请求记录</div>
        ) : (
          <div className="divide-y border-y">
            {logs.slice(0, 5).map((log) => (
              <div key={log.id} className="flex items-center justify-between gap-4 px-4 py-3 transition-colors hover:bg-muted/25">
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
            ))}
          </div>
        )}
        <Button asChild variant="ghost" className="h-11 w-full rounded-none text-primary">
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
  const isAdmin = useAuthStore((state) => state.currentUser?.role === 'admin')
  const actions = [
    { to: '/api-keys', icon: KeyRound, title: '创建 API 密钥', desc: '生成新的 API 密钥' },
    { to: '/logs', icon: ScrollText, title: '查看使用记录', desc: '排查错误、成本和延迟' },
    { to: '/models', icon: Layers, title: '管理模型路由', desc: '检查供应商和别名' },
  ].filter((action) => isAdmin || action.to === '/logs')

  return (
    <Card className="h-full">
      <CardHeader className="pb-3">
        <CardTitle className="text-base">快捷操作</CardTitle>
      </CardHeader>
      <CardContent className="divide-y border-y p-0">
        {actions.map((action) => {
          const Icon = action.icon
          return (
            <Link
              key={action.to}
              to={action.to}
              className="flex items-center justify-between gap-4 px-4 py-3.5 transition-colors hover:bg-muted/35"
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

function OperationalStat({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <div className="min-w-0 px-4 py-3 sm:px-5 sm:py-4">
      <p className="text-xs font-medium text-muted-foreground">{label}</p>
      <p className="mt-1 truncate font-mono text-lg font-semibold" title={value}>{value}</p>
      <p className="mt-1 truncate text-xs text-muted-foreground" title={detail}>{detail}</p>
    </div>
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
  const rangeError = useMemo(
    () => customRangeError(trendRange, customFrom, customTo),
    [customFrom, customTo, trendRange],
  )
  const { data: stats, isLoading, isFetching, error, refetch, dataUpdatedAt } = useDashboard(dashboardParams)
  const { data: logsData } = useLogs(undefined, 1, 5)

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
    () => compactModelUsageRows(stats?.modelUsage ?? []),
    [stats?.modelUsage],
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
    return stats.tokenTimeSeries.map((point) => ({
      time: formatChartTime(point.timestamp, stats.trendRange?.bucketMs),
      input: point.inputTokens,
      output: point.outputTokens,
      cacheWrite: point.cacheWriteTokens,
      cacheRead: point.cacheReadTokens,
      cacheHitRate: point.cacheHitRate,
    }))
  }, [stats])

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

  const summary = stats.rangeSummary
  const summaryInputTokens = summary?.totalInputTokens ?? stats.todayInputTokens ?? 0
  const summaryOutputTokens = summary?.totalOutputTokens ?? stats.todayOutputTokens ?? 0
  const summaryCacheWriteTokens = summary?.totalCacheWriteTokens ?? stats.todayCacheWriteTokens ?? 0
  const summaryCacheReadTokens = summary?.totalCacheReadTokens ?? stats.todayCacheReadTokens ?? 0
  const summaryTokens = summary?.totalTokens
    ?? summaryInputTokens + summaryOutputTokens + summaryCacheWriteTokens + summaryCacheReadTokens
  const summaryCost = summary?.totalCostEstimate ?? stats.todayCostEstimate ?? 0
  const rangeSuccessRate = summary.totalRequests > 0
    ? (summary.successRequests / summary.totalRequests) * 100
    : null
  const billedInputTokens = summaryInputTokens + summaryCacheWriteTokens + summaryCacheReadTokens
  const cacheHitRate = billedInputTokens > 0 ? (summaryCacheReadTokens / billedInputTokens) * 100 : 0

  const summaryTokenDesc = tokenBreakdownDescription(
    summaryInputTokens,
    summaryOutputTokens,
    summaryCacheWriteTokens,
    summaryCacheReadTokens,
  )

  const rangeName = rangeLabel(stats.trendRange?.range ?? trendRange)
  const primaryModel = modelUsageRows[0]?.model ?? stats.topModels[0]?.model ?? '默认路由'
  const updatedAt = dataUpdatedAt
    ? new Date(dataUpdatedAt).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })
    : '—'
  const dataSourceLabel = stats.rangeDataSource === 'persisted-usage'
    ? '持久化用量'
    : stats.rangeDataSource === 'process-metrics-estimate'
      ? '进程指标估算'
      : '暂无数据'
  const hasRequestTrend = chartData.some((point) => point.requests > 0 || point.errors > 0)

  // ---- Render ----

  return (
    <div className="space-y-5">
      {/* ---------------------------------------------------------------- */}
      {/* Header                                                           */}
      {/* ---------------------------------------------------------------- */}
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">仪表盘</h1>
          <p className="text-sm text-muted-foreground">
            先判断网关是否可用，再定位异常、成本与路由变化。
          </p>
          <p className="mt-1 text-xs text-muted-foreground" aria-live="polite">
            {isFetching ? '正在刷新…' : `更新于 ${updatedAt}`} · 数据源：{dataSourceLabel}
          </p>
        </div>
        <RangeSelector
          value={trendRange}
          onChange={setTrendRange}
          customFrom={customFrom}
          customTo={customTo}
          onCustomFromChange={setCustomFrom}
          onCustomToChange={setCustomTo}
          error={rangeError}
        />
      </div>

      {(stats.rangeDataEstimated || stats.rangeDataAtRetentionLimit) && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          {stats.rangeDataEstimated && (
            <Badge variant="outline">进程指标估算</Badge>
          )}
          {stats.rangeDataEstimated && (
            <span>当前没有持久化用量记录，范围统计来自本次进程启动后的指标。</span>
          )}
          {stats.rangeDataAtRetentionLimit && (
            <>
              <Badge variant="secondary">已达保留上限</Badge>
              <span>更早的用量记录可能已被轮转，长时间范围仅覆盖当前保留窗口。</span>
            </>
          )}
        </div>
      )}

      {/* ---------------------------------------------------------------- */}
      {/* Metric Cards                                                     */}
      {/* ---------------------------------------------------------------- */}
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <MetricCard
          title={`${rangeName}请求`}
          value={formatNumber(summary.totalRequests)}
          icon={Activity}
          sparkline={sparklineRequests}
          trend={summary.totalRequests > 0 ? { value: requestTrend, label: '较范围前半段' } : undefined}
          description={summary.totalRequests === 0 ? '当前范围暂无调用' : undefined}
        />
        <MetricCard
          title={`${rangeName}成功率`}
          value={rangeSuccessRate === null ? '—' : `${rangeSuccessRate.toFixed(1)}%`}
          icon={Gauge}
          sparkline={successSparkline}
          description={rangeSuccessRate === null
            ? '没有请求，不能推断成功率'
            : `${formatNumber(summary.totalRequests - summary.successRequests)} 次异常`}
        />
        <MetricCard
          title={`${rangeName}估算费用`}
          value={formatUsd(summaryCost, 4)}
          icon={WalletCards}
          sparkline={sparklineRequests}
          description="运维估算，不等同于 Provider 账单"
        />
        <MetricCard
          title="进程平均延迟"
          value={formatLatency(stats.avgLatencyMs)}
          icon={Zap}
          sparkline={sparklineRequests}
          description="进程累计，不受范围筛选影响"
        />
      </div>

      <section aria-label="补充运行指标" className="overflow-hidden border-y bg-card">
        <div className="grid grid-cols-2 divide-x divide-y lg:grid-cols-4 lg:divide-y-0">
          <OperationalStat label={`${rangeName} Token`} value={formatNumber(summaryTokens)} detail={summaryTokenDesc} />
          <OperationalStat label="范围吞吐" value={`${formatRate(summary?.rpm ?? 0)} RPM`} detail={`${formatRate(summary?.tpm ?? 0)} TPM`} />
          <OperationalStat label="客户端密钥" value={`${formatNumber(stats.apiKeysActive ?? 0)} / ${formatNumber(stats.apiKeysTotal ?? 0)}`} detail="启用 / 总数" />
          <OperationalStat label="缓存命中" value={formatPercentValue(cacheHitRate)} detail={`读 ${formatNumber(summaryCacheReadTokens)} Token`} />
        </div>
      </section>

      <GatewayOperationsPanel
        stats={stats}
        logs={logsData?.logs ?? []}
        primaryModel={primaryModel}
      />

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
          {hasRequestTrend ? (
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
          ) : (
            <EmptyState
              icon={Activity}
              title="当前范围暂无请求"
              description="发送首个请求后，这里会展示真实请求与错误趋势。"
              className="min-h-[320px] py-10"
            />
          )}
        </CardContent>
      </Card>

      <ProviderBreakdown providers={stats.providerHealth} />

      <div className="grid gap-4 xl:grid-cols-2">
        <ModelDistributionCard rows={modelUsageRows} pieData={modelPieData} />
        <TokenTrendCard data={tokenTrendData} />
      </div>

      <div className="grid gap-4 xl:grid-cols-[2fr_1fr]">
        <RecentUsageCard logs={logsData?.logs ?? []} />
        <QuickActionsCard />
      </div>
    </div>
  )
}
