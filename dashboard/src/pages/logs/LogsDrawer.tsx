import { useEffect, useRef, useState, type ElementType, type ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { cn, formatLatency } from '@/lib/utils'
import {
  BadgeDollarSign,
  Check,
  Copy,
  DatabaseZap,
  Server,
  X,
} from 'lucide-react'
import type { RequestLog } from '@/types'
import {
  billingModeLabel,
  compactDetail,
  costFormula,
  formatInteger,
  formatMoney,
  formatPercent,
  latencyTone,
  parseLogDate,
  protocolLabel,
  providerTone,
  shortId,
} from './log-utils'

// ── Copy-to-clipboard helper ─────────────────────────────────────

async function copyToClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    const ta = document.createElement('textarea')
    ta.value = text
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    document.execCommand('copy')
    document.body.removeChild(ta)
  }
}

function CopyButton({ value }: { value: string }) {
  const [copied, setCopied] = useState(false)
  const timeoutRef = useRef<ReturnType<typeof setTimeout>>(undefined)

  useEffect(() => {
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current)
    }
  }, [])

  const handleClick = async (e: React.MouseEvent) => {
    e.stopPropagation()
    await copyToClipboard(value)
    setCopied(true)
    if (timeoutRef.current) clearTimeout(timeoutRef.current)
    timeoutRef.current = setTimeout(() => setCopied(false), 2000)
  }

  return (
    <button
      type="button"
      className="ml-1 inline-flex h-5 w-5 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
      title={copied ? '已复制' : '复制'}
      onClick={handleClick}
    >
      {copied ? <Check className="h-3 w-3 text-emerald-600" /> : <Copy className="h-3 w-3" />}
    </button>
  )
}

// ── Detail section ───────────────────────────────────────────────

