import { useMemo, useState, type FormEvent } from 'react'
import {
  Activity,
  ArrowRight,
  Braces,
  CheckCircle2,
  CircleDollarSign,
  Clock3,
  Copy,
  Database,
  KeyRound,
  Layers3,
  RefreshCw,
  Search,
  Server,
  ShieldCheck,
  SlidersHorizontal,
  TriangleAlert,
  X,
} from 'lucide-react'
import { toast } from 'sonner'
import {
  useAdjustEnterpriseBudget,
  useEnterpriseBudget,
  useEnterpriseOverview,
  useEnterpriseRequest,
  useEnterpriseRequests,
  useUpdateEnterpriseBudget,
} from '@/hooks'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Switch } from '@/components/ui/switch'
import { EmptyState } from '@/components/shared/EmptyState'
import { ErrorState } from '@/components/shared/ErrorState'
import { PaginationBar } from '@/components/shared/PaginationBar'
import { Skeleton } from '@/components/shared/Skeleton'
import { cn } from '@/lib/utils'
import { DEFAULT_ENTERPRISE_BUDGET_SCOPE } from '@/services/enterprise.service'
import type {
  EnterpriseAttempt,
  EnterpriseBudgetAccount,
  EnterpriseBudgetEvent,
  EnterpriseBudgetScope,
  EnterpriseClientProtocol,
  EnterpriseLedgerOverview,
  EnterpriseRequest,
  EnterpriseRequestFilters,
  EnterpriseRequestState,
} from '@/types'

const ALL = 'all'
const PAGE_SIZE_OPTIONS = [20, 50, 100]

