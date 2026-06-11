import { Fragment, useState } from 'react'
import type { ElementType, ReactNode } from 'react'
import { useLogs } from '@/hooks'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Card, CardContent } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn, formatLatency } from '@/lib/utils'
import {
  Activity,
  ArrowDown,
  ArrowUp,
  BadgeDollarSign,
  CalendarClock,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Database,
  DatabaseZap,
  Filter,
  Gauge,
  RotateCcw,
  Search,
  Server,
  UserRound,
  WalletCards,
  Zap,
} from 'lucide-react'
import type { LogFilters, RequestLog, RequestStatus, StreamMode } from '@/types'

const ALL = '__all__'
const PAGE_SIZE = 25

export function LogsPage() {
  const [filters, setFilters] = useState<LogFilters>({})
  const [page, setPage] = useState(1)
  const [expandedId, setExpandedId] = useState<string | null>(null)

  const { data, isLoading } = useLogs(filters, page, PAGE_SIZE)
  const logs = data?.logs || []
  const total = data?.total || 0
  const summary = data?.summary
  const totalPages = Math.max(Math.ceil(total / PAGE_SIZE), 1)
  const activeFilterCount = Object.values(filters).filter(Boolean).length

  const totalRequests = summary?.totalRequests || 0
  const successRequests = summary?.successRequests || 0
  const successRate = totalRequests > 0 ? (successRequests / totalRequests) * 100 : 0
  const cacheTokens = (summary?.totalCacheWriteTokens || 0) + (summary?.totalCacheReadTokens || 0)

  const updateFilters = (next: LogFilters) => {
    setFilters(next)
    setPage(1)
    setExpandedId(null)
  }

  if (isLoading) {
    return <div className="flex h-64 items-center justify-center text-muted-foreground">加载中...</div>
  }

  return (
    <div className="space-y-5">
      <section>
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0 space-y-1">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline" className="gap-1.5 border-emerald-200 bg-emerald-50 text-emerald-700">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                实时聚合
              </Badge>
              <Badge variant="outline" className="border-slate-200 bg-slate-50 text-slate-700">
                {formatInteger(total)} 条结果
              </Badge>
            </div>
            <h2 className="text-2xl font-bold tracking-tight">请求日志</h2>
            <p className="max-w-3xl text-sm text-muted-foreground">
              路由、身份、缓存、计费和延迟的请求明细。
            </p>
          </div>
          <div className="flex items-center gap-2 rounded-lg border bg-background px-3 py-2 text-sm shadow-sm">
            <Clock3 className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground">每页</span>
            <span className="font-mono font-semibold">{PAGE_SIZE}</span>
          </div>
        </div>
      </section>

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

      <Card className="overflow-hidden rounded-lg shadow-sm">
        <CardContent className="p-0">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b bg-muted/20 px-4 py-3">
            <div className="flex items-center gap-3">
              <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
                <Filter className="h-4 w-4" />
              </div>
              <div>
                <p className="text-sm font-semibold">筛选条件</p>
                <p className="text-xs text-muted-foreground">时间 · 来源 · 身份 · 状态</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              {activeFilterCount > 0 && (
                <Badge variant="outline" className="border-primary/20 bg-primary/5 text-primary">
                  已启用 {activeFilterCount}
                </Badge>
              )}
              <Button variant="outline" size="sm" disabled={activeFilterCount === 0} onClick={() => updateFilters({})}>
                <RotateCcw className="h-4 w-4" />
                重置
              </Button>
            </div>
          </div>

          <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-12">
            <FilterField label="开始时间" icon={CalendarClock} className="xl:col-span-2">
              <Input
                type="datetime-local"
                className="h-10"
                value={filters.dateFrom || ''}
                onChange={(event) => updateFilters({ ...filters, dateFrom: event.target.value || undefined })}
              />
            </FilterField>
            <FilterField label="结束时间" icon={CalendarClock} className="xl:col-span-2">
              <Input
                type="datetime-local"
                className="h-10"
                value={filters.dateTo || ''}
                onChange={(event) => updateFilters({ ...filters, dateTo: event.target.value || undefined })}
              />
            </FilterField>
            <FilterField label="关键词" icon={Search} className="md:col-span-2 xl:col-span-3">
              <div className="relative">
                <Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
                <Input
                  placeholder="模型、渠道、令牌、请求 ID"
                  className="h-10 pl-9"
                  value={filters.search || ''}
                  onChange={(event) => updateFilters({ ...filters, search: event.target.value || undefined })}
                />
              </div>
            </FilterField>
            <FilterField label="用户" icon={UserRound} className="xl:col-span-2">
              <Input
                placeholder="用户名"
                className="h-10"
                value={filters.username || ''}
                onChange={(event) => updateFilters({ ...filters, username: event.target.value || undefined })}
              />
            </FilterField>
            <FilterField label="分组" icon={WalletCards} className="xl:col-span-3">
              <Input
                placeholder="API Key 分组"
                className="h-10"
                value={filters.group || ''}
                onChange={(event) => updateFilters({ ...filters, group: event.target.value || undefined })}
              />
            </FilterField>
            <FilterField label="渠道" icon={Server} className="xl:col-span-2">
              <Select value={filters.provider || ALL} onValueChange={(value) => updateFilters({ ...filters, provider: value === ALL ? undefined : value })}>
                <SelectTrigger className="h-10"><SelectValue placeholder="全部渠道" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL}>全部渠道</SelectItem>
                  <SelectItem value="mimo">Mimo</SelectItem>
                  <SelectItem value="deepseek">DeepSeek</SelectItem>
                  <SelectItem value="openai">OpenAI</SelectItem>
                  <SelectItem value="anthropic">Anthropic</SelectItem>
                  <SelectItem value="gemini">Gemini</SelectItem>
                  <SelectItem value="dashscope">DashScope</SelectItem>
                  <SelectItem value="openrouter">OpenRouter</SelectItem>
                  <SelectItem value="kimi">Kimi</SelectItem>
                  <SelectItem value="zhipu">Zhipu</SelectItem>
                </SelectContent>
              </Select>
            </FilterField>
            <FilterField label="状态" icon={Activity} className="xl:col-span-2">
              <Select value={filters.status || ALL} onValueChange={(value) => updateFilters({ ...filters, status: value === ALL ? undefined : value as RequestStatus })}>
                <SelectTrigger className="h-10"><SelectValue placeholder="全部状态" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL}>全部状态</SelectItem>
                  <SelectItem value="success">成功</SelectItem>
                  <SelectItem value="error">错误</SelectItem>
                  <SelectItem value="timeout">超时</SelectItem>
                </SelectContent>
              </Select>
            </FilterField>
            <FilterField label="模式" icon={Zap} className="xl:col-span-2">
              <Select value={filters.stream || ALL} onValueChange={(value) => updateFilters({ ...filters, stream: value === ALL ? undefined : value as StreamMode })}>
                <SelectTrigger className="h-10"><SelectValue placeholder="全部模式" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL}>全部模式</SelectItem>
                  <SelectItem value="stream">流式</SelectItem>
                  <SelectItem value="non-stream">非流式</SelectItem>
                </SelectContent>
              </Select>
            </FilterField>
          </div>
        </CardContent>
      </Card>

      <Card className="overflow-hidden rounded-lg shadow-sm">
        <CardContent className="p-0">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b bg-card px-4 py-3">
            <div>
              <p className="text-sm font-semibold">调用明细</p>
              <p className="text-xs text-muted-foreground">缓存 · 计价 · 错误上下文</p>
            </div>
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <span>第</span>
              <span className="font-mono text-foreground">{page}</span>
              <span>/</span>
              <span className="font-mono text-foreground">{totalPages}</span>
              <span>页</span>
            </div>
          </div>

          <Table className="min-w-[1380px]">
            <TableHeader>
              <TableRow className="bg-muted/40 hover:bg-muted/40">
                <CenteredHead className="w-10" />
                <TableHead className="min-w-[170px]">时间 / 状态</TableHead>
                <TableHead className="min-w-[180px]">渠道</TableHead>
                <TableHead className="min-w-[190px]">身份</TableHead>
                <TableHead className="min-w-[210px]">模型</TableHead>
                <CenteredHead className="min-w-[165px]">延迟</CenteredHead>
                <CenteredHead className="min-w-[150px]">Tokens</CenteredHead>
                <CenteredHead className="min-w-[130px]">花费</CenteredHead>
                <TableHead className="min-w-[130px]">网络</TableHead>
                <TableHead className="min-w-[270px]">详情</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {logs.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={10} className="h-40 text-center">
                    <div className="mx-auto max-w-sm space-y-2">
                      <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                        <Search className="h-5 w-5" />
                      </div>
                      <p className="font-medium">没有匹配的请求日志</p>
                      <p className="text-sm text-muted-foreground">当前筛选下暂无记录。</p>
                    </div>
                  </TableCell>
                </TableRow>
              ) : logs.map((log) => {
                const isExpanded = expandedId === log.id
                return (
                  <Fragment key={log.id}>
                    <TableRow className={cn("border-l-4 align-top", rowTone(log.status), isExpanded && "bg-muted/30")}>
                      <TableCell className="text-center">
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 rounded-md"
                          onClick={() => setExpandedId(isExpanded ? null : log.id)}
                          aria-label={isExpanded ? '收起详情' : '展开详情'}
                        >
                          {isExpanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                        </Button>
                      </TableCell>
                      <TableCell>
                        <TimeStatusCell log={log} />
                      </TableCell>
                      <TableCell>
                        <RouteCell log={log} />
                      </TableCell>
                      <TableCell>
                        <IdentityCell log={log} />
                      </TableCell>
                      <TableCell>
                        <ModelCell log={log} />
                      </TableCell>
                      <TableCell>
                        <LatencyCell log={log} />
                      </TableCell>
                      <TableCell>
                        <TokensCell log={log} />
                      </TableCell>
                      <TableCell>
                        <CostCell log={log} />
                      </TableCell>
                      <TableCell>
                        <NetworkCell log={log} />
                      </TableCell>
                      <TableCell>
                        <DetailPreview log={log} />
                      </TableCell>
                    </TableRow>
                    {isExpanded && (
                      <TableRow className="bg-muted/25 hover:bg-muted/25">
                        <TableCell colSpan={10} className="p-0">
                          <LogExpandedDetail log={log} />
                        </TableCell>
                      </TableRow>
                    )}
                  </Fragment>
                )
              })}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-sm text-muted-foreground">共 {formatInteger(total)} 条，第 {page} / {totalPages} 页</p>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage(page - 1)}>
            <ChevronLeft className="h-4 w-4" />
            上一页
          </Button>
          <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage(page + 1)}>
            下一页
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  )
}

