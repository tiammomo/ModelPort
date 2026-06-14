import { useRef, type CSSProperties, type ElementType, type ReactNode } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { PaginationBar } from '@/components/shared/PaginationBar'
import { Card, CardContent, CardFooter } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { cn, formatLatency } from '@/lib/utils'
import {
  ArrowDown,
  ArrowUp,
  Database,
  Search,
  Server,
  UserRound,
} from 'lucide-react'
import type { RequestLog } from '@/types'
import {
  billingModeLabel,
  compactDetail,
  formatCompactTokenCount,
  formatInteger,
  formatMoney,
  latencyTone,
  parseLogDate,
  protocolLabel,
  providerTone,
  rowTone,
  shortId,
} from './log-utils'

const ROW_HEIGHT = 96
const OVERSCAN = 10
const TABLE_MIN_WIDTH = 1320
const TABLE_GRID_STYLE: CSSProperties = {
  gridTemplateColumns: '36px 126px 150px minmax(150px,1fr) minmax(170px,1.1fr) 128px 144px 104px 96px minmax(210px,1.2fr)',
}

// ── Cell components ──────────────────────────────────────────────

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
    <div className="mx-auto w-[118px] space-y-2">
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="text-muted-foreground">总耗时</span>
        <span className="font-mono font-medium">{formatLatency(log.latencyMs)}</span>
      </div>
      <div className="h-1.5 rounded-full bg-muted">
        <div className={cn('h-full rounded-full', latencyTone(log.latencyMs))} style={{ width: `${width}%` }} />
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
    <div className="mx-auto w-[128px] space-y-1.5 py-0.5">
      <div className="flex items-center justify-between gap-3">
        <TokenValue icon={ArrowDown} value={log.inputTokens} className="text-emerald-600" title="输入 Tokens" />
        <TokenValue icon={ArrowUp} value={log.outputTokens} className="text-violet-500" title="输出 Tokens" />
      </div>
      <div
        className={cn(
          'flex items-center gap-1.5 font-mono text-sm',
          cacheTokens > 0 ? 'text-sky-600' : 'text-muted-foreground',
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
    <div className="max-w-[260px] space-y-1.5 text-xs leading-5">
      <p className={cn('break-words', log.errorMessage ? 'font-medium text-destructive' : 'text-foreground')}>
        {log.errorMessage || log.detail || compactDetail(log)}
      </p>
      <p className="font-mono text-muted-foreground">{shortId(log.id)}</p>
    </div>
  )
}

// ── Shared small components ──────────────────────────────────────

function ProviderBadge({ log }: { log: RequestLog }) {
  return (
    <Badge variant="outline" className={cn('max-w-[132px] gap-1.5 px-2 py-1', providerTone(log.provider))}>
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
      <Icon className={cn('h-4 w-4 shrink-0', className)} />
      <span className="font-mono text-sm font-medium text-foreground">{formatInteger(value)}</span>
    </div>
  )
}

function CodePill({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex max-w-full items-center rounded-md border bg-muted/50 px-2 py-0.5 font-mono text-xs">
      <span className="truncate">{children}</span>
    </span>
  )
}

// ── Table header (grid-based to match virtual rows) ──────────────

function TableHeaderRow() {
  return (
    <div
      className="grid items-center border-b bg-muted/40 px-4 text-xs font-medium text-muted-foreground hover:bg-muted/40"
      style={TABLE_GRID_STYLE}
    >
      <div />
      <div className="py-3">时间 / 状态</div>
      <div className="py-3">渠道</div>
      <div className="py-3">身份</div>
      <div className="py-3">模型</div>
      <div className="py-3 text-center">延迟</div>
      <div className="py-3 text-center">Tokens</div>
      <div className="py-3 text-center">花费</div>
      <div className="py-3">网络</div>
      <div className="py-3">详情</div>
    </div>
  )
}

// ── Virtual row ──────────────────────────────────────────────────

