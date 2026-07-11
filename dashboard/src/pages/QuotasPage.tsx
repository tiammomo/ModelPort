import { useMemo, useState } from 'react'
import { useQuotas, useCreateQuota, useDeleteQuota, useUpdateQuota, useUsers } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { MetricCard } from '@/components/shared/MetricCard'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { EmptyState } from '@/components/shared/EmptyState'
import { ConfirmDialog } from '@/components/shared/ConfirmDialog'
import { PaginationBar } from '@/components/shared/PaginationBar'
import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Progress } from '@/components/ui/progress'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { AlertTriangle, Ban, Gauge, Pencil, Plus, RotateCw, Search, Trash2, UsersRound, X } from 'lucide-react'
import { cn, formatDate, formatNumber } from '@/lib/utils'
import { paginateItems } from '@/lib/pagination'
import { resolveQuotaUser } from '@/features/quotas/quota-user'
import {
  filterQuotas,
  isQuotaFilterActive,
  isQuotaLimitValid,
  quotaPeriodRange,
  quotaRisk,
  quotaUsagePercent,
  type QuotaRiskFilter,
} from '@/features/quotas/quota-view'
import { toast } from 'sonner'
import type { Quota, QuotaPeriod, QuotaType } from '@/types'

type QuotaTypeFilter = 'all' | QuotaType
type QuotaPeriodFilter = 'all' | QuotaPeriod

