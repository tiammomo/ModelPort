import { useEffect, useRef, useState, type ElementType, type ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn, formatLatency } from '@/lib/utils'
import {
  ArrowRight,
  BadgeDollarSign,
  Braces,
  Check,
  Clock3,
  Copy,
  DatabaseZap,
  GitBranch,
  Info,
  Route,
  Server,
  Wrench,
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
      aria-label={copied ? '已复制到剪贴板' : '复制到剪贴板'}
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
          highlight && 'font-medium text-rose-700 dark:text-rose-300',
          danger && 'font-medium text-destructive',
        )}
      >
        <span className="min-w-0 break-words">{value}</span>
        {copyValue && <CopyButton value={copyValue} />}
      </div>
    </div>
  )
}

type Tone = 'blue' | 'emerald' | 'amber' | 'rose' | 'slate' | 'violet'

const toneClasses: Record<Tone, { dot: string; surface: string; text: string; border: string }> = {
  blue: {
    dot: 'bg-blue-500',
    surface: 'bg-blue-50 dark:bg-blue-950/40',
    text: 'text-blue-700 dark:text-blue-300',
    border: 'border-blue-200 dark:border-blue-900',
  },
  emerald: {
    dot: 'bg-emerald-500',
    surface: 'bg-emerald-50 dark:bg-emerald-950/40',
    text: 'text-emerald-700 dark:text-emerald-300',
    border: 'border-emerald-200 dark:border-emerald-900',
  },
  amber: {
    dot: 'bg-amber-500',
    surface: 'bg-amber-50 dark:bg-amber-950/40',
    text: 'text-amber-700 dark:text-amber-300',
    border: 'border-amber-200 dark:border-amber-900',
  },
  rose: {
    dot: 'bg-rose-500',
    surface: 'bg-rose-50 dark:bg-rose-950/40',
    text: 'text-rose-700 dark:text-rose-300',
    border: 'border-rose-200 dark:border-rose-900',
  },
  slate: {
    dot: 'bg-slate-400',
    surface: 'bg-slate-50 dark:bg-slate-900',
    text: 'text-slate-700 dark:text-slate-300',
    border: 'border-slate-200 dark:border-slate-800',
  },
  violet: {
    dot: 'bg-violet-500',
    surface: 'bg-violet-50 dark:bg-violet-950/40',
    text: 'text-violet-700 dark:text-violet-300',
    border: 'border-violet-200 dark:border-violet-900',
  },
}

function statusTone(status: RequestLog['status']): Tone {
  if (status === 'error') return 'rose'
  if (status === 'timeout') return 'amber'
  return 'emerald'
}

function statusBadgeClass(status: RequestLog['status']): string {
  if (status === 'error') return 'border-rose-200 bg-rose-50 text-rose-700 dark:border-rose-900 dark:bg-rose-950/40 dark:text-rose-300'
  if (status === 'timeout') return 'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-300'
  return 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300'
}

function formattedDate(log: RequestLog): string {
  const date = parseLogDate(log.timestamp)
  if (!date) return log.timestamp
  return `${date.toLocaleDateString('zh-CN')} ${date.toLocaleTimeString('zh-CN', { hour12: false })}`
}

function codeSnippet(value: unknown): string {
  return JSON.stringify(value, null, 2)
}

function SignalTile({
  label,
  value,
  icon: Icon,
  tone = 'slate',
}: {
  label: string
  value: string
  icon: ElementType
  tone?: Tone
}) {
  const classes = toneClasses[tone]
  return (
    <div className={cn('rounded-lg border p-3', classes.border, classes.surface)}>
      <div className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        <Icon className={cn('h-3.5 w-3.5', classes.text)} />
        {label}
      </div>
      <p className={cn('mt-2 truncate font-mono text-sm font-semibold', classes.text)}>
        {value}
      </p>
    </div>
  )
}

function TraceStep({
  title,
  detail,
  meta,
  tone,
  last,
}: {
  title: string
  detail: string
  meta: string
  tone: Tone
  last?: boolean
}) {
  const classes = toneClasses[tone]
  return (
    <div className="relative flex gap-3">
      {!last && <div className="absolute left-[15px] top-8 h-[calc(100%+12px)] w-px bg-border" />}
      <div className={cn('relative z-10 flex h-8 w-8 shrink-0 items-center justify-center rounded-full border-4 border-background', classes.dot)}>
        <span className="h-2 w-2 rounded-full bg-white" />
      </div>
      <div className="min-w-0 flex-1 pb-4">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <p className="truncate text-sm font-medium">{title}</p>
          <Badge variant="outline" className={cn('shrink-0', classes.border, classes.surface, classes.text)}>
            {meta}
          </Badge>
        </div>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">{detail}</p>
      </div>
    </div>
  )
}