function VirtualRow({
  log,
  onSelect,
}: {
  log: RequestLog
  onSelect: (log: RequestLog) => void
}) {
  return (
    <div
      className={cn(
        'grid min-h-[96px] cursor-pointer items-start border-b border-l-4 px-4 transition-colors hover:bg-muted/30',
        rowTone(log.status),
      )}
      style={TABLE_GRID_STYLE}
      onClick={() => onSelect(log)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') onSelect(log)
      }}
    >
      <div className="flex items-center justify-center">
        <div className="h-2 w-2 rounded-full bg-muted-foreground/30" />
      </div>
      <div className="py-3">
        <TimeStatusCell log={log} />
      </div>
      <div className="py-3">
        <RouteCell log={log} />
      </div>
      <div className="py-3">
        <IdentityCell log={log} />
      </div>
      <div className="py-3">
        <ModelCell log={log} />
      </div>
      <div className="py-3">
        <LatencyCell log={log} />
      </div>
      <div className="py-3">
        <TokensCell log={log} />
      </div>
      <div className="py-3">
        <CostCell log={log} />
      </div>
      <div className="py-3">
        <NetworkCell log={log} />
      </div>
      <div className="py-3">
        <DetailPreview log={log} />
      </div>
    </div>
  )
}

// ── Main table component ─────────────────────────────────────────

export function LogsTable({
  logs,
  total,
  page,
  pageSize,
  totalPages,
  start,
  end,
  pageSizeOptions,
  isLoading,
  onPageChange,
  onPageSizeChange,
  onSelectLog,
}: {
  logs: RequestLog[]
  total: number
  page: number
  pageSize: number
  totalPages: number
  start: number
  end: number
  pageSizeOptions: number[]
  isLoading: boolean
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
  onSelectLog: (log: RequestLog) => void
}) {
  const parentRef = useRef<HTMLDivElement>(null)

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: logs.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: OVERSCAN,
  })

  const bodyHeight =
    logs.length === 0
      ? 240
      : Math.min(640, Math.max(180, Math.min(logs.length, 8) * ROW_HEIGHT))

  return (
    <Card className="overflow-hidden rounded-lg shadow-sm">
      <CardContent className="p-0">
        {/* Table header */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b bg-card px-4 py-3">
          <div>
            <p className="text-sm font-semibold">调用明细</p>
            <p className="text-xs text-muted-foreground">缓存 · 计价 · 错误上下文 · 点击行查看详情</p>
          </div>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <span>共</span>
            <span className="font-mono text-foreground">{formatInteger(total)}</span>
            <span>条</span>
          </div>
        </div>

        <div className="overflow-x-auto">
          <div style={{ minWidth: TABLE_MIN_WIDTH }}>
            {/* Column headers */}
            <TableHeaderRow />

            {/* Virtualized body */}
            {logs.length === 0 && !isLoading ? (
              <div className="flex h-60 flex-col items-center justify-center gap-2 text-center">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                  <Search className="h-5 w-5" />
                </div>
                <p className="font-medium">没有匹配的请求日志</p>
                <p className="text-sm text-muted-foreground">当前筛选下暂无记录。</p>
              </div>
            ) : (
              <div
                ref={parentRef}
                className="overflow-y-auto"
                style={{ height: bodyHeight }}
              >
                <div
                  style={{
                    height: `${virtualizer.getTotalSize()}px`,
                    width: '100%',
                    position: 'relative',
                  }}
                >
                  {virtualizer.getVirtualItems().map((virtualItem) => {
                    const log = logs[virtualItem.index]
                    return (
                      <div
                        key={virtualItem.key}
                        data-index={virtualItem.index}
                        ref={virtualizer.measureElement}
                        style={{
                          position: 'absolute',
                          top: 0,
                          left: 0,
                          width: '100%',
                          transform: `translateY(${virtualItem.start}px)`,
                        }}
                      >
                        <VirtualRow log={log} onSelect={onSelectLog} />
                      </div>
                    )
                  })}
                </div>
              </div>
            )}
          </div>
        </div>
      </CardContent>
      <CardFooter className="border-t px-4 py-3">
        <PaginationBar
          total={total}
          page={page}
          pageSize={pageSize}
          totalPages={totalPages}
          start={start}
          end={end}
          totalLabel="条日志"
          pageSizeOptions={pageSizeOptions}
          onPageChange={onPageChange}
          onPageSizeChange={onPageSizeChange}
        />
      </CardFooter>
    </Card>
  )
}
