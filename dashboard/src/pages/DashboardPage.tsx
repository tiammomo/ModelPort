import { useDashboard } from '@/hooks'
import { MetricCard } from '@/components/shared/MetricCard'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { formatNumber, formatRelativeTime } from '@/lib/utils'
import { Activity, Users, Server, Clock, TrendingUp, AlertCircle, Info } from 'lucide-react'
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer, AreaChart, Area } from 'recharts'

export function DashboardPage() {
  const { data: stats, isLoading } = useDashboard()

  if (isLoading || !stats) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  const chartData = stats.requestTimeSeries.map((p, i) => ({
    time: new Date(p.timestamp).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }),
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