export function QuotasPage() {
  const { data: quotas = [], isLoading, isError, error, refetch } = useQuotas()
  const { data: users = [], isLoading: usersLoading, isError: usersError, error: usersQueryError, refetch: refetchUsers } = useUsers()
  const createQuota = useCreateQuota()
  const updateQuota = useUpdateQuota()
  const deleteQuota = useDeleteQuota()

  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [editingQuota, setEditingQuota] = useState<Quota | null>(null)
  const [confirmZeroQuota, setConfirmZeroQuota] = useState(false)
  const [quotaToDelete, setQuotaToDelete] = useState<Quota | null>(null)
  const [search, setSearch] = useState('')
  const [typeFilter, setTypeFilter] = useState<QuotaTypeFilter>('all')
  const [periodFilter, setPeriodFilter] = useState<QuotaPeriodFilter>('all')
  const [riskFilter, setRiskFilter] = useState<QuotaRiskFilter>('all')
  const [quotaPage, setQuotaPage] = useState(1)
  const [quotaPageSize, setQuotaPageSize] = useState(20)
  const [form, setForm] = useState({
    userId: '',
    username: '',
    quotaType: 'tokens' as QuotaType,
    limit: '',
    period: 'monthly' as QuotaPeriod,
  })

  const filteredQuotas = useMemo(() => filterQuotas(quotas, {
    search,
    quotaType: typeFilter,
    period: periodFilter,
    risk: riskFilter,
  }), [periodFilter, quotas, riskFilter, search, typeFilter])
  const quotaWindow = useMemo(
    () => paginateItems(filteredQuotas, quotaPage, quotaPageSize),
    [filteredQuotas, quotaPage, quotaPageSize],
  )
  const affectedUsers = new Set(quotas.map((quota) => quota.userId)).size
  const warningQuotas = quotas.filter((quota) => quotaRisk(quota.used, quota.limit) === 'warning').length
  const exhaustedQuotas = quotas.filter((quota) => quotaRisk(quota.used, quota.limit) === 'exhausted').length
  const filtersActive = isQuotaFilterActive({ search, quotaType: typeFilter, period: periodFilter, risk: riskFilter })
  const parsedLimit = Number(form.limit)
  const formValid = Boolean(form.userId && form.limit.trim()) && isQuotaLimitValid(parsedLimit, form.quotaType)
  const quotaMutationPending = createQuota.isPending || updateQuota.isPending

  const resetFilters = () => {
    setSearch('')
    setTypeFilter('all')
    setPeriodFilter('all')
    setRiskFilter('all')
    setQuotaPage(1)
  }

  const openCreateQuotaDialog = () => {
    createQuota.reset()
    setEditingQuota(null)
    setForm({ userId: '', username: '', quotaType: 'tokens', limit: '', period: 'monthly' })
    setShowCreateDialog(true)
  }

  const openEditQuotaDialog = (quota: Quota) => {
    updateQuota.reset()
    setShowCreateDialog(false)
    setEditingQuota(quota)
    setForm({
      userId: quota.userId,
      username: quota.username,
      quotaType: quota.quotaType,
      limit: String(quota.limit),
      period: quota.period,
    })
  }

  const saveQuotaRule = () => {
    if (!formValid) return
    const selectedUser = resolveQuotaUser(users, form.userId)
    if (!selectedUser) {
      toast.error('所选用户已不存在，请重新选择')
      return
    }

    const quotaInput = {
      ...selectedUser,
      quotaType: form.quotaType,
      limit: parsedLimit,
      period: form.period,
    }
    const callbacks = {
      onSuccess: () => {
        setConfirmZeroQuota(false)
        setShowCreateDialog(false)
        setEditingQuota(null)
        setForm((current) => ({ ...current, userId: '', username: '', limit: '' }))
        toast.success(editingQuota ? '配额限额已更新' : '用户配额已创建')
      },
      onError: (mutationError: unknown) => toast.error(errorMessage(mutationError) || (editingQuota ? '更新配额失败' : '创建配额失败')),
    }

    if (editingQuota) {
      updateQuota.mutate({ id: editingQuota.id, data: quotaInput }, callbacks)
    } else {
      createQuota.mutate({
        ...quotaInput,
        used: 0,
        ...quotaPeriodRange(form.period),
      }, callbacks)
    }
  }

  const handleCreateQuota = () => {
    if (!formValid) return
    if (parsedLimit === 0) {
      setConfirmZeroQuota(true)
      return
    }
    saveQuotaRule()
  }

  const handleDeleteQuota = () => {
    if (!quotaToDelete) return
    deleteQuota.mutate(quotaToDelete.id, {
      onSuccess: () => toast.success('配额规则已删除'),
      onSettled: () => setQuotaToDelete(null),
      onError: (mutationError) => toast.error(errorMessage(mutationError) || '删除配额失败'),
    })
  }

  if (isLoading || usersLoading) return <LoadingPage />

  return (
    <div className="space-y-6">
      <PageHeader
        title="配额管理"
        description="按用户限制 Token、请求数或费用；周期均按 UTC 自然边界重置"
        action={{ label: '新建配额', onClick: openCreateQuotaDialog, icon: Plus }}
      />

      {isError && <ErrorNotice message={`配额加载失败：${errorMessage(error)}`} actionLabel="重新加载" onAction={() => void refetch()} />}
      {usersError && <ErrorNotice message={`用户列表加载失败：${errorMessage(usersQueryError)}`} actionLabel="重试" onAction={() => void refetchUsers()} />}
      {deleteQuota.error && <ErrorNotice message={errorMessage(deleteQuota.error)} />}

      <div className="grid grid-cols-2 gap-3 xl:grid-cols-4" aria-label="配额概览">
        <MetricCard title="配额规则" value={quotas.length} icon={Gauge} description="不同单位不会混合相加" />
        <MetricCard title="受限用户" value={affectedUsers} icon={UsersRound} description={`共 ${formatNumber(users.length)} 个系统用户`} />
        <MetricCard title="需要关注" value={warningQuotas} icon={AlertTriangle} description="使用率已达 80%" className={warningQuotas > 0 ? 'border-amber-300 dark:border-amber-900' : undefined} />
        <MetricCard title="已阻止调用" value={exhaustedQuotas} icon={Ban} description="额度为 0 或使用率已达 100%" className={exhaustedQuotas > 0 ? 'border-red-300 dark:border-red-900' : undefined} />
      </div>

      <Card className="overflow-hidden">
        <CardHeader className="border-b bg-muted/20 pb-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <CardTitle className="text-base">用户配额</CardTitle>
              <p className="mt-1 text-xs text-muted-foreground">0 表示完全阻止该类型的用量；删除规则表示不再单独限制。</p>
            </div>
            <Button variant="outline" size="icon" onClick={() => void refetch()} aria-label="刷新配额">
              <RotateCw className="h-4 w-4" />
            </Button>
          </div>
          <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-[minmax(240px,1fr)_150px_150px_170px_auto]">
            <div className="relative">
              <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                className="bg-background pl-9"
                aria-label="搜索配额用户"
                placeholder="搜索用户名或用户 ID"
                value={search}
                onChange={(event) => { setSearch(event.target.value); setQuotaPage(1) }}
              />
            </div>
            <FilterSelect label="按类型筛选" value={typeFilter} onChange={(value) => { setTypeFilter(value as QuotaTypeFilter); setQuotaPage(1) }} items={[
              ['all', '全部类型'], ['tokens', 'Token 数'], ['requests', '请求数'], ['cost', '费用'],
            ]} />
            <FilterSelect label="按周期筛选" value={periodFilter} onChange={(value) => { setPeriodFilter(value as QuotaPeriodFilter); setQuotaPage(1) }} items={[
              ['all', '全部周期'], ['daily', '每日'], ['weekly', '每周'], ['monthly', '每月'],
            ]} />
            <FilterSelect label="按使用风险筛选" value={riskFilter} onChange={(value) => { setRiskFilter(value as QuotaRiskFilter); setQuotaPage(1) }} items={[
              ['all', '全部使用状态'], ['healthy', '正常（< 80%）'], ['warning', '需关注（≥ 80%）'], ['exhausted', '已阻止（0 或 ≥ 100%）'],
            ]} />
            {filtersActive && <Button variant="ghost" className="justify-start" onClick={resetFilters}><X className="mr-2 h-4 w-4" />清除</Button>}
          </div>
          <p className="mt-2 text-xs text-muted-foreground" aria-live="polite">显示 {formatNumber(filteredQuotas.length)} / {formatNumber(quotas.length)} 条规则</p>
        </CardHeader>

        <CardContent className="p-0">
          {filteredQuotas.length === 0 ? (
            <EmptyState
              icon={Gauge}
              title={isError ? '无法加载配额' : filtersActive ? '没有匹配的配额规则' : '暂无配额规则'}
              description={isError ? '检查网络或服务状态后重新加载。' : filtersActive ? '调整筛选条件后重试。' : '创建规则，为用户设置明确的用量边界。'}
              action={isError
                ? <Button variant="outline" onClick={() => void refetch()}>重新加载</Button>
                : filtersActive
                ? <Button variant="outline" onClick={resetFilters}>清除筛选</Button>
                : <Button onClick={openCreateQuotaDialog} disabled={users.length === 0}>新建配额</Button>}
            />
          ) : (
            <>
              <div className="hidden md:block">
                <Table className="min-w-[980px]">
                  <TableHeader>
                    <TableRow>
                      <TableHead>用户</TableHead>
                      <TableHead>类型</TableHead>
                      <TableHead className="text-right">已用 / 限额</TableHead>
                      <TableHead className="w-56">使用率</TableHead>
                      <TableHead>周期</TableHead>
                      <TableHead>下次重置</TableHead>
                      <TableHead className="w-20"><span className="sr-only">操作</span></TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {quotaWindow.items.map((quota) => <QuotaTableRow key={quota.id} quota={quota} onEdit={openEditQuotaDialog} onDelete={setQuotaToDelete} />)}
                  </TableBody>
                </Table>
              </div>
              <div className="divide-y md:hidden">
                {quotaWindow.items.map((quota) => <QuotaMobileCard key={quota.id} quota={quota} onEdit={openEditQuotaDialog} onDelete={setQuotaToDelete} />)}
              </div>
            </>
          )}
        </CardContent>
        {filteredQuotas.length > 0 && (
          <CardFooter className="border-t px-4 py-3">
            <PaginationBar
              total={filteredQuotas.length}
              page={quotaWindow.currentPage}
              pageSize={quotaPageSize}
              totalPages={quotaWindow.totalPages}
              start={quotaWindow.start}
              end={quotaWindow.end}
              totalLabel="条配额"
              onPageChange={(page) => setQuotaPage(Math.min(Math.max(page, 1), quotaWindow.totalPages))}
              onPageSizeChange={(pageSize) => { setQuotaPageSize(pageSize); setQuotaPage(1) }}
            />
          </CardFooter>
        )}
      </Card>

      <Dialog
        open={showCreateDialog || !!editingQuota}
        onOpenChange={(open) => {
          if (!open) {
            setShowCreateDialog(false)
            setEditingQuota(null)
            setConfirmZeroQuota(false)
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editingQuota ? '调整配额限额' : '新建配额'}</DialogTitle>
            <DialogDescription>{editingQuota ? '为保证用量统计口径一致，编辑时只调整限额。' : '一条规则只限制一个用户、一个用量类型和一个 UTC 周期。'}</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {(createQuota.error || updateQuota.error) && <ErrorNotice message={errorMessage(createQuota.error || updateQuota.error)} />}
            <div className="space-y-2">
              <Label htmlFor="quota-user">用户 <span className="text-destructive" aria-hidden="true">*</span></Label>
              <Select
                disabled={!!editingQuota}
                value={form.userId}
                onValueChange={(userId) => {
                  const selectedUser = resolveQuotaUser(users, userId)
                  if (selectedUser) setForm({ ...form, ...selectedUser })
                }}
              >
                <SelectTrigger id="quota-user" aria-required="true"><SelectValue placeholder="选择真实用户" /></SelectTrigger>
                <SelectContent>
                  {users.map((user) => (
                    <SelectItem key={user.id} value={user.id}>
                      {user.username} · {user.email}{user.status !== 'active' ? '（不可用）' : ''}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {users.length === 0 && <p className="text-xs text-muted-foreground">暂无用户。先创建用户，再配置配额。</p>}
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="quota-type">配额类型</Label>
                <Select disabled={!!editingQuota} value={form.quotaType} onValueChange={(value) => setForm({ ...form, quotaType: value as QuotaType, limit: '' })}>
                  <SelectTrigger id="quota-type"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="tokens">Token 数</SelectItem>
                    <SelectItem value="requests">请求数</SelectItem>
                    <SelectItem value="cost">费用（USD）</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label htmlFor="quota-period">重置周期</Label>
                <Select disabled={!!editingQuota} value={form.period} onValueChange={(value) => setForm({ ...form, period: value as QuotaPeriod })}>
                  <SelectTrigger id="quota-period" aria-describedby="quota-period-help"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="daily">每日</SelectItem>
                    <SelectItem value="weekly">每周</SelectItem>
                    <SelectItem value="monthly">每月</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="quota-limit">限额 <span className="text-destructive" aria-hidden="true">*</span></Label>
              <Input
                id="quota-limit"
                required
                type="number"
                inputMode={form.quotaType === 'cost' ? 'decimal' : 'numeric'}
                min="0"
                step={form.quotaType === 'cost' ? '0.01' : '1'}
                value={form.limit}
                onChange={(event) => setForm({ ...form, limit: event.target.value })}
                placeholder={form.quotaType === 'cost' ? '例如：25.00' : '例如：100000'}
                aria-describedby="quota-limit-help"
              />
              <p id="quota-limit-help" className="text-xs leading-relaxed text-muted-foreground">
                {form.quotaType === 'cost' ? '单位为 USD，可输入小数。' : '请输入非负整数。'}设为 0 会阻止该类型的全部新用量。
              </p>
              {form.limit.trim() && !isQuotaLimitValid(parsedLimit, form.quotaType) && (
                <p role="alert" className="text-xs text-destructive">请输入有效的{form.quotaType === 'cost' ? '非负金额' : '非负整数'}。</p>
              )}
              {form.limit.trim() && parsedLimit === 0 && (
                <div role="alert" className="rounded-md border border-red-300 bg-red-50 px-3 py-2 text-xs leading-relaxed text-red-900 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200">
                  这是阻断规则：该用户的新{quotaTypeLabels[form.quotaType]}用量会立即被拒绝，而不是“不设置限额”。
                </div>
              )}
            </div>
            <div id="quota-period-help" className="rounded-md bg-muted p-3 text-xs leading-relaxed text-muted-foreground">
              {periodDescription(form.period)}。创建后，服务端会以当前 UTC 周期为准设置开始与重置时间。
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => { setShowCreateDialog(false); setEditingQuota(null) }}>取消</Button>
            <Button onClick={handleCreateQuota} disabled={!formValid || quotaMutationPending}>
              {quotaMutationPending ? '保存中…' : parsedLimit === 0 && form.limit.trim() ? '保存阻断规则' : editingQuota ? '保存限额' : '创建配额'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={confirmZeroQuota}
        title={editingQuota ? '确认将限额调整为 0？' : '确认阻止该用户的调用？'}
        description={`限额 0 会立即阻止 ${form.username || '所选用户'} 的${periodLabels[form.period]}${quotaTypeLabels[form.quotaType]}新用量。${editingQuota ? '取消可保留原限额。' : '若不想单独限制，请取消并关闭创建窗口。'}`}
        confirmLabel="确认阻止"
        destructive
        pending={quotaMutationPending}
        onCancel={() => setConfirmZeroQuota(false)}
        onConfirm={saveQuotaRule}
      />

      <ConfirmDialog
        open={!!quotaToDelete}
        title={`删除 ${quotaToDelete?.username || ''} 的配额？`}
        description={quotaToDelete
          ? `将移除其${periodLabels[quotaToDelete.period]}${quotaTypeLabels[quotaToDelete.quotaType]}限制；删除后该类型不再受这条规则约束。历史用量不会删除。`
          : ''}
        confirmLabel="删除规则"
        destructive
        pending={deleteQuota.isPending}
        onCancel={() => setQuotaToDelete(null)}
        onConfirm={handleDeleteQuota}
      />
    </div>
  )
}

function QuotaTableRow({ quota, onEdit, onDelete }: { quota: Quota; onEdit: (quota: Quota) => void; onDelete: (quota: Quota) => void }) {
  return (
    <TableRow>
      <TableCell>
        <p className="font-medium">{quota.username}</p>
        <p className="text-xs text-muted-foreground">{quota.userId}</p>
      </TableCell>
      <TableCell><Badge variant="outline">{quotaTypeLabels[quota.quotaType]}</Badge></TableCell>
      <TableCell className="text-right font-medium tabular-nums">{formatQuotaValue(quota.used, quota.quotaType)} <span className="text-muted-foreground">/ {formatQuotaValue(quota.limit, quota.quotaType)}</span></TableCell>
      <TableCell><QuotaProgress quota={quota} /></TableCell>
      <TableCell>{periodLabels[quota.period]}</TableCell>
      <TableCell className="text-sm text-muted-foreground">{formatDate(quota.resetAt)}</TableCell>
      <TableCell>
        <div className="flex items-center">
          <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => onEdit(quota)} aria-label={`编辑 ${quota.username} 的${quotaTypeLabels[quota.quotaType]}配额`}>
            <Pencil className="h-4 w-4" />
          </Button>
          <Button variant="ghost" size="icon" className="h-8 w-8 text-destructive" onClick={() => onDelete(quota)} aria-label={`删除 ${quota.username} 的${quotaTypeLabels[quota.quotaType]}配额`}>
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </TableCell>
    </TableRow>
  )
}

function QuotaMobileCard({ quota, onEdit, onDelete }: { quota: Quota; onEdit: (quota: Quota) => void; onDelete: (quota: Quota) => void }) {
  return (
    <article className="space-y-4 p-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="font-semibold">{quota.username}</h3>
          <p className="text-xs text-muted-foreground">{periodLabels[quota.period]} · {quotaTypeLabels[quota.quotaType]}</p>
        </div>
        <div className="flex items-center">
          <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => onEdit(quota)} aria-label={`编辑 ${quota.username} 的${quotaTypeLabels[quota.quotaType]}配额`}><Pencil className="h-4 w-4" /></Button>
          <Button variant="ghost" size="icon" className="h-8 w-8 text-destructive" onClick={() => onDelete(quota)} aria-label={`删除 ${quota.username} 的${quotaTypeLabels[quota.quotaType]}配额`}><Trash2 className="h-4 w-4" /></Button>
        </div>
      </div>
      <div>
        <p className="mb-2 text-sm font-medium tabular-nums">{formatQuotaValue(quota.used, quota.quotaType)} <span className="text-muted-foreground">/ {formatQuotaValue(quota.limit, quota.quotaType)}</span></p>
        <QuotaProgress quota={quota} />
      </div>
      <p className="text-xs text-muted-foreground">下次重置：{formatDate(quota.resetAt)}</p>
    </article>
  )
}

function QuotaProgress({ quota }: { quota: Quota }) {
  const percent = quotaUsagePercent(quota.used, quota.limit)
  const risk = quotaRisk(quota.used, quota.limit)
  const blocked = quota.limit <= 0
  return (
    <div className="flex items-center gap-2">
      <Progress
        value={Math.min(percent, 100)}
        aria-label={`${quota.username} ${quotaTypeLabels[quota.quotaType]}使用率`}
        aria-valuetext={blocked ? '额度为 0，已阻止调用' : `${percent}%`}
        className={cn('flex-1', risk === 'exhausted' ? '[&>div]:bg-red-500' : risk === 'warning' ? '[&>div]:bg-amber-500' : '[&>div]:bg-emerald-500')}
      />
      <span className={cn('w-12 text-right text-sm font-semibold tabular-nums', risk === 'exhausted' ? 'text-red-600 dark:text-red-400' : risk === 'warning' ? 'text-amber-600 dark:text-amber-400' : 'text-emerald-600 dark:text-emerald-400')}>
        {blocked ? '阻止' : `${percent}%`}
      </span>
    </div>
  )
}

function FilterSelect({ label, value, items, onChange }: { label: string; value: string; items: [string, string][]; onChange: (value: string) => void }) {
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="w-full bg-background" aria-label={label}><SelectValue /></SelectTrigger>
      <SelectContent>{items.map(([itemValue, itemLabel]) => <SelectItem key={itemValue} value={itemValue}>{itemLabel}</SelectItem>)}</SelectContent>
    </Select>
  )
}

function ErrorNotice({ message, actionLabel, onAction }: { message: string; actionLabel?: string; onAction?: () => void }) {
  if (!message) return null
  return (
    <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      <span>{message}</span>
      {actionLabel && onAction && <Button variant="outline" size="sm" onClick={onAction}><RotateCw className="mr-2 h-4 w-4" />{actionLabel}</Button>}
    </div>
  )
}

const quotaTypeLabels: Record<QuotaType, string> = {
  tokens: 'Token 数',
  requests: '请求数',
  cost: '费用',
}

const periodLabels: Record<QuotaPeriod, string> = {
  daily: '每日',
  weekly: '每周',
  monthly: '每月',
}

function formatQuotaValue(value: number, type: QuotaType): string {
  if (type === 'cost') return `$${value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 4 })}`
  return formatNumber(value)
}

function periodDescription(period: QuotaPeriod): string {
  if (period === 'daily') return '每日 00:00 UTC 重置'
  if (period === 'weekly') return '每周一 00:00 UTC 重置'
  return '每个自然月首日 00:00 UTC 重置'
}

function errorMessage(error: unknown): string {
  if (!error) return ''
  return error instanceof Error ? error.message : String(error)
}