export function EnterprisePage() {
  const [filters, setFilters] = useState<EnterpriseRequestFilters>({})
  const [searchDraft, setSearchDraft] = useState('')
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)
  const [selectedLedgerId, setSelectedLedgerId] = useState<string>()
  const [budgetOpen, setBudgetOpen] = useState(false)

  const overviewQuery = useEnterpriseOverview()
  const budgetQuery = useEnterpriseBudget(DEFAULT_ENTERPRISE_BUDGET_SCOPE)
  const requestsQuery = useEnterpriseRequests(filters, page, pageSize)
  const detailQuery = useEnterpriseRequest(selectedLedgerId)
  const requests = requestsQuery.data?.requests ?? []
  const total = requestsQuery.data?.total ?? 0
  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const currentPage = Math.min(page, totalPages)
  const start = total === 0 ? 0 : (currentPage - 1) * pageSize + 1
  const end = Math.min(total, start + requests.length - 1)
  const hasFilters = Object.values(filters).some(Boolean)

  const refreshedAt = useMemo(() => {
    const updatedAt = Math.max(overviewQuery.dataUpdatedAt, requestsQuery.dataUpdatedAt)
    return updatedAt
      ? new Date(updatedAt).toLocaleTimeString('zh-CN', { hour12: false })
      : '—'
  }, [overviewQuery.dataUpdatedAt, requestsQuery.dataUpdatedAt])

  const submitSearch = (event: FormEvent) => {
    event.preventDefault()
    setFilters((current) => ({ ...current, search: searchDraft.trim() || undefined }))
    setPage(1)
  }

  const updateFilter = <K extends keyof EnterpriseRequestFilters>(
    key: K,
    value: EnterpriseRequestFilters[K],
  ) => {
    setFilters((current) => ({ ...current, [key]: value || undefined }))
    setPage(1)
  }

  const clearFilters = () => {
    setFilters({})
    setSearchDraft('')
    setPage(1)
  }

  const refresh = () => {
    void overviewQuery.refetch()
    void budgetQuery.refetch()
    void requestsQuery.refetch()
  }

  if (requestsQuery.isError && !requestsQuery.data) {
    return (
      <ErrorState
        title="企业账本加载失败"
        message={requestsQuery.error instanceof Error ? requestsQuery.error.message : '无法查询企业请求账本。'}
        onRetry={refresh}
      />
    )
  }

  return (
    <div className="space-y-6">
      <header className="flex flex-col gap-4 border-b pb-5 lg:flex-row lg:items-end lg:justify-between">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2 text-xs font-medium text-muted-foreground">
            <span className="uppercase tracking-[0.18em]">Enterprise control plane</span>
            <span className="text-border">/</span>
            <span>请求事实账本</span>
          </div>
          <h1 className="text-3xl font-semibold tracking-[-0.03em]">企业运行</h1>
          <p className="max-w-3xl text-sm leading-6 text-muted-foreground">
            从租户隔离的不可变请求事实中检查幂等、Provider 尝试、租约和计费状态。原始请求体与幂等键不会进入控制台。
          </p>
        </div>
        <div className="flex items-center gap-3 text-xs text-muted-foreground">
          <span>更新于 {refreshedAt}</span>
          <Button variant="outline" size="sm" onClick={refresh} disabled={requestsQuery.isFetching || overviewQuery.isFetching}>
            <RefreshCw className={cn('h-3.5 w-3.5', (requestsQuery.isFetching || overviewQuery.isFetching) && 'animate-spin')} />
            刷新
          </Button>
        </div>
      </header>

      <OverviewRail overview={overviewQuery.data} loading={overviewQuery.isLoading} />

      {overviewQuery.data && <RuntimeStrip overview={overviewQuery.data} />}

      <BudgetRail
        account={budgetQuery.data?.account}
        loading={budgetQuery.isLoading}
        error={budgetQuery.isError}
        onConfigure={() => setBudgetOpen(true)}
      />

      {(overviewQuery.isError || requestsQuery.isError) && (
        <div className="flex items-center gap-2 border-y border-amber-300 bg-amber-50/70 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/30 dark:text-amber-200">
          <TriangleAlert className="h-4 w-4 shrink-0" />
          部分刷新失败，当前保留上一次成功获取的数据。
        </div>
      )}

      <section aria-label="企业账本筛选" className="border-y bg-muted/15 py-3">
        <div className="flex flex-col gap-3 px-1 xl:flex-row xl:items-center">
          <form onSubmit={submitSearch} className="flex min-w-0 flex-1 gap-2">
            <div className="relative min-w-0 flex-1">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={searchDraft}
                onChange={(event) => setSearchDraft(event.target.value)}
                className="h-9 border-0 bg-background pl-9 shadow-none ring-1 ring-border"
                placeholder="搜索 Ledger ID、Request ID、模型、主体或终止原因"
                aria-label="搜索企业账本"
              />
            </div>
            <Button type="submit" size="sm" variant="secondary">查询</Button>
          </form>

          <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:flex">
            <FilterSelect
              label="状态"
              value={filters.state || ALL}
              onValueChange={(value) => updateFilter('state', value === ALL ? undefined : value as EnterpriseRequestState)}
              options={[
                [ALL, '全部状态'],
                ['started', '执行中'],
                ['completed', '已完成'],
                ['failed', '失败'],
                ['cancelled', '已取消'],
              ]}
            />
            <FilterSelect
              label="客户端协议"
              value={filters.protocol || ALL}
              onValueChange={(value) => updateFilter('protocol', value === ALL ? undefined : value as EnterpriseClientProtocol)}
              options={[
                [ALL, '全部协议'],
                ['openai-chat-completions', 'OpenAI Chat'],
                ['anthropic-messages', 'Anthropic'],
              ]}
            />
            <Input
              value={filters.environmentId || ''}
              onChange={(event) => updateFilter('environmentId', event.target.value)}
              className="h-9 xl:w-40"
              placeholder="环境 ID"
              aria-label="按环境 ID 筛选"
            />
          </div>

          {hasFilters && (
            <Button variant="ghost" size="sm" onClick={clearFilters} className="self-start xl:self-auto">
              <X className="h-3.5 w-3.5" />清除
            </Button>
          )}
        </div>
      </section>

      <section aria-label="企业请求记录" className="overflow-hidden border-y">
        <div className="flex items-center justify-between border-b bg-muted/20 px-3 py-2.5">
          <div>
            <h2 className="text-sm font-semibold">Gateway Requests</h2>
            <p className="text-xs text-muted-foreground">按创建时间倒序 · {total.toLocaleString('zh-CN')} 条</p>
          </div>
          {overviewQuery.data?.activeLeases ? (
            <Badge variant="outline" className="gap-1.5 border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-900 dark:bg-blue-950/40 dark:text-blue-300">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-blue-500" />
              {overviewQuery.data.activeLeases} 个活跃租约
            </Badge>
          ) : null}
        </div>

        {requestsQuery.isLoading ? (
          <div className="space-y-px bg-border" aria-label="正在加载企业账本">
            {Array.from({ length: 7 }).map((_, index) => <Skeleton key={index} className="h-16 w-full rounded-none" />)}
          </div>
        ) : requests.length === 0 ? (
          <EmptyState
            icon={Database}
            title={hasFilters ? '没有匹配的账本记录' : '企业账本还没有请求'}
            description={hasFilters ? '清除筛选条件或换一个关联 ID。' : '当请求进入 /v1/messages 或 /v1/chat/completions 后，这里会显示真实生命周期。'}
            action={hasFilters ? <Button variant="outline" onClick={clearFilters}>清除筛选</Button> : undefined}
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="bg-muted/20 hover:bg-muted/20">
                <TableHead className="w-[190px] pl-3">请求</TableHead>
                <TableHead>租户 / 主体</TableHead>
                <TableHead>模型与协议</TableHead>
                <TableHead>状态</TableHead>
                <TableHead className="text-right">用量 / 成本</TableHead>
                <TableHead className="pr-3 text-right">时间</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {requests.map((request) => (
                <EnterpriseRequestRow
                  key={request.ledgerId}
                  request={request}
                  onSelect={() => setSelectedLedgerId(request.ledgerId)}
                />
              ))}
            </TableBody>
          </Table>
        )}

        {total > 0 && (
          <PaginationBar
            className="border-t px-3 py-3"
            total={total}
            page={currentPage}
            pageSize={pageSize}
            totalPages={totalPages}
            start={start}
            end={end}
            pageSizeOptions={PAGE_SIZE_OPTIONS}
            totalLabel="条请求"
            onPageChange={setPage}
            onPageSizeChange={(nextPageSize) => {
              setPageSize(nextPageSize)
              setPage(1)
            }}
          />
        )}
      </section>

      <RequestDetailDrawer
        open={Boolean(selectedLedgerId)}
        onOpenChange={(open) => !open && setSelectedLedgerId(undefined)}
        loading={detailQuery.isLoading}
        error={detailQuery.error}
        detail={detailQuery.data}
        onRetry={() => void detailQuery.refetch()}
      />

      {budgetOpen && (
        <BudgetDialog
          open
          onOpenChange={setBudgetOpen}
          scope={DEFAULT_ENTERPRISE_BUDGET_SCOPE}
          account={budgetQuery.data?.account}
          events={budgetQuery.data?.recentEvents ?? []}
        />
      )}
    </div>
  )
}