function JsonPanel({ title, value }: { title: string; value: unknown }) {
  return (
    <div className="min-w-0 rounded-lg border bg-muted/20">
      <div className="flex items-center justify-between border-b px-3 py-2">
        <p className="text-xs font-semibold">{title}</p>
        <Badge variant="outline" className="font-mono text-[10px]">redacted</Badge>
      </div>
      <pre className="max-h-56 overflow-auto p-3 text-[11px] leading-5 text-muted-foreground">
        {codeSnippet(value)}
      </pre>
    </div>
  )
}

function OverviewTab({
  log,
  pricing,
  cost,
}: {
  log: RequestLog
  pricing: RequestLog['modelPricing']
  cost: RequestLog['costBreakdown']
}) {
  return (
    <div className="space-y-6">
      <div className="flex flex-wrap gap-2">
        <Badge variant="outline" className={cn('gap-1.5', providerTone(log.provider))}>
          <Server className="h-3 w-3" />
          {log.channelName || log.provider}
        </Badge>
        <Badge variant="outline" className={statusBadgeClass(log.status)}>
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

      <div className="grid grid-cols-2 gap-3">
        <SignalTile label="时间" value={formattedDate(log)} icon={Clock3} tone="slate" />
        <SignalTile label="总耗时" value={formatLatency(log.latencyMs)} icon={Route} tone={statusTone(log.status)} />
      </div>

      <DetailSection title="路由与请求" icon={Server}>
        <DetailLine label="日志 ID" value={log.id} copyValue={log.id} mono />
        <DetailLine
          label="请求 ID"
          value={log.requestId || '未记录'}
          copyValue={log.requestId || undefined}
          mono
        />
        <DetailLine label="渠道信息" value={`${log.channelId || log.provider} - ${log.channelName || log.provider}`} />
        <DetailLine
          label="请求路径"
          value={log.requestPath || '未记录'}
          copyValue={log.requestPath || undefined}
          mono
        />
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

      {log.errorMessage && (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4">
          <p className="text-xs font-medium text-destructive">错误信息</p>
          <p className="mt-1 break-words text-sm text-destructive">{log.errorMessage}</p>
        </div>
      )}
    </div>
  )
}

function ProtocolTraceTab({ log }: { log: RequestLog }) {
  const inboundProtocol = log.requestPath?.includes('/v1/chat')
    ? 'OpenAI Chat'
    : log.requestPath?.includes('/v1/messages')
      ? 'Anthropic Messages'
      : '协议未记录'
  const providerProtocol = protocolLabel(log.protocol)
  const resolvedModel = log.resolvedModel || log.model
  const routeName = log.channelId ? `${log.channelId}:${resolvedModel}` : resolvedModel

  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3">
        <SignalTile
          label="Request ID"
          value={log.requestId ? shortId(log.requestId) : '未记录'}
          icon={GitBranch}
          tone="blue"
        />
        <SignalTile
          label="首字延迟"
          value={log.firstByteLatencyMs ? formatLatency(log.firstByteLatencyMs) : '未记录'}
          icon={Clock3}
          tone={statusTone(log.status)}
        />
        <SignalTile label="Retry" value={String(log.retryCount || 0)} icon={Route} tone={(log.retryCount || 0) > 0 ? 'amber' : 'slate'} />
        <SignalTile label="Provider" value={log.channelName || log.provider} icon={Server} tone="violet" />
      </div>

      <div className="rounded-lg border p-4">
        <div className="mb-4 flex items-center justify-between gap-3">
          <div>
            <p className="text-sm font-semibold">持久化日志摘要</p>
            <p className="text-xs text-muted-foreground">仅呈现服务端已保存字段，不代表逐阶段执行 Trace 或耗时。</p>
          </div>
          <Badge variant="outline" className={statusBadgeClass(log.status)}>{log.status}</Badge>
        </div>
        <TraceStep
          title="请求已记录"
          detail={`${inboundProtocol} · 请求模型 ${log.model} · ${log.stream === 'stream' ? '流式' : '非流式'}。日志没有保存原始请求体。`}
          meta={log.requestPath || '路径未记录'}
          tone="blue"
        />
        <TraceStep
          title="身份上下文"
          detail={`${log.tokenName || log.apiKeyName || 'Token 未记录'} · ${log.group || log.apiKeyGroup || '标签未记录'} · IP ${log.clientIp || '未记录'}。这些字段不能证明具体认证阶段的结果。`}
          meta={log.userId ? '有记录' : '未记录'}
          tone="slate"
        />
        <TraceStep
          title="路由归因摘要"
          detail={`日志将 ${log.model} 归因为 ${routeName}；无法仅凭当前字段确认每个路由阶段是否执行。`}
          meta={providerProtocol}
          tone="violet"
        />
        <TraceStep
          title="请求结果"
          detail={`状态 ${log.status} · Provider 归因 ${log.channelId || log.provider} · 重试记录 ${log.retryCount || 0} 次 · 总耗时 ${formatLatency(log.latencyMs)}。`}
          meta={String(log.statusCode)}
          tone={statusTone(log.status)}
          last
        />
      </div>

      <div className="grid gap-3 xl:grid-cols-3">
        <JsonPanel
          title="客户端摘要（重构）"
          value={{
            path: log.requestPath || null,
            model: log.model,
            stream: log.stream === 'stream',
            user: log.username,
            api_key_label: log.tokenName || log.apiKeyName || null,
          }}
        />
        <JsonPanel
          title="归一化路由摘要（重构）"
          value={{
            request_id: log.requestId || null,
            resolved_model: resolvedModel,
            provider: log.channelId || log.provider,
            cache: {
              creation: log.cacheWriteTokens || 0,
              read: log.cacheReadTokens || 0,
            },
          }}
        />
        <JsonPanel
          title="Provider 归因摘要（重构）"
          value={{
            protocol: providerProtocol,
            model: resolvedModel,
            stream: log.stream === 'stream',
            retry_count: log.retryCount || 0,
            status_code: log.statusCode,
          }}
        />
      </div>
    </div>
  )
}

