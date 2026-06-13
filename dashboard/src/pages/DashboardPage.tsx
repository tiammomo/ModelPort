import { useMemo, useState } from 'react'
import { useDashboard } from '@/hooks'
import { MetricCard } from '@/components/shared/MetricCard'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { formatNumber, formatRelativeTime, parseDate } from '@/lib/utils'
import type { DashboardRange, DashboardStatsParams } from '@/services/dashboard.service'
import { Activity, Users, Server, Clock, TrendingUp, AlertCircle, Info } from 'lucide-react'
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, AreaChart, Area } from 'recharts'

const DAY_MS = 24 * 60 * 60 * 1000
const TREND_RANGES: Array<{ value: DashboardRange; label: string }> = [
  { value: '1d', label: '近1天' },
  { value: '3d', label: '近3天' },
  { value: '7d', label: '近7天' },
  { value: 'custom', label: '自定义' },
]

export function DashboardPage() {
  const [trendRange, setTrendRange] = useState<DashboardRange>('1d')
  const [customFrom, setCustomFrom] = useState(() => toDateTimeLocal(Date.now() - DAY_MS))
  const [customTo, setCustomTo] = useState(() => toDateTimeLocal(Date.now()))
  const dashboardParams = useMemo(
    () => dashboardTrendParams(trendRange, customFrom, customTo),
    [customFrom, customTo, trendRange],
  )
  const { data: stats, isLoading } = useDashboard(dashboardParams)

  if (isLoading || !stats) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  const chartData = stats.requestTimeSeries.map((p, i) => ({
    time: formatChartTime(p.timestamp, stats.trendRange?.bucketMs),
    requests: p.value,
    errors: stats.errorTimeSeries[i]?.value || 0,
  }))

  const severityIcons = {
    info: <Info className="h-4 w-4 text-blue-500" />,
    warning: <AlertCircle className="h-4 w-4 text-yellow-500" />,
    error: <AlertCircle className="h-4 w-4 text-red-500" />,
  }

  return (
    <div className="space-y-6">
      {/* Metric Cards */}
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <MetricCard
          title="今日请求量"
          value={formatNumber(stats.todayRequests ?? stats.totalRequests)}
          icon={Activity}
          trend={{ value: 12.5, label: '较昨日' }}
        />
        <MetricCard
          title="API Keys"
          value={stats.apiKeysActive ?? 0}
          icon={Users}
          description={`${stats.apiKeysTotal ?? 0} total`}
        />
        <MetricCard
          title="活跃提供商"
          value={`${stats.activeProviders} / ${stats.totalProviders}`}
          icon={Server}
          description="已配置 API Key"
        />
        <MetricCard
          title="今日 Tokens"
          value={formatNumber((stats.todayInputTokens ?? 0) + (stats.todayOutputTokens ?? 0) + (stats.todayCacheWriteTokens ?? 0) + (stats.todayCacheReadTokens ?? 0))}
          icon={Clock}
          description={`In ${formatNumber(stats.todayInputTokens ?? 0)} / Out ${formatNumber(stats.todayOutputTokens ?? 0)} / Cache ${formatNumber((stats.todayCacheWriteTokens ?? 0) + (stats.todayCacheReadTokens ?? 0))}`}
        />
      </div>

      <div className="flex flex-wrap items-center justify-end gap-2">
        {TREND_RANGES.map((option) => (
          <Button
            key={option.value}
            type="button"
            size="sm"
            variant={trendRange === option.value ? 'default' : 'outline'}
            onClick={() => setTrendRange(option.value)}
          >
            {option.label}
          </Button>
        ))}
        {trendRange === 'custom' && (
          <div className="flex flex-wrap items-center gap-2">
            <Input
              aria-label="开始时间"
              type="datetime-local"
              value={customFrom}
              onChange={(event) => setCustomFrom(event.target.value)}
              className="w-[170px]"
            />
            <Input
              aria-label="结束时间"
              type="datetime-local"
              value={customTo}
              onChange={(event) => setCustomTo(event.target.value)}
              className="w-[170px]"
            />
          </div>
        )}
      </div>

      {/* Charts */}
      <div className="grid gap-4 lg:grid-cols-7">
        <Card className="lg:col-span-4">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <TrendingUp className="h-4 w-4" />
              请求量趋势（24h）
            </CardTitle>
          </CardHeader>
          <CardContent>
            <ResponsiveContainer width="100%" height={250}>
              <LineChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
                <XAxis dataKey="time" className="text-xs" tick={{ fontSize: 11 }} />
                <YAxis className="text-xs" tick={{ fontSize: 11 }} />
                <Tooltip
                  contentStyle={{
                    backgroundColor: 'hsl(var(--card))',
                    border: '1px solid hsl(var(--border))',
                    borderRadius: '8px',
                    fontSize: '12px',
                  }}
                />
                <Line type="monotone" dataKey="requests" stroke="hsl(var(--primary))" strokeWidth={2} dot={false} />
              </LineChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>

        <Card className="lg:col-span-3">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <AlertCircle className="h-4 w-4" />
              错误率趋势（24h）
            </CardTitle>
          </CardHeader>
          <CardContent>
            <ResponsiveContainer width="100%" height={250}>
              <AreaChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" className="stroke-muted" />
                <XAxis dataKey="time" className="text-xs" tick={{ fontSize: 11 }} />
                <YAxis className="text-xs" tick={{ fontSize: 11 }} />
                <Tooltip
                  contentStyle={{
                    backgroundColor: 'hsl(var(--card))',
                    border: '1px solid hsl(var(--border))',
                    borderRadius: '8px',
                    fontSize: '12px',
                  }}
                />
                <Area type="monotone" dataKey="errors" stroke="hsl(var(--destructive))" fill="hsl(var(--destructive))" fillOpacity={0.2} />
              </AreaChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>
      </div>

      {/* Bottom section */}
      <div className="grid gap-4 lg:grid-cols-7">
        {/* Top Models */}
        <Card className="lg:col-span-4">
          <CardHeader>
            <CardTitle>热门模型</CardTitle>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>模型</TableHead>
                  <TableHead>提供商</TableHead>
                  <TableHead className="text-right">请求量</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {stats.topModels.map((model) => (
                  <TableRow key={model.model}>
                    <TableCell className="font-medium">{model.model}</TableCell>
                    <TableCell className="text-muted-foreground">{model.provider}</TableCell>
                    <TableCell className="text-right">{formatNumber(model.requests)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>

        {/* Recent Activity */}
        <Card className="lg:col-span-3">
          <CardHeader>
            <CardTitle>最近活动</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-4">
              {stats.recentActivity.map((activity) => (
                <div key={activity.id} className="flex items-start gap-3">
                  <div className="mt-0.5">{severityIcons[activity.severity]}</div>
                  <div className="flex-1 min-w-0">
                    <p className="text-sm">{activity.message}</p>
                    <p className="text-xs text-muted-foreground">{formatRelativeTime(activity.timestamp)}</p>
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Provider Health */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Server className="h-4 w-4" />
            提供商状态
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid gap-3 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {stats.providerHealth.map((provider) => (
              <div
                key={provider.providerId}
                className="flex items-center justify-between rounded-lg border p-3"
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium truncate">{provider.displayName}</p>
                  <p className="text-xs text-muted-foreground">
                    {formatNumber(provider.requestsTotal)} 请求
                  </p>
                </div>
                <StatusBadge status={provider.status} />
              </div>
            ))}
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

function dashboardTrendParams(range: DashboardRange, from: string, to: string): DashboardStatsParams {
  if (range !== 'custom') return { range }
  const fromMs = dateTimeLocalToMillis(from)
  const toMs = dateTimeLocalToMillis(to)
  if (!fromMs || !toMs || Number(fromMs) >= Number(toMs)) return { range: '1d' }
  return { range, from: fromMs, to: toMs }
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

function toDateTimeLocal(timestamp: number): string {
  const date = new Date(timestamp)
  const localDate = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return localDate.toISOString().slice(0, 16)
}

function dateTimeLocalToMillis(value: string): string | undefined {
  const timestamp = new Date(value).getTime()
  return Number.isFinite(timestamp) ? String(timestamp) : undefined
}