function OverviewRail({ overview, loading }: { overview?: EnterpriseLedgerOverview; loading: boolean }) {
  const metrics = [
    { label: '请求总数', value: overview?.totalRequests, detail: `${overview?.startedRequests ?? 0} 执行中`, icon: Activity },
    { label: '完成', value: overview?.completedRequests, detail: `${overview?.failedRequests ?? 0} 失败`, icon: CheckCircle2 },
    { label: '活跃租约', value: overview?.activeLeases, detail: `${overview?.expiredLeases ?? 0} 已过期`, icon: Clock3 },
    { label: '幂等请求', value: overview?.idempotentRequests, detail: '仅暴露存在性', icon: KeyRound },
    { label: '未对账', value: overview?.unreconciledRequests, detail: '需 Provider 证据', icon: TriangleAlert },
    { label: '账本成本', value: overview ? formatMoney(overview.totalCostMicrounits) : undefined, detail: 'USD · 微单位汇总', icon: Layers3 },
  ]
  return (
    <section className="grid overflow-hidden border-y sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6" aria-label="企业账本概览">
      {metrics.map((metric, index) => {
        const Icon = metric.icon
        return (
          <div key={metric.label} className={cn('min-w-0 px-4 py-4', index > 0 && 'border-t sm:border-l sm:border-t-0', index === 2 && 'sm:border-l-0 lg:border-l', index === 3 && 'lg:border-l-0 xl:border-l')}>
            <div className="flex items-center gap-2 text-xs text-muted-foreground"><Icon className="h-3.5 w-3.5" />{metric.label}</div>
            {loading ? <Skeleton className="mt-3 h-7 w-20" /> : <p className="mt-2 text-2xl font-semibold tracking-tight">{metric.value ?? 0}</p>}
            <p className="mt-1 truncate text-[11px] text-muted-foreground">{metric.detail}</p>
          </div>
        )
      })}
    </section>
  )
}

function RuntimeStrip({ overview }: { overview: EnterpriseLedgerOverview }) {
  const relational = overview.backend === 'postgres'
  return (
    <section className="flex flex-col gap-3 border-l-2 border-primary/70 bg-muted/20 px-4 py-3 text-xs lg:flex-row lg:items-center lg:justify-between">
      <div className="flex min-w-0 items-center gap-3">
        <Database className={cn('h-4 w-4 shrink-0', relational ? 'text-emerald-600' : 'text-amber-600')} />
        <div className="min-w-0">
          <span className="font-semibold">{relational ? 'PostgreSQL 关系型账本' : '单实例内存账本'}</span>
          <span className="mx-2 text-border">/</span>
          <span className="font-mono text-muted-foreground">{overview.location}</span>
        </div>
      </div>
      <div className="flex flex-wrap gap-x-5 gap-y-1 text-muted-foreground">
        <span>租约 TTL <b className="font-mono text-foreground">{overview.leaseTtlSecs}s</b></span>
        <span>自动协调 <b className="font-mono text-foreground">{overview.reconcileIntervalSecs}s</b></span>
        <span>范围 <b className="font-mono text-foreground">{overview.organizationCount}/{overview.projectCount}/{overview.environmentCount}</b></span>
      </div>
    </section>
  )
}