function CapabilityPill({
  label,
  active = true,
}: {
  label: string
  active?: boolean
}) {
  return (
    <Badge
      variant="outline"
      className={cn(
        'font-mono text-[11px]',
        active
          ? 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300'
          : 'border-slate-200 bg-slate-50 text-slate-600 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300',
      )}
    >
      {label}
    </Badge>
  )
}

function ToolUseTab({ log }: { log: RequestLog }) {
  const providerMode = log.protocol === 'anthropic' ? 'native Anthropic tool blocks' : 'adapter mapped tool calls'

  return (
    <div className="space-y-5">
      <div className="rounded-lg border border-amber-200 bg-amber-50 p-4 dark:border-amber-900 dark:bg-amber-950/40">
        <div className="flex items-start justify-between gap-4">
          <div>
            <p className="text-sm font-semibold text-amber-950 dark:text-amber-100">本次请求没有工具级遥测</p>
            <p className="mt-1 text-xs leading-5 text-amber-900 dark:text-amber-200">
              当前日志未持久化 tools、tool_choice、tool_use_id 或 tool_result，因而无法证明本次请求是否调用工具或通过了哪些校验。
            </p>
          </div>
          <Badge variant="outline" className="shrink-0 border-amber-300 bg-amber-100 text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200">
            能力参考
          </Badge>
        </div>
        <div className="mt-4 flex flex-wrap gap-2">
          <CapabilityPill label="tools: 未记录" active={false} />
          <CapabilityPill label="tool_choice: 未记录" active={false} />
          <CapabilityPill label="tool_result: 未记录" active={false} />
        </div>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="rounded-lg border bg-muted/20 p-4">
          <div className="flex items-center gap-2">
            <Wrench className="h-4 w-4 text-blue-600" />
            <p className="text-sm font-medium">这条日志可确认</p>
          </div>
          <div className="mt-4 space-y-3">
            <TraceCheck label="Provider 协议" value={`${providerMode}（由日志协议字段推断）`} />
            <TraceCheck label="响应模式" value={log.stream === 'stream' ? '流式' : '非流式'} />
            <TraceCheck label="请求结果" value={`${log.status} · HTTP ${log.statusCode}`} />
          </div>
        </div>

        <div className="rounded-lg border bg-muted/20 p-4">
          <div className="flex items-center gap-2">
            <Braces className="h-4 w-4 text-violet-600" />
            <p className="text-sm font-medium">这条日志无法确认</p>
          </div>
          <div className="mt-4 space-y-3">
            <TraceCheck label="工具定义" value="未保存 tools 与 JSON Schema" />
            <TraceCheck label="工具执行" value="未保存工具名称、参数和结果" />
            <TraceCheck label="校验过程" value="未保存逐阶段校验事件与耗时" />
          </div>
        </div>
      </div>

      <div className="rounded-lg border">
        <div className="border-b px-4 py-3">
          <p className="text-sm font-semibold">协议事件能力参考（非本次 Trace）</p>
          <p className="mt-1 text-xs text-muted-foreground">用于理解网关可能处理的事件；不表示下列事件出现在本次请求中。</p>
        </div>
        <div className="divide-y text-sm">
          {[
            ['message_start', '初始化响应与计量上下文'],
            ['content_block_start', '识别 text / tool_use 内容块'],
            ['input_json_delta', '聚合工具参数片段'],
            ['content_block_stop', '完成 schema 与 tool_use_id 约束'],
            ['message_delta', '映射 usage 与 stop_reason'],
            ['message_stop', '完成日志、成本和错误归一化'],
          ].map(([event, detail]) => (
            <div key={event} className="grid gap-2 px-4 py-3 sm:grid-cols-[150px_1fr]">
              <code className="font-mono text-xs text-blue-700 dark:text-blue-300">{event}</code>
              <span className="text-xs text-muted-foreground">{detail}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

function TraceCheck({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-start gap-2">
      <Info className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0">
        <p className="text-xs font-medium">{label}</p>
        <p className="mt-0.5 text-xs text-muted-foreground">{value}</p>
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
  const panelRef = useRef<HTMLDivElement>(null)
  const closeButtonRef = useRef<HTMLButtonElement>(null)
  const previousFocusRef = useRef<HTMLElement | null>(null)
  const isOpen = Boolean(log)

  // Treat the drawer as a modal: move focus in, trap it, and restore focus on close.
  useEffect(() => {
    if (!isOpen) return
    previousFocusRef.current = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null
    const frameId = window.requestAnimationFrame(() => closeButtonRef.current?.focus())

    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onClose()
        return
      }
      if (e.key !== 'Tab' || !panelRef.current) return

      const focusable = Array.from(panelRef.current.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      )).filter((element) => element.offsetParent !== null)
      if (focusable.length === 0) {
        e.preventDefault()
        panelRef.current.focus()
        return
      }

      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault()
        last.focus()
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault()
        first.focus()
      }
    }
    document.addEventListener('keydown', handleKey)
    return () => {
      window.cancelAnimationFrame(frameId)
      document.removeEventListener('keydown', handleKey)
      previousFocusRef.current?.focus()
    }
  }, [isOpen, onClose])

  // Prevent body scroll when open
  useEffect(() => {
    if (isOpen) {
      document.body.style.overflow = 'hidden'
      return () => {
        document.body.style.overflow = ''
      }
    }
  }, [isOpen])

  if (!log) return null

  const pricing = log.modelPricing
  const cost = log.costBreakdown

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
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="log-detail-title"
        tabIndex={-1}
        className={cn(
          'fixed right-0 top-0 flex h-full w-full max-w-3xl flex-col border-l bg-background shadow-2xl transition-transform duration-300 ease-out',
          log ? 'translate-x-0' : 'translate-x-full',
        )}
      >
        {/* Header */}
        <div className="border-b px-5 py-4">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0 space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline" className={cn('gap-1.5', providerTone(log.provider))}>
                  <Server className="h-3 w-3" />
                  {log.channelName || log.provider}
                </Badge>
                <Badge variant="outline" className={statusBadgeClass(log.status)}>
                  {log.status}
                </Badge>
                <Badge variant="outline" className="font-mono text-[11px]">
                  Request {log.requestId ? shortId(log.requestId) : '未记录'}
                </Badge>
              </div>
              <div>
                <h2 id="log-detail-title" className="text-base font-semibold">请求详情</h2>
                <p className="mt-1 truncate font-mono text-xs text-muted-foreground">
                  {log.requestPath || '路径未记录'} <ArrowRight className="mx-1 inline h-3 w-3" /> {log.resolvedModel || log.model}
                </p>
              </div>
            </div>
            <Button ref={closeButtonRef} variant="ghost" size="icon" onClick={onClose} aria-label="关闭请求详情">
              <X className="h-5 w-5" />
            </Button>
          </div>
        </div>

        {/* Scrollable content */}
        <div className="flex-1 overflow-y-auto p-5">
          <Tabs defaultValue="overview" className="space-y-4">
            <TabsList className="grid w-full grid-cols-3">
              <TabsTrigger value="overview">概览</TabsTrigger>
              <TabsTrigger value="trace">协议 Trace</TabsTrigger>
              <TabsTrigger value="tool-use">Tool Use</TabsTrigger>
            </TabsList>
            <TabsContent value="overview">
              <OverviewTab log={log} pricing={pricing} cost={cost} />
            </TabsContent>
            <TabsContent value="trace">
              <ProtocolTraceTab log={log} />
            </TabsContent>
            <TabsContent value="tool-use">
              <ToolUseTab log={log} />
            </TabsContent>
          </Tabs>
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