function DetailSection({
  title,
  icon: Icon,
  children,
}: {
  title: string
  icon: ElementType
  children: ReactNode
}) {
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

function DetailLine({
  label,
  value,
  copyValue,
  highlight,
  danger,
  mono,
}: {
  label: string
  value: string
  copyValue?: string
  highlight?: boolean
  danger?: boolean
  mono?: boolean
}) {
  return (
    <div className="grid gap-1.5 sm:grid-cols-[92px_1fr]">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div
        className={cn(
          'group flex min-w-0 items-start break-words text-sm',
          mono && 'font-mono text-xs leading-5',
          highlight && 'font-medium text-rose-700',
          danger && 'font-medium text-destructive',
        )}
      >
        <span className="min-w-0 break-words">{value}</span>
        {copyValue && <CopyButton value={copyValue} />}
      </div>
    </div>
  )
}

// ── Main drawer component ────────────────────────────────────────

export function LogsDrawer({
  log,
  onClose,
}: {
  log: RequestLog | null
  onClose: () => void
}) {
  const backdropRef = useRef<HTMLDivElement>(null)

  // Close on Escape
  useEffect(() => {
    if (!log) return
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleKey)
    return () => document.removeEventListener('keydown', handleKey)
  }, [log, onClose])

  // Prevent body scroll when open
  useEffect(() => {
    if (log) {
      document.body.style.overflow = 'hidden'
      return () => {
        document.body.style.overflow = ''
      }
    }
  }, [log])

  if (!log) return null

  const pricing = log.modelPricing
  const cost = log.costBreakdown
  const date = parseLogDate(log.timestamp)

  return (
    <div className="fixed inset-0 z-50">
      {/* Backdrop */}
      <div
        ref={backdropRef}
        className={cn(
          'fixed inset-0 bg-black/40 transition-opacity duration-300',
          log ? 'opacity-100' : 'opacity-0',
        )}
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Panel */}
      <div
        className={cn(
          'fixed right-0 top-0 flex h-full w-full max-w-xl flex-col border-l bg-background shadow-2xl transition-transform duration-300 ease-out',
          log ? 'translate-x-0' : 'translate-x-full',
        )}
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b px-5 py-4">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-semibold">请求详情</p>
            <p className="truncate font-mono text-xs text-muted-foreground">{shortId(log.id)}</p>
          </div>
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="关闭">
            <X className="h-5 w-5" />
          </Button>
        </div>

        {/* Scrollable content */}
        <div className="flex-1 overflow-y-auto p-5">
          <div className="space-y-6">
            {/* Overview badges */}
            <div className="flex flex-wrap gap-2">
              <Badge variant="outline" className={cn('gap-1.5', providerTone(log.provider))}>
                <Server className="h-3 w-3" />
                {log.channelName || log.provider}
              </Badge>
              <Badge variant="outline" className={cn(
                log.status === 'error' ? 'border-rose-200 bg-rose-50 text-rose-700'
                  : log.status === 'timeout' ? 'border-amber-200 bg-amber-50 text-amber-700'
                    : 'border-emerald-200 bg-emerald-50 text-emerald-700',
              )}>
                {log.status}
              </Badge>
              <Badge variant="outline" className="font-mono text-[11px]">
                {log.statusCode}
              </Badge>
              <Badge variant="outline">
                {log.stream === 'stream' ? '流式' : '非流式'}
              </Badge>
              <Badge variant="outline">{protocolLabel(log.protocol)}</Badge>
            </div>

            {/* Time & latency quick view */}
            <div className="grid grid-cols-2 gap-3">
              <div className="rounded-lg border bg-muted/20 p-3">
                <p className="text-xs text-muted-foreground">时间</p>
                <p className="mt-1 font-mono text-sm">
                  {date
                    ? `${date.toLocaleDateString('zh-CN')} ${date.toLocaleTimeString('zh-CN', { hour12: false })}`
                    : log.timestamp}
                </p>
              </div>
              <div className="rounded-lg border bg-muted/20 p-3">
                <p className="text-xs text-muted-foreground">总耗时</p>
                <div className="mt-1 flex items-center gap-2">
                  <p className="font-mono text-sm font-medium">{formatLatency(log.latencyMs)}</p>
                  <div className={cn('h-2 w-2 rounded-full', latencyTone(log.latencyMs))} />
                </div>
              </div>
            </div>

            {/* Detail sections */}
            <DetailSection title="路由与请求" icon={Server}>
              <DetailLine label="请求 ID" value={log.id} copyValue={log.id} mono />
              <DetailLine label="渠道信息" value={`${log.channelId || log.provider} - ${log.channelName || log.provider}`} />
              <DetailLine label="请求路径" value={log.requestPath || '/v1/messages'} copyValue={log.requestPath || '/v1/messages'} mono />
              <DetailLine label="流式模式" value={log.stream === 'stream' ? '流式' : '非流式'} />
              <DetailLine label="日志详情" value={compactDetail(log)} />
            </DetailSection>

            <DetailSection title="身份" icon={Server}>
              <DetailLine label="用户名" value={log.username} />
              <DetailLine label="Token 名" value={log.tokenName || log.apiKeyName || 'legacy'} />
              <DetailLine label="分组" value={log.group || log.apiKeyGroup || 'default'} />
              <DetailLine label="用户 ID" value={log.userId} copyValue={log.userId} mono />
              <DetailLine label="客户端 IP" value={log.clientIp || '-'} copyValue={log.clientIp || undefined} mono />
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
              <DetailLine label="花费" value={formatMoney(log.costEstimate || 0, 6)} mono />
              <DetailLine label="计费模式" value={billingModeLabel(log.billingMode)} />
            </DetailSection>

            {/* Error message */}
            {log.errorMessage && (
              <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4">
                <p className="text-xs font-medium text-destructive">错误信息</p>
                <p className="mt-1 break-words text-sm text-destructive">{log.errorMessage}</p>
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="border-t px-5 py-3">
          <Button variant="outline" className="w-full" onClick={onClose}>
            关闭
          </Button>
        </div>
      </div>
    </div>
  )
}