function BudgetRail({
  account,
  loading,
  error,
  onConfigure,
}: {
  account?: EnterpriseBudgetAccount
  loading: boolean
  error: boolean
  onConfigure: () => void
}) {
  const percent = Math.min(100, Math.max(0, (account?.utilizationBasisPoints ?? 0) / 100))
  const finite = account?.limitMicrounits != null
  return (
    <section aria-label="企业预算控制" className="border-y">
      <div className="grid lg:grid-cols-[minmax(230px,1.3fr)_repeat(4,minmax(120px,0.7fr))_auto] lg:divide-x">
        <div className="flex min-w-0 items-center gap-3 px-4 py-4">
          <CircleDollarSign className="h-5 w-5 shrink-0 text-primary" />
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h2 className="text-sm font-semibold">事务预算控制</h2>
              <Badge variant="outline" className="h-5 border-emerald-200 bg-emerald-50 px-1.5 text-[10px] text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300">硬门禁</Badge>
            </div>
            <p className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
              {account ? `${account.organizationId}/${account.projectId}/${account.environmentId}` : 'org_local/prj_default/env_default'}
            </p>
          </div>
        </div>
        <BudgetMetric label="预算上限" value={loading ? undefined : finite ? formatMoney(account.limitMicrounits!) : '不限额'} />
        <BudgetMetric label="已预留" value={loading ? undefined : formatMoney(account?.reservedMicrounits ?? 0)} tone={(account?.reservedMicrounits ?? 0) > 0 ? 'text-blue-600 dark:text-blue-400' : undefined} />
        <BudgetMetric label="已结算" value={loading ? undefined : formatMoney(account?.settledMicrounits ?? 0)} />
        <BudgetMetric label="可用" value={loading ? undefined : finite ? formatMoney(account.availableMicrounits ?? 0) : '不限额'} tone={(account?.availableMicrounits ?? 0) < 0 ? 'text-rose-600 dark:text-rose-400' : undefined} />
        <div className="flex items-center justify-between gap-3 border-t px-4 py-3 lg:border-t-0">
          <div className="w-24">
            <div className="mb-1.5 flex justify-between text-[10px] text-muted-foreground"><span>占用</span><span>{finite ? `${percent.toFixed(1)}%` : '∞'}</span></div>
            <div className="h-1.5 overflow-hidden rounded-full bg-muted"><div className={cn('h-full rounded-full', percent >= 90 ? 'bg-rose-500' : percent >= 70 ? 'bg-amber-500' : 'bg-primary')} style={{ width: finite ? `${percent}%` : '0%' }} /></div>
          </div>
          <Button variant="outline" size="sm" onClick={onConfigure}>
            <SlidersHorizontal className="h-3.5 w-3.5" />管理
          </Button>
        </div>
      </div>
      {error && <p className="border-t px-4 py-2 text-xs text-amber-700 dark:text-amber-300">预算状态暂时无法刷新，推理门禁仍由服务端独立执行。</p>}
    </section>
  )
}

function BudgetMetric({ label, value, tone }: { label: string; value?: string; tone?: string }) {
  return (
    <div className="border-t px-4 py-3 lg:border-t-0">
      <p className="text-[11px] text-muted-foreground">{label}</p>
      {value === undefined ? <Skeleton className="mt-2 h-5 w-20" /> : <p className={cn('mt-1 font-mono text-sm font-semibold', tone)}>{value}</p>}
    </div>
  )
}