function SummaryMetric({
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
        <div className={cn("flex h-10 w-10 shrink-0 items-center justify-center rounded-lg ring-1", tones[tone])}>
          <Icon className="h-5 w-5" />
        </div>
      </div>
      <p className="mt-3 truncate text-xs text-muted-foreground">{helper}</p>
    </div>
  )
}

function FilterField({ label, icon: Icon, children, className }: { label: string; icon: ElementType; children: ReactNode; className?: string }) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Icon className="h-3.5 w-3.5" />
        <span>{label}</span>
      </div>
      {children}
    </div>
  )
}

function TimeStatusCell({ log }: { log: RequestLog }) {
  const date = parseLogDate(log.timestamp)

  return (
    <div className="space-y-2">
      <div className="font-mono text-xs leading-5">
        {date ? (
          <>
            <div>{date.toLocaleDateString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit' })}</div>
            <div className="text-muted-foreground">{date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })}</div>
          </>
        ) : (
          <div className="text-muted-foreground">{log.timestamp}</div>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <StatusBadge status={log.status} className="rounded-md" />
        <Badge variant="outline" className="font-mono text-[11px]">
          {log.statusCode}
        </Badge>
      </div>
    </div>
  )
}

function RouteCell({ log }: { log: RequestLog }) {
  return (
    <div className="space-y-1.5">
      <ProviderBadge log={log} />
      <div className="space-y-0.5 text-xs text-muted-foreground">
        <div className="font-mono">{log.channelId || log.provider}</div>
        <div>{protocolLabel(log.protocol)}</div>
      </div>
    </div>
  )
}

function IdentityCell({ log }: { log: RequestLog }) {
  return (
    <div className="space-y-2">
      <div className="flex min-w-0 items-center gap-2">
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
          <UserRound className="h-3.5 w-3.5" />
        </div>
        <span className="truncate text-sm font-medium">{log.username}</span>
      </div>
      <div className="flex flex-wrap gap-1.5">
        <CodePill>{log.tokenName || log.apiKeyName || 'legacy'}</CodePill>
        <CodePill>{log.group || log.apiKeyGroup || 'default'}</CodePill>
      </div>
    </div>
  )
}

function ModelCell({ log }: { log: RequestLog }) {
  return (
    <div className="space-y-1">
      <div className="break-all font-mono text-xs font-medium">{log.resolvedModel || log.model}</div>
      {log.model !== log.resolvedModel && (
        <div className="break-all text-xs text-muted-foreground">{log.model}</div>
      )}
      <RequestModeBadge log={log} />
    </div>
  )
}

function LatencyCell({ log }: { log: RequestLog }) {
  const firstByte = log.firstByteLatencyMs || log.latencyMs
  const width = Math.min(100, Math.max(8, (log.latencyMs / 12000) * 100))

  return (
    <div className="mx-auto w-[150px] space-y-2">
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="text-muted-foreground">总耗时</span>
        <span className="font-mono font-medium">{formatLatency(log.latencyMs)}</span>
      </div>
      <div className="h-1.5 rounded-full bg-muted">
        <div className={cn("h-full rounded-full", latencyTone(log.latencyMs))} style={{ width: `${width}%` }} />
      </div>
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="text-muted-foreground">首字</span>
        <span className="font-mono">{formatLatency(firstByte)}</span>
      </div>
    </div>
  )
}

function TokensCell({ log }: { log: RequestLog }) {
  const cacheWriteTokens = log.cacheWriteTokens || 0
  const cacheReadTokens = log.cacheReadTokens || 0
  const cacheTokens = cacheWriteTokens + cacheReadTokens

  return (
    <div className="mx-auto w-[135px] space-y-1.5 py-0.5">
      <div className="flex items-center justify-between gap-3">
        <TokenValue icon={ArrowDown} value={log.inputTokens} className="text-emerald-600" title="输入 Tokens" />
        <TokenValue icon={ArrowUp} value={log.outputTokens} className="text-violet-500" title="输出 Tokens" />
      </div>
      <div
        className={cn(
          "flex items-center gap-1.5 font-mono text-sm",
          cacheTokens > 0 ? "text-sky-600" : "text-muted-foreground"
        )}
        title={`缓存写 ${formatInteger(cacheWriteTokens)} / 缓存读 ${formatInteger(cacheReadTokens)}`}
      >
        <Database className="h-3.5 w-3.5 shrink-0" />
        <span>{formatCompactTokenCount(cacheTokens)}</span>
      </div>
    </div>
  )
}

function CostCell({ log }: { log: RequestLog }) {
  return (
    <div className="space-y-1 text-center">
      <div className="font-mono text-sm font-semibold">{formatMoney(log.costEstimate || 0, 6)}</div>
      <div className="text-xs text-muted-foreground">{billingModeLabel(log.billingMode)}</div>
    </div>
  )
}

function NetworkCell({ log }: { log: RequestLog }) {
  return (
    <div className="space-y-1.5 text-xs">
      <div className="font-mono text-muted-foreground">{log.clientIp || '-'}</div>
      <Badge variant="outline" className="font-mono text-[11px]">
        retry {log.retryCount || 0}
      </Badge>
    </div>
  )
}

function DetailPreview({ log }: { log: RequestLog }) {
  return (
    <div className="max-w-[320px] space-y-1.5 text-xs leading-5">
      <p className={cn("break-words", log.errorMessage ? "font-medium text-destructive" : "text-foreground")}>
        {log.errorMessage || log.detail || compactDetail(log)}
      </p>
      <p className="font-mono text-muted-foreground">{shortId(log.id)}</p>
    </div>
  )
}

function LogExpandedDetail({ log }: { log: RequestLog }) {
  const pricing = log.modelPricing
  const cost = log.costBreakdown

  return (
    <div className="grid gap-x-8 gap-y-5 border-t bg-muted/20 p-5 text-sm lg:grid-cols-3">
      <DetailSection title="路由与请求" icon={Server}>
        <DetailLine label="请求 ID" value={log.id} mono />
        <DetailLine label="渠道信息" value={`${log.channelId || log.provider} - ${log.channelName || log.provider}`} />
        <DetailLine label="请求路径" value={log.requestPath || '/v1/messages'} mono />
        <DetailLine label="流式模式" value={log.stream === 'stream' ? '流式' : '非流式'} />
        <DetailLine label="日志详情" value={compactDetail(log)} />
      </DetailSection>
      <DetailSection title="缓存与用量" icon={DatabaseZap}>
        <DetailLine label="输入 Tokens" value={formatInteger(log.inputTokens)} mono />
        <DetailLine label="输出 Tokens" value={formatInteger(log.outputTokens)} mono />
        <DetailLine label="缓存创建" value={formatInteger(log.cacheWriteTokens || 0)} highlight mono />
        <DetailLine label="缓存命中" value={`${formatInteger(log.cacheReadTokens || 0)} (${formatPercent(log.cacheHitRate || 0)})`} mono />
        <DetailLine label="计费 Tokens" value={formatInteger(log.billedInputTokens || log.inputTokens)} mono />
      </DetailSection>
      <DetailSection title="计费拆分" icon={BadgeDollarSign}>
        <DetailLine
          label="价格"
          value={pricing
            ? `提示 $${pricing.inputPerMillion}/1M · 补全 $${pricing.outputPerMillion}/1M · 缓存创建 $${pricing.cacheWritePerMillion}/1M · 缓存命中 $${pricing.cacheReadPerMillion}/1M`
            : '未返回价格'}
        />
        <DetailLine label="计算过程" value={costFormula(log)} mono />
        <DetailLine label="输入成本" value={formatMoney(cost?.inputCost || 0, 6)} mono />
        <DetailLine label="缓存成本" value={`${formatMoney(cost?.cacheWriteCost || 0, 6)} / ${formatMoney(cost?.cacheReadCost || 0, 6)}`} mono />
        <DetailLine label="输出成本" value={formatMoney(cost?.outputCost || 0, 6)} mono />
        {log.errorMessage && <DetailLine label="错误信息" value={log.errorMessage} danger />}
      </DetailSection>
    </div>
  )
}

function DetailSection({ title, icon: Icon, children }: { title: string; icon: ElementType; children: ReactNode }) {
  return (
    <div className="min-w-0 space-y-3">
      <div className="flex items-center gap-2 border-b pb-2">
        <Icon className="h-4 w-4 text-muted-foreground" />
        <p className="font-medium">{title}</p>
      </div>
      <div className="space-y-2.5">{children}</div>
    </div>
  )
}

function DetailLine({ label, value, highlight, danger, mono }: { label: string; value: string; highlight?: boolean; danger?: boolean; mono?: boolean }) {
  return (
    <div className="grid gap-1.5 sm:grid-cols-[92px_1fr]">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div
        className={cn(
          "min-w-0 break-words text-sm",
          mono && "font-mono text-xs leading-5",
          highlight && "font-medium text-rose-700",
          danger && "font-medium text-destructive"
        )}
      >
        {value}
      </div>
    </div>
  )
}

function ProviderBadge({ log }: { log: RequestLog }) {
  return (
    <Badge variant="outline" className={cn("max-w-[170px] gap-1.5 px-2 py-1", providerTone(log.provider))}>
      <Server className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate">{log.channelName || log.provider}</span>
    </Badge>
  )
}

function RequestModeBadge({ log }: { log: RequestLog }) {
  if (log.status !== 'success') {
    return <Badge variant="outline" className="border-rose-200 bg-rose-50 text-rose-700">异常</Badge>
  }
  return (
    <div className="flex flex-wrap gap-1">
      <Badge variant="outline" className="border-lime-200 bg-lime-50 text-lime-700">消费</Badge>
      {log.stream === 'stream' && <Badge variant="outline" className="border-sky-200 bg-sky-50 text-sky-700">流式</Badge>}
    </div>
  )
}

function TokenValue({
  icon: Icon,
  value,
  className,
  title,
}: {
  icon: ElementType
  value: number
  className: string
  title: string
}) {
  return (
    <div className="flex min-w-0 items-center gap-1.5" title={title}>
      <Icon className={cn("h-4 w-4 shrink-0", className)} />
      <span className="font-mono text-sm font-medium text-foreground">{formatInteger(value)}</span>
    </div>
  )
}

function CenteredHead({ children, className }: { children?: ReactNode; className?: string }) {
  return <TableHead className={cn("text-center", className)}>{children}</TableHead>
}

function CodePill({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex max-w-full items-center rounded-md border bg-muted/50 px-2 py-0.5 font-mono text-xs">
      <span className="truncate">{children}</span>
    </span>
  )
}

function compactDetail(log: RequestLog) {
  const pricing = log.modelPricing
  if (!pricing) return `模型: ${log.resolvedModel} · 缓存创建: ${formatInteger(log.cacheWriteTokens || 0)}`
  return `模型: ${log.resolvedModel} · 缓存创建: ${pricing.cacheWritePerMillion}/1M · 缓存命中: ${pricing.cacheReadPerMillion}/1M`
}

function costFormula(log: RequestLog) {
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

function rowTone(status: RequestStatus) {
  if (status === 'error') return 'border-l-rose-400 bg-rose-50/30 hover:bg-rose-50/60'
  if (status === 'timeout') return 'border-l-amber-400 bg-amber-50/30 hover:bg-amber-50/60'
  return 'border-l-transparent'
}

function providerTone(provider: string) {
  const key = provider.toLowerCase()
  if (key.includes('mimo')) return 'border-orange-200 bg-orange-50 text-orange-700'
  if (key.includes('deepseek')) return 'border-cyan-200 bg-cyan-50 text-cyan-700'
  if (key.includes('openai')) return 'border-emerald-200 bg-emerald-50 text-emerald-700'
  if (key.includes('anthropic')) return 'border-violet-200 bg-violet-50 text-violet-700'
  if (key.includes('gemini')) return 'border-blue-200 bg-blue-50 text-blue-700'
  if (key.includes('dashscope')) return 'border-amber-200 bg-amber-50 text-amber-700'
  return 'border-slate-200 bg-slate-50 text-slate-700'
}

function latencyTone(value: number) {
  if (value >= 6000) return 'bg-rose-500'
  if (value >= 2500) return 'bg-amber-500'
  return 'bg-emerald-500'
}

function protocolLabel(value?: string) {
  if (value === 'openai-compat') return 'OpenAI-compatible'
  if (value === 'anthropic') return 'Anthropic Messages'
  return value || 'default'
}

function billingModeLabel(value?: string) {
  if (value === 'upstream-returned') return '上游返回'
  if (value === 'metrics-fallback') return '进程指标回退'
  return value || '本地估算'
}

function parseLogDate(value: string) {
  const date = /^\d+$/.test(value) ? new Date(Number(value)) : new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return date
}

function shortId(value: string) {
  if (value.length <= 24) return value
  return `${value.slice(0, 18)}...${value.slice(-4)}`
}

function formatInteger(value: number) {
  return Math.round(value).toLocaleString('en-US')
}

function formatCompactTokenCount(value: number) {
  if (value >= 1_000_000) return `${trimFixed(value / 1_000_000)}M`
  if (value >= 1_000) return `${trimFixed(value / 1_000)}K`
  return formatInteger(value)
}

function trimFixed(value: number) {
  return value.toFixed(1).replace(/\.0$/, '')
}

function formatMoney(value: number, digits: number) {
  return `$${value.toFixed(digits)}`
}

function formatPercent(value: number) {
  return `${value.toFixed(1)}%`
}
