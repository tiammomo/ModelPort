import { useState } from 'react'
import { useLogs, useLatencyStats } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { TableToolbar } from '@/components/shared/TableToolbar'
import { MetricCard } from '@/components/shared/MetricCard'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Card, CardContent } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { formatNumber, formatLatency, formatDate, truncateId } from '@/lib/utils'
import { BarChart3, Clock, CheckCircle, Search, ChevronLeft, ChevronRight, Eye } from 'lucide-react'
import type { LogFilters, RequestLog, RequestStatus } from '@/types'

export function LogsPage() {
  const [filters, setFilters] = useState<LogFilters>({})
  const [page, setPage] = useState(1)
  const [selectedLog, setSelectedLog] = useState<RequestLog | null>(null)
  const pageSize = 15

  const { data, isLoading } = useLogs(filters, page, pageSize)
  const { data: latencyStats } = useLatencyStats()

  const logs = data?.logs || []
  const total = data?.total || 0
  const totalPages = Math.ceil(total / pageSize)

  if (isLoading) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  return (
    <div className="space-y-6">
      <PageHeader title="请求日志" description="查看和分析 API 请求历史" />

      {/* Stats Cards */}
      <div className="grid gap-4 md:grid-cols-4">
        <MetricCard title="总请求数" value={formatNumber(total)} icon={BarChart3} description="匹配当前筛选" />
        <MetricCard title="成功率" value={`${total > 0 ? Math.round((logs.filter((l) => l.status === 'success').length / Math.max(logs.length, 1)) * 100) : 0}%`} icon={CheckCircle} />
        <MetricCard title="平均延迟" value={latencyStats ? formatLatency(latencyStats.avg) : '-'} icon={Clock} />
        <MetricCard title="P99 延迟" value={latencyStats ? formatLatency(latencyStats.p99) : '-'} icon={Clock} />
      </div>

      {/* Filters */}
      <Card>
        <CardContent className="p-4">
          <TableToolbar
            actions={(
              <Button variant="outline" onClick={() => { setFilters({}); setPage(1) }}>重置</Button>
            )}
          >
            <div className="flex-1 min-w-[200px]">
              <div className="relative">
                <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
                <Input
                  placeholder="搜索请求 ID、用户、模型..."
                  className="pl-8"
                  value={filters.search || ''}
                  onChange={(e) => { setFilters({ ...filters, search: e.target.value }); setPage(1) }}
                />
              </div>
            </div>
            <Select value={filters.provider || '__all__'} onValueChange={(v) => { setFilters({ ...filters, provider: v === '__all__' ? undefined : v }); setPage(1) }}>
              <SelectTrigger className="w-[150px]"><SelectValue placeholder="提供商" /></SelectTrigger>
              <SelectContent>
                <SelectItem value="__all__">全部提供商</SelectItem>
                <SelectItem value="mimo">Mimo</SelectItem>
                <SelectItem value="deepseek">DeepSeek</SelectItem>
                <SelectItem value="openai">OpenAI</SelectItem>
                <SelectItem value="anthropic">Anthropic</SelectItem>
                <SelectItem value="gemini">Gemini</SelectItem>
                <SelectItem value="dashscope">DashScope</SelectItem>
              </SelectContent>
            </Select>
            <Select value={filters.status || '__all__'} onValueChange={(v) => { setFilters({ ...filters, status: v === '__all__' ? undefined : v as RequestStatus }); setPage(1) }}>
              <SelectTrigger className="w-[120px]"><SelectValue placeholder="状态" /></SelectTrigger>
              <SelectContent>
                <SelectItem value="__all__">全部状态</SelectItem>
                <SelectItem value="success">成功</SelectItem>
                <SelectItem value="error">错误</SelectItem>
                <SelectItem value="timeout">超时</SelectItem>
              </SelectContent>
            </Select>
          </TableToolbar>
        </CardContent>
      </Card>

      {/* Log Table */}
      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>时间</TableHead>
                <TableHead>请求 ID</TableHead>
                <TableHead>用户</TableHead>
                <TableHead>模型</TableHead>
                <TableHead>提供商</TableHead>
                <TableHead>流模式</TableHead>
                <TableHead>状态</TableHead>
                <TableHead className="text-right">延迟</TableHead>
                <TableHead className="text-right">Token (入/出)</TableHead>
                <TableHead className="w-12"></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {logs.map((log) => (
                <TableRow key={log.id}>
                  <TableCell className="text-xs text-muted-foreground whitespace-nowrap">
                    {formatDate(log.timestamp)}
                  </TableCell>
                  <TableCell className="font-mono text-xs">{truncateId(log.id)}</TableCell>
                  <TableCell>{log.username}</TableCell>
                  <TableCell className="font-mono text-xs">{log.model}</TableCell>
                  <TableCell>{log.provider}</TableCell>
                  <TableCell>
                    <Badge variant={log.stream === 'stream' ? 'default' : 'secondary'}>
                      {log.stream === 'stream' ? '流式' : '非流式'}
                    </Badge>
                  </TableCell>
                  <TableCell><StatusBadge status={log.status} /></TableCell>
                  <TableCell className="text-right text-sm">{formatLatency(log.latencyMs)}</TableCell>
                  <TableCell className="text-right text-sm text-muted-foreground">
                    {formatNumber(log.inputTokens + (log.cacheReadTokens || 0) + (log.cacheWriteTokens || 0))} / {formatNumber(log.outputTokens)}
                  </TableCell>
                  <TableCell>
                    <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => setSelectedLog(log)}>
                      <Eye className="h-4 w-4" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Pagination */}
      {totalPages > 1 && (
        <div className="flex items-center justify-between">
          <p className="text-sm text-muted-foreground">
            共 {total} 条，第 {page} / {totalPages} 页
          </p>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage(page - 1)}>
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage(page + 1)}>
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      )}

      {/* Log Detail Dialog */}
      <Dialog open={!!selectedLog} onOpenChange={() => setSelectedLog(null)}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>请求详情</DialogTitle>
            <DialogDescription className="font-mono">{selectedLog?.id}</DialogDescription>
          </DialogHeader>
          {selectedLog && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-xs text-muted-foreground">时间</p>
                  <p className="text-sm">{formatDate(selectedLog.timestamp)}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">用户</p>
                  <p className="text-sm">{selectedLog.username}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">请求模型</p>
                  <p className="text-sm font-mono">{selectedLog.model}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">解析模型</p>
                  <p className="text-sm font-mono">{selectedLog.resolvedModel}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">提供商</p>
                  <p className="text-sm">{selectedLog.provider} ({selectedLog.protocol})</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">流模式</p>
                  <p className="text-sm">{selectedLog.stream === 'stream' ? '流式' : '非流式'}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">状态码</p>
                  <p className="text-sm">{selectedLog.statusCode}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">延迟</p>
                  <p className="text-sm">{formatLatency(selectedLog.latencyMs)}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">输入 Token</p>
                  <p className="text-sm">{formatNumber(selectedLog.inputTokens)}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">输出 Token</p>
                  <p className="text-sm">{formatNumber(selectedLog.outputTokens)}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Cache Write</p>
                  <p className="text-sm">{formatNumber(selectedLog.cacheWriteTokens || 0)}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Cache Read</p>
                  <p className="text-sm">{formatNumber(selectedLog.cacheReadTokens || 0)}</p>
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">估算成本</p>
                  <p className="text-sm">${(selectedLog.costEstimate || 0).toFixed(6)}</p>
                </div>
              </div>
              {selectedLog.errorMessage && (
                <div>
                  <p className="text-xs text-muted-foreground mb-1">错误信息</p>
                  <pre className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">{selectedLog.errorMessage}</pre>
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