function BudgetDialog({
  open,
  onOpenChange,
  scope,
  account,
  events,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  scope: EnterpriseBudgetScope
  account?: EnterpriseBudgetAccount
  events: EnterpriseBudgetEvent[]
}) {
  const updateBudget = useUpdateEnterpriseBudget()
  const adjustBudget = useAdjustEnterpriseBudget()
  const [unlimited, setUnlimited] = useState(account?.limitMicrounits == null)
  const [limit, setLimit] = useState(account?.limitMicrounits == null ? '' : microunitsToInput(account.limitMicrounits))
  const [delta, setDelta] = useState('')
  const [reason, setReason] = useState('')
  const [evidenceReference, setEvidenceReference] = useState('')

  const saveLimit = async (event: FormEvent) => {
    event.preventDefault()
    try {
      const limitMicrounits = unlimited ? undefined : parseUsdMicrounits(limit, false)
      await updateBudget.mutateAsync({ ...scope, unlimited, limitMicrounits })
      toast.success(unlimited ? '预算已设为不限额' : '预算硬上限已更新')
    } catch (error) {
      toast.error(error instanceof Error ? error.message : '预算上限更新失败')
    }
  }

  const submitAdjustment = async (event: FormEvent) => {
    event.preventDefault()
    try {
      const deltaMicrounits = parseUsdMicrounits(delta, true)
      if (deltaMicrounits === 0) throw new Error('调整金额不能为 0')
      await adjustBudget.mutateAsync({ ...scope, deltaMicrounits, reason: reason.trim(), evidenceReference: evidenceReference.trim() })
      setDelta('')
      setReason('')
      setEvidenceReference('')
      toast.success('预算调整已写入不可变证据流水')
    } catch (error) {
      toast.error(error instanceof Error ? error.message : '预算调整失败')
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] max-w-3xl overflow-y-auto p-0">
        <DialogHeader className="border-b px-6 py-5 pr-12 text-left">
          <DialogTitle>事务预算与证据</DialogTitle>
          <DialogDescription>{scope.organizationId} / {scope.projectId} / {scope.environmentId} · USD 微单位精确记账</DialogDescription>
        </DialogHeader>

        <form onSubmit={saveLimit} className="grid gap-5 border-b px-6 py-5 sm:grid-cols-[1fr_1fr_auto] sm:items-end">
          <div className="sm:col-span-3">
            <h3 className="text-sm font-semibold">推理硬上限</h3>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">每个 Provider Attempt 在出站前原子预留预算；余额不足时不会调用上游。</p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="budget-limit">预算上限（USD）</Label>
            <Input id="budget-limit" value={limit} onChange={(event) => setLimit(event.target.value)} disabled={unlimited} inputMode="decimal" placeholder="例如 1000.000000" />
          </div>
          <label className="flex h-9 items-center justify-between gap-4 border-b px-1 text-sm">
            不限额
            <Switch checked={unlimited} onCheckedChange={setUnlimited} aria-label="预算不限额" />
          </label>
          <Button type="submit" disabled={updateBudget.isPending}>{updateBudget.isPending ? '保存中…' : '保存上限'}</Button>
        </form>

        <form onSubmit={submitAdjustment} className="grid gap-4 border-b bg-muted/15 px-6 py-5 sm:grid-cols-2">
          <div className="sm:col-span-2">
            <h3 className="text-sm font-semibold">人工账务调整</h3>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">正数补记、负数冲销；每次操作必须留下原因和外部证据引用，不修改历史流水。</p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="budget-delta">调整金额（USD，可正负）</Label>
            <Input id="budget-delta" value={delta} onChange={(event) => setDelta(event.target.value)} inputMode="decimal" placeholder="例如 -2.500000" required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="budget-evidence">证据引用</Label>
            <Input id="budget-evidence" value={evidenceReference} onChange={(event) => setEvidenceReference(event.target.value)} placeholder="账单号、工单号或对象存储 URI" required maxLength={500} />
          </div>
          <div className="space-y-2 sm:col-span-2">
            <Label htmlFor="budget-reason">调整原因</Label>
            <Input id="budget-reason" value={reason} onChange={(event) => setReason(event.target.value)} placeholder="说明为何需要补记或冲销" required maxLength={500} />
          </div>
          <div className="sm:col-span-2 sm:text-right"><Button type="submit" variant="secondary" disabled={adjustBudget.isPending}>{adjustBudget.isPending ? '写入中…' : '写入证据流水'}</Button></div>
        </form>

        <section className="px-6 py-5">
          <div className="flex items-end justify-between gap-3">
            <div><h3 className="text-sm font-semibold">最近证据事件</h3><p className="mt-1 text-xs text-muted-foreground">最多显示 50 条，时间倒序。</p></div>
            <span className="font-mono text-xs text-muted-foreground">v{account?.version ?? 0}</span>
          </div>
          {events.length === 0 ? (
            <p className="mt-5 border-y py-6 text-center text-sm text-muted-foreground">尚无预算事件。</p>
          ) : (
            <div className="mt-4 border-y">
              {events.map((item) => <BudgetEventRow key={item.eventId} event={item} />)}
            </div>
          )}
        </section>
      </DialogContent>
    </Dialog>
  )
}

function BudgetEventRow({ event }: { event: EnterpriseBudgetEvent }) {
  const delta = event.reservedDeltaMicrounits + event.settledDeltaMicrounits
  return (
    <div className="grid gap-2 border-b px-1 py-3 text-xs last:border-b-0 sm:grid-cols-[120px_minmax(0,1fr)_120px] sm:items-center">
      <div><p className="font-medium">{budgetEventLabel(event.eventType)}</p><p className="mt-1 text-[10px] text-muted-foreground">{formatDateTime(event.createdAtMs)}</p></div>
      <div className="min-w-0"><p className="truncate text-muted-foreground">{event.reason || event.evidenceSource}</p><p className="mt-1 truncate font-mono text-[10px] text-muted-foreground">{event.attemptId || event.actorId || event.eventId}</p></div>
      <div className="text-right"><p className={cn('font-mono font-semibold', delta < 0 ? 'text-rose-600 dark:text-rose-400' : delta > 0 ? 'text-emerald-600 dark:text-emerald-400' : undefined)}>{formatSignedMoney(delta)}</p><p className="mt-1 text-[10px] text-muted-foreground">{event.evidenceSource}</p></div>
    </div>
  )
}

function FilterSelect({
  label,
  value,
  options,
  onValueChange,
}: {
  label: string
  value: string
  options: Array<[string, string]>
  onValueChange: (value: string) => void
}) {
  return (
    <Select value={value} onValueChange={onValueChange}>
      <SelectTrigger className="h-9 min-w-36" aria-label={label}><SelectValue /></SelectTrigger>
      <SelectContent>
        {options.map(([option, optionLabel]) => <SelectItem key={option} value={option}>{optionLabel}</SelectItem>)}
      </SelectContent>
    </Select>
  )
}

function EnterpriseRequestRow({ request, onSelect }: { request: EnterpriseRequest; onSelect: () => void }) {
  const totalTokens = request.inputTokens + request.outputTokens + request.cacheWriteTokens + request.cacheReadTokens
  return (
    <TableRow className="group cursor-pointer" onClick={onSelect}>
      <TableCell className="pl-3">
        <button type="button" className="block max-w-[180px] text-left" onClick={onSelect}>
          <span className="block truncate font-mono text-xs font-medium text-primary">{shortId(request.ledgerId)}</span>
          <span className="mt-1 block truncate font-mono text-[11px] text-muted-foreground">{shortId(request.requestId)}</span>
        </button>
      </TableCell>
      <TableCell>
        <p className="max-w-52 truncate text-xs font-medium">{request.organizationId} / {request.projectId}</p>
        <p className="mt-1 max-w-52 truncate text-[11px] text-muted-foreground">{request.environmentId} · {request.principalId}</p>
      </TableCell>
      <TableCell>
        <p className="max-w-48 truncate text-xs font-medium">{request.requestedModel}</p>
        <p className="mt-1 text-[11px] text-muted-foreground">{protocolLabel(request.clientProtocol)} · {request.stream ? 'SSE' : 'JSON'}</p>
      </TableCell>
      <TableCell>
        <div className="flex flex-wrap items-center gap-1.5">
          <StateBadge state={request.state} />
          {request.hasIdempotencyKey && <Badge variant="outline" className="h-5 px-1.5 text-[10px]">幂等</Badge>}
        </div>
        <p className="mt-1 text-[11px] text-muted-foreground">{leaseLabel(request)} · {request.attemptCount} attempts</p>
      </TableCell>
      <TableCell className="text-right">
        <p className="font-mono text-xs">{totalTokens.toLocaleString('zh-CN')} tok</p>
        <p className="mt-1 font-mono text-[11px] text-muted-foreground">{formatMoney(request.costAmountMicrounits)}</p>
      </TableCell>
      <TableCell className="pr-3 text-right">
        <p className="whitespace-nowrap text-xs">{formatTime(request.createdAtMs)}</p>
        <p className="mt-1 text-[11px] text-muted-foreground">{durationLabel(request)}</p>
      </TableCell>
    </TableRow>
  )
}

function RequestDetailDrawer({
  open,
  onOpenChange,
  loading,
  error,
  detail,
  onRetry,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  loading: boolean
  error: unknown
  detail?: { request: EnterpriseRequest; attempts: EnterpriseAttempt[] }
  onRetry: () => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="left-auto right-0 top-0 h-screen max-w-2xl translate-x-0 translate-y-0 content-start gap-0 overflow-y-auto rounded-none border-y-0 border-r-0 p-0 [&>button]:z-20 sm:rounded-none">
        <DialogHeader className="sticky top-0 z-10 border-b bg-background/95 px-6 py-5 pr-14 text-left backdrop-blur">
          <DialogTitle className="text-xl">请求事实</DialogTitle>
          <DialogDescription>Gateway Request 与每次 Provider Attempt 的持久化终态。</DialogDescription>
        </DialogHeader>
        {loading ? (
          <div className="space-y-4 p-6"><Skeleton className="h-24 w-full" /><Skeleton className="h-52 w-full" /><Skeleton className="h-40 w-full" /></div>
        ) : error ? (
          <div className="p-6"><ErrorState title="请求详情加载失败" message={error instanceof Error ? error.message : '无法读取请求详情。'} onRetry={onRetry} /></div>
        ) : detail ? (
          <RequestDetailContent request={detail.request} attempts={detail.attempts} />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}

function RequestDetailContent({ request, attempts }: { request: EnterpriseRequest; attempts: EnterpriseAttempt[] }) {
  return (
    <div>
      <div className="flex flex-wrap items-center gap-2 border-b px-6 py-4">
        <StateBadge state={request.state} />
        <Badge variant="outline">{protocolLabel(request.clientProtocol)}</Badge>
        <Badge variant="outline">{request.stream ? '流式 SSE' : '非流式 JSON'}</Badge>
        {request.hasIdempotencyKey && <Badge variant="outline" className="gap-1"><KeyRound className="h-3 w-3" />幂等保护</Badge>}
      </div>

      <DetailSection title="关联标识" icon={Braces}>
        <DetailRow label="Ledger ID" value={request.ledgerId} copy />
        <DetailRow label="Request ID" value={request.requestId} copy />
        <DetailRow label="Principal" value={request.principalId} copy />
        <DetailRow label="租户范围" value={`${request.organizationId} / ${request.projectId} / ${request.environmentId}`} />
      </DetailSection>

      <DetailSection title="生命周期" icon={Clock3}>
        <DetailRow label="请求模型" value={request.requestedModel} />
        <DetailRow label="状态 / HTTP" value={`${stateLabel(request.state)} / ${request.statusCode ?? '—'}`} />
        <DetailRow label="终止原因" value={request.terminalReason || '执行中'} />
        <DetailRow label="计费模式" value={`${request.billingMode || '未结算'} · ${request.chargeable ? '可计费' : '不可计费'}`} />
        <DetailRow label="创建时间" value={formatDateTime(request.createdAtMs)} />
        <DetailRow label="完成时间" value={request.completedAtMs ? formatDateTime(request.completedAtMs) : '—'} />
        <DetailRow label="租约" value={`${leaseLabel(request)} · ${shortId(request.leaseOwner)} · ${formatDateTime(request.leaseExpiresAtMs)}`} />
      </DetailSection>

      <DetailSection title="用量与成本" icon={Layers3}>
        <div className="grid grid-cols-2 gap-px overflow-hidden border bg-border sm:grid-cols-4">
          <UsageCell label="输入" value={request.inputTokens} />
          <UsageCell label="输出" value={request.outputTokens} />
          <UsageCell label="缓存创建" value={request.cacheWriteTokens} />
          <UsageCell label="缓存读取" value={request.cacheReadTokens} />
        </div>
        <div className="mt-3 flex items-center justify-between border-b pb-3 text-sm">
          <span className="text-muted-foreground">账本成本</span>
          <span className="font-mono font-semibold">{formatMoney(request.costAmountMicrounits)} {request.currency}</span>
        </div>
      </DetailSection>

      <DetailSection title={`Provider Attempts · ${attempts.length}`} icon={Server}>
        {attempts.length === 0 ? (
          <p className="border-l-2 border-muted pl-4 text-sm text-muted-foreground">请求尚未进入 Provider egress。</p>
        ) : (
          <div className="relative space-y-0 before:absolute before:bottom-5 before:left-[7px] before:top-5 before:w-px before:bg-border">
            {attempts.map((attempt, index) => (
              <div key={attempt.attemptId} className="relative flex gap-4 py-4 first:pt-0">
                <span className={cn('relative z-[1] mt-1 h-[15px] w-[15px] shrink-0 rounded-full border-4 border-background', attempt.state === 'completed' ? 'bg-emerald-500' : attempt.state === 'started' ? 'bg-blue-500' : 'bg-rose-500')} />
                <div className="min-w-0 flex-1 border-b pb-4">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div>
                      <p className="text-sm font-semibold">{attempt.providerId} <ArrowRight className="mx-1 inline h-3 w-3" /> {attempt.resolvedModel}</p>
                      <p className="mt-1 font-mono text-[11px] text-muted-foreground">#{index + 1} · {attempt.attemptId}</p>
                    </div>
                    <StateBadge state={attempt.state} />
                  </div>
                  <div className="mt-3 grid gap-2 text-xs text-muted-foreground sm:grid-cols-2">
                    <span>协议 <b className="font-medium text-foreground">{attempt.providerProtocol}</b></span>
                    <span>HTTP <b className="font-mono font-medium text-foreground">{attempt.statusCode ?? '—'}</b></span>
                    <span>Tokens <b className="font-mono font-medium text-foreground">{(attempt.inputTokens + attempt.outputTokens + attempt.cacheWriteTokens + attempt.cacheReadTokens).toLocaleString('zh-CN')}</b></span>
                    <span>成本 <b className="font-mono font-medium text-foreground">{formatMoney(attempt.costAmountMicrounits)}</b></span>
                  </div>
                  {(attempt.terminalReason || attempt.errorMessage) && (
                    <p className="mt-3 break-words border-l-2 border-muted pl-3 text-xs text-muted-foreground">{attempt.terminalReason}{attempt.errorMessage ? ` · ${attempt.errorMessage}` : ''}</p>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </DetailSection>

      {request.errorMessage && (
        <div className="mx-6 mb-8 border-l-2 border-destructive bg-destructive/5 px-4 py-3 text-sm text-destructive">
          <p className="font-semibold">终态错误</p>
          <p className="mt-1 break-words text-xs leading-5">{request.errorMessage}</p>
        </div>
      )}
    </div>
  )
}

function DetailSection({ title, icon: Icon, children }: { title: string; icon: typeof ShieldCheck; children: React.ReactNode }) {
  return (
    <section className="border-b px-6 py-5">
      <h3 className="mb-4 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground"><Icon className="h-3.5 w-3.5" />{title}</h3>
      {children}
    </section>
  )
}

function DetailRow({ label, value, copy = false }: { label: string; value: string; copy?: boolean }) {
  const copyValue = async () => {
    await navigator.clipboard.writeText(value)
    toast.success(`${label} 已复制`)
  }
  return (
    <div className="grid grid-cols-[110px_minmax(0,1fr)_28px] items-start gap-3 border-b py-2.5 text-sm last:border-b-0">
      <span className="text-muted-foreground">{label}</span>
      <span className={cn('break-all', copy && 'font-mono text-xs')}>{value}</span>
      {copy ? <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => void copyValue()} aria-label={`复制${label}`}><Copy className="h-3.5 w-3.5" /></Button> : <span />}
    </div>
  )
}

function UsageCell({ label, value }: { label: string; value: number }) {
  return <div className="bg-background px-3 py-3"><p className="text-[11px] text-muted-foreground">{label}</p><p className="mt-1 font-mono text-sm font-semibold">{value.toLocaleString('zh-CN')}</p></div>
}

function StateBadge({ state }: { state: EnterpriseRequestState }) {
  return <Badge variant="outline" className={cn('h-5 px-1.5 text-[10px]', stateTone(state))}>{stateLabel(state)}</Badge>
}

function stateLabel(state: EnterpriseRequestState) {
  return ({ started: '执行中', completed: '已完成', failed: '失败', cancelled: '已取消' })[state]
}

function stateTone(state: EnterpriseRequestState) {
  if (state === 'completed') return 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300'
  if (state === 'started') return 'border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-900 dark:bg-blue-950/40 dark:text-blue-300'
  if (state === 'cancelled') return 'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-300'
  return 'border-rose-200 bg-rose-50 text-rose-700 dark:border-rose-900 dark:bg-rose-950/40 dark:text-rose-300'
}

function protocolLabel(protocol: EnterpriseClientProtocol) {
  return protocol === 'openai-chat-completions' ? 'OpenAI Chat' : 'Anthropic Messages'
}

function leaseLabel(request: EnterpriseRequest) {
  if (request.state !== 'started') return '租约已释放'
  return request.leaseExpiresAtMs > Date.now() ? '租约活跃' : '租约已过期'
}

function durationLabel(request: EnterpriseRequest) {
  const end = request.completedAtMs || Date.now()
  const duration = Math.max(0, end - request.createdAtMs)
  if (duration < 1000) return `${duration}ms`
  if (duration < 60_000) return `${(duration / 1000).toFixed(1)}s`
  return `${(duration / 60_000).toFixed(1)}min`
}

function shortId(value: string) {
  if (value.length <= 22) return value
  return `${value.slice(0, 12)}…${value.slice(-6)}`
}

function formatTime(timestamp: number) {
  return new Date(timestamp).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false })
}

function formatDateTime(timestamp: number) {
  return new Date(timestamp).toLocaleString('zh-CN', { hour12: false })
}

function formatMoney(microunits: number) {
  return `$${(microunits / 1_000_000).toFixed(6)}`
}

function formatSignedMoney(microunits: number) {
  if (microunits === 0) return '$0.000000'
  return `${microunits > 0 ? '+' : '-'}${formatMoney(Math.abs(microunits))}`
}

function microunitsToInput(microunits: number) {
  const dollars = Math.floor(microunits / 1_000_000)
  const fraction = String(microunits % 1_000_000).padStart(6, '0')
  return `${dollars}.${fraction}`
}

function parseUsdMicrounits(value: string, signed: boolean) {
  const normalized = value.trim()
  const pattern = signed ? /^([+-]?)(\d+)(?:\.(\d{1,6}))?$/ : /^(\d+)(?:\.(\d{1,6}))?$/
  const match = normalized.match(pattern)
  if (!match) throw new Error('请输入有效的 USD 金额，最多保留 6 位小数')
  const sign = signed && match[1] === '-' ? -1n : 1n
  const wholeIndex = signed ? 2 : 1
  const fractionIndex = signed ? 3 : 2
  const amount = sign * (BigInt(match[wholeIndex]) * 1_000_000n + BigInt((match[fractionIndex] || '').padEnd(6, '0')))
  if (amount > BigInt(Number.MAX_SAFE_INTEGER) || amount < BigInt(Number.MIN_SAFE_INTEGER)) {
    throw new Error('金额超出控制台可安全处理的范围')
  }
  return Number(amount)
}

function budgetEventLabel(eventType: EnterpriseBudgetEvent['eventType']) {
  return ({
    reservation_created: '预算预留',
    settled: '用量结算',
    released: '超时释放',
    adjustment: '人工调整',
  })[eventType]
}
