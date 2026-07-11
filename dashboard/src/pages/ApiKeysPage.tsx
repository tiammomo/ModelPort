import { useMemo, useState, type ElementType } from 'react'
import { useApiKeys, useCreateApiKey, useDeleteApiKey, useRevokeApiKey, useTeams, useUpdateApiKey, useUpsertTeam, useUsers } from '@/hooks'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { ConfirmDialog } from '@/components/shared/ConfirmDialog'
import { EmptyState } from '@/components/shared/EmptyState'
import { Skeleton } from '@/components/shared/Skeleton'
import { PaginationBar } from '@/components/shared/PaginationBar'
import { Card, CardContent, CardFooter } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { cn, formatDate, formatNumber } from '@/lib/utils'
import { paginateItems } from '@/lib/pagination'
import { useAuthStore } from '@/stores'
import { apiKeyAccessForRole, apiKeySelfServiceUpdate } from '@/features/api-keys/api-key-access'
import { apiKeyExpiryState, filterApiKeys, isApiKeyFilterActive, type ApiKeyStatusFilter } from '@/features/api-keys/api-key-view'
import { AlertTriangle, CalendarClock, Copy, DollarSign, FolderKanban, KeyRound, Pencil, Plus, RotateCw, Search, ShieldCheck, ShieldOff, Trash2, X, Zap } from 'lucide-react'
import type { ApiKey, Team } from '@/types'
import { toast } from 'sonner'

const ALL = '__all__'
const ALL_STATUSES = 'all'
const NO_GROUP = '__none__'
const NO_TEAM = '__none_team__'
const TEAM_PAGE_SIZE_OPTIONS = [4, 8, 12]

interface ConfirmAction {
  title: string
  description: string
  confirmLabel: string
  destructive?: boolean
  onConfirm: () => void
}

interface EditApiKeyForm {
  name: string
  group: string
  teamId: string
  allowedModels: string
  allowedProviders: string
  status: ApiKey['status']
  expiresAt: string
  ipRestricted: boolean
  allowedIps: string
  spendLimitUsd: string
  rateLimited: boolean
  fiveHourLimitUsd: string
  dailyLimitUsd: string
  weeklyLimitUsd: string
  monthlyLimitUsd: string
}

interface TeamForm {
  name: string
  dailyLimitUsd: string
  monthlyLimitUsd: string
  allowedModels: string
  allowedProviders: string
}

const emptyEditForm: EditApiKeyForm = {
  name: '',
  group: '',
  teamId: '',
  allowedModels: '',
  allowedProviders: '',
  status: 'active',
  expiresAt: '',
  ipRestricted: false,
  allowedIps: '',
  spendLimitUsd: '',
  rateLimited: false,
  fiveHourLimitUsd: '',
  dailyLimitUsd: '',
  weeklyLimitUsd: '',
  monthlyLimitUsd: '',
}

export function ApiKeysPage() {
  const currentUser = useAuthStore((state) => state.currentUser)
  const access = apiKeyAccessForRole(currentUser?.role)
  const isAdmin = access.isAdmin
  const { data: apiKeys = [], isLoading, isError, error, refetch } = useApiKeys()
  const { data: users = [], isLoading: usersLoading, isError: usersError, error: usersQueryError, refetch: refetchUsers } = useUsers()
  const { data: teams = [], isError: teamsError, error: teamsQueryError, refetch: refetchTeams } = useTeams()
  const createApiKey = useCreateApiKey()
  const upsertTeam = useUpsertTeam()
  const revokeApiKey = useRevokeApiKey()
  const updateApiKey = useUpdateApiKey()
  const deleteApiKey = useDeleteApiKey()

  const [search, setSearch] = useState('')
  const [referenceTime] = useState(() => Date.now())
  const [status, setStatus] = useState<ApiKeyStatusFilter>(ALL_STATUSES)
  const [group, setGroup] = useState(ALL)
  const [keysPage, setKeysPage] = useState(1)
  const [keysPageSize, setKeysPageSize] = useState(20)
  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [editingKey, setEditingKey] = useState<ApiKey | null>(null)
  const [confirmAction, setConfirmAction] = useState<ConfirmAction | null>(null)
  const [newKey, setNewKey] = useState<string | null>(null)
  const [form, setForm] = useState({
    userId: '',
    name: '',
    group: '',
    teamId: '',
    allowedModels: '',
    allowedProviders: '',
  })
  const [teamForm, setTeamForm] = useState<TeamForm>({
    name: '',
    dailyLimitUsd: '',
    monthlyLimitUsd: '',
    allowedModels: '',
    allowedProviders: '',
  })
  const [editForm, setEditForm] = useState<EditApiKeyForm>(emptyEditForm)

  const groups = useMemo(() => {
    return Array.from(new Set(apiKeys.map((key) => key.group).filter(Boolean))).sort()
  }, [apiKeys])

  const filteredKeys = useMemo(() => filterApiKeys(apiKeys, {
    search,
    status,
    group,
    allGroupValue: ALL,
    noGroupValue: NO_GROUP,
  }, referenceTime), [apiKeys, group, referenceTime, search, status])
  const keyWindow = paginateItems(filteredKeys, keysPage, keysPageSize)

  const usableKeys = apiKeys.filter((key) => key.status === 'active' && apiKeyExpiryState(key, referenceTime) !== 'expired').length
  const revokedKeys = apiKeys.filter((key) => key.status === 'revoked').length
  const expiringKeys = apiKeys.filter((key) => key.status === 'active' && apiKeyExpiryState(key, referenceTime) === 'expiring').length
  const expiredKeys = apiKeys.filter((key) => key.status === 'active' && apiKeyExpiryState(key, referenceTime) === 'expired').length
  const requestsToday = apiKeys.reduce((sum, key) => sum + (key.requestsToday ?? 0), 0)
  const tokensToday = apiKeys.reduce((sum, key) => sum + (key.tokensToday ?? 0), 0)
  const apiBaseUrl = String(import.meta.env.VITE_API_BASE_URL || window.location.origin).replace(/\/+$/, '')
  const eligibleUsers = users.filter((user) => user.status === 'active')
  const filtersActive = isApiKeyFilterActive({ search, status, group, allGroupValue: ALL })
  const editExpiresInPast = Boolean(editForm.expiresAt && new Date(editForm.expiresAt).getTime() <= referenceTime)
  const editHasInvalidUsdLimit = [editForm.spendLimitUsd, editForm.fiveHourLimitUsd, editForm.dailyLimitUsd, editForm.weeklyLimitUsd, editForm.monthlyLimitUsd]
    .some((value) => !isUsdInputValid(value))
  const teamHasInvalidBudget = [teamForm.dailyLimitUsd, teamForm.monthlyLimitUsd]
    .some((value) => !isUsdInputValid(value))

  const handleCreate = () => {
    if (!access.canCreate) return
    const user = users.find((item) => item.id === form.userId)
    if (!user || user.status !== 'active' || !form.name.trim()) return
    createApiKey.mutate({
      userId: user.id,
      username: user.username,
      name: form.name.trim(),
      group: form.group.trim() || undefined,
      teamId: form.teamId || undefined,
      allowedModels: parsePolicyList(form.allowedModels),
      allowedProviders: parsePolicyList(form.allowedProviders),
    }, {
      onSuccess: (key) => {
        setForm({ userId: '', name: '', group: '', teamId: '', allowedModels: '', allowedProviders: '' })
        if (key.key) {
          setNewKey(key.key)
          toast.success('API 密钥已创建')
        } else {
          setShowCreateDialog(false)
          toast.error('密钥已创建，但服务端未返回完整密钥；请删除该密钥后重新创建')
        }
      },
      onError: (error) => toast.error(error instanceof Error ? error.message : '创建密钥失败'),
    })
  }

  const handleCreateTeam = () => {
    if (!access.canManageTeams) return
    if (!teamForm.name.trim() || teamHasInvalidBudget) return
    upsertTeam.mutate({
      name: teamForm.name.trim(),
      dailyLimitUsd: parseUsdLimit(teamForm.dailyLimitUsd),
      monthlyLimitUsd: parseUsdLimit(teamForm.monthlyLimitUsd),
      allowedModels: parsePolicyList(teamForm.allowedModels),
      allowedProviders: parsePolicyList(teamForm.allowedProviders),
      status: 'active',
    }, {
      onSuccess: () => {
        setTeamForm({ name: '', dailyLimitUsd: '', monthlyLimitUsd: '', allowedModels: '', allowedProviders: '' })
        toast.success('项目已创建')
      },
      onError: (error) => toast.error(error instanceof Error ? error.message : '创建项目失败'),
    })
  }

  const openCreateDialog = () => {
    if (!access.canCreate) return
    createApiKey.reset()
    setNewKey(null)
    setShowCreateDialog(true)
  }

  const openEditDialog = (apiKey: ApiKey) => {
    if (!access.canEdit) return
    updateApiKey.reset()
    setEditingKey(apiKey)
    setEditForm(apiKeyToEditForm(apiKey))
  }

  const updateEditForm = (patch: Partial<EditApiKeyForm>) => {
    setEditForm((current) => ({ ...current, ...patch }))
  }

  const closeEditDialog = () => {
    setEditingKey(null)
    setEditForm(emptyEditForm)
  }

  const handleToggleStatus = (apiKey: ApiKey) => {
    if (apiKey.status === 'active') {
      if (!access.canRevoke) return
      setConfirmAction({
        title: '禁用 API 密钥',
        description: `禁用后，${apiKey.name} 将无法继续调用 API。`,
        confirmLabel: '禁用',
        destructive: true,
        onConfirm: () => revokeApiKey.mutate(apiKey.id, {
          onSuccess: () => toast.success('API 密钥已禁用'),
          onSettled: () => setConfirmAction(null),
          onError: (error) => toast.error(error instanceof Error ? error.message : '禁用密钥失败'),
        }),
      })
      return
    }

    if (!access.canRestore) return
    if (apiKeyExpiryState(apiKey) === 'expired') {
      toast.error(isAdmin ? '该密钥已过期，请先编辑过期时间再恢复' : '该密钥已过期，请联系管理员调整过期时间')
      return
    }
    updateApiKey.mutate({
      keyId: apiKey.id,
      data: { status: 'active' },
    }, {
      onSuccess: () => toast.success('API 密钥已恢复'),
      onError: (error) => toast.error(error instanceof Error ? error.message : '恢复密钥失败'),
    })
  }

  const persistApiKeyUpdate = () => {
    if (!access.canEdit || !editingKey || !editForm.name.trim() || editHasInvalidUsdLimit) return

    const selfServiceFields = apiKeySelfServiceUpdate(editForm.name, editForm.group)

    updateApiKey.mutate({
      keyId: editingKey.id,
      data: access.canManagePolicy
        ? {
            ...selfServiceFields,
            teamId: editForm.teamId,
            allowedModels: parsePolicyList(editForm.allowedModels),
            allowedProviders: parsePolicyList(editForm.allowedProviders),
            status: editForm.status,
            expiresAt: localDateTimeToMillis(editForm.expiresAt),
            ipRestricted: editForm.ipRestricted,
            allowedIps: parseAllowedIps(editForm.allowedIps),
            spendLimitUsd: parseUsdLimit(editForm.spendLimitUsd),
            rateLimited: editForm.rateLimited,
            fiveHourLimitUsd: parseUsdLimit(editForm.fiveHourLimitUsd),
            dailyLimitUsd: parseUsdLimit(editForm.dailyLimitUsd),
            weeklyLimitUsd: parseUsdLimit(editForm.weeklyLimitUsd),
            monthlyLimitUsd: parseUsdLimit(editForm.monthlyLimitUsd),
          }
        : selfServiceFields,
    }, {
      onSuccess: () => {
        closeEditDialog()
        toast.success('API 密钥设置已保存')
      },
      onSettled: () => setConfirmAction(null),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新密钥失败'),
    })
  }

  const handleUpdate = () => {
    if (!editingKey || !editForm.name.trim()) return
    if (editingKey.status === 'active' && editForm.status === 'revoked') {
      setConfirmAction({
        title: `禁用 API 密钥 ${editingKey.name}？`,
        description: '禁用立即生效，使用该密钥的客户端会停止调用；其他编辑内容也会同时保存。',
        confirmLabel: '保存并禁用',
        destructive: true,
        onConfirm: persistApiKeyUpdate,
      })
      return
    }
    persistApiKeyUpdate()
  }

  const copyText = async (text: string, label = '内容') => {
    try {
      await navigator.clipboard.writeText(text)
      toast.success(`${label}已复制`)
    } catch {
      toast.error('复制失败，请手动复制')
    }
  }

  const mutationError = errorMessage(deleteApiKey.error || revokeApiKey.error || updateApiKey.error)

  const handleKeysPageChange = (page: number) => {
    setKeysPage(Math.min(Math.max(page, 1), keyWindow.totalPages))
  }

  const handleKeysPageSizeChange = (pageSize: number) => {
    setKeysPageSize(pageSize)
    setKeysPage(1)
  }

  const resetFilters = () => {
    setSearch('')
    setStatus(ALL_STATUSES)
    setGroup(ALL)
    setKeysPage(1)
  }

  const requestDeleteKey = (apiKey: ApiKey) => {
    setConfirmAction({
      title: `永久删除 ${apiKey.name}？`,
      description: '删除后密钥无法恢复，客户端会立即停止调用；历史请求记录仍会保留。若只想临时停止访问，请选择“禁用”。',
      confirmLabel: '永久删除',
      destructive: true,
      onConfirm: () => deleteApiKey.mutate(apiKey.id, {
        onSuccess: () => toast.success('API 密钥已删除'),
        onSettled: () => setConfirmAction(null),
        onError: (error) => toast.error(error instanceof Error ? error.message : '删除密钥失败'),
      }),
    })
  }

  return (
    <div className="space-y-6">
      <div className="rounded-md border bg-card p-4 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h1 className="text-2xl font-bold tracking-tight">API 密钥</h1>
            <p className="mt-1 text-sm text-muted-foreground">
              {formatNumber(usableKeys)} 个可用 / {formatNumber(apiKeys.length)} 个总数 · 今日 {formatNumber(requestsToday)} 请求 · {formatNumber(tokensToday)} tokens
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <Button variant="outline" size="icon" onClick={() => refetch()} aria-label="刷新 API 密钥">
              <RotateCw className="h-4 w-4" />
            </Button>
            {access.canCreate && (
              <Button onClick={openCreateDialog}>
                <Plus className="mr-2 h-4 w-4" />
                创建密钥
              </Button>
            )}
          </div>
        </div>

        <div className="mt-5 flex flex-wrap items-center gap-3">
          <div className="relative min-w-[260px] flex-1">
            <Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
            <Input
              className="h-11 rounded-md bg-background pl-9"
              placeholder="搜索名称、用户、密钥标识或项目"
              aria-label="搜索 API 密钥"
              value={search}
              onChange={(event) => {
                setSearch(event.target.value)
                setKeysPage(1)
              }}
            />
          </div>
          <Select
            value={group}
            onValueChange={(value) => {
              setGroup(value)
              setKeysPage(1)
            }}
          >
            <SelectTrigger className="h-11 w-full bg-background sm:w-[180px]" aria-label="按标签筛选"><SelectValue placeholder="全部标签" /></SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL}>全部标签</SelectItem>
              <SelectItem value={NO_GROUP}>无标签</SelectItem>
              {groups.map((item) => (
                <SelectItem key={item} value={item || ''}>{item}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={status}
            onValueChange={(value) => {
              setStatus(value as ApiKeyStatusFilter)
              setKeysPage(1)
            }}
          >
            <SelectTrigger className="h-11 w-full bg-background sm:w-[180px]" aria-label="按状态和过期风险筛选"><SelectValue placeholder="全部状态" /></SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL_STATUSES}>全部状态</SelectItem>
              <SelectItem value="active">已启用</SelectItem>
              <SelectItem value="expiring">7 天内过期</SelectItem>
              <SelectItem value="expired">已过期</SelectItem>
              <SelectItem value="revoked">已禁用</SelectItem>
            </SelectContent>
          </Select>
          {filtersActive && (
            <Button variant="ghost" className="h-11" onClick={resetFilters}>
              <X className="mr-2 h-4 w-4" />清除筛选
            </Button>
          )}
        </div>

        <p className="mt-2 text-xs text-muted-foreground" aria-live="polite">
          显示 {formatNumber(filteredKeys.length)} / {formatNumber(apiKeys.length)} 个密钥
        </p>

        <div className="mt-4 flex flex-wrap gap-2">
          <EndpointPill label="Anthropic" tag="默认" value={`${apiBaseUrl}/v1/messages`} onCopy={(value) => void copyText(value, 'Anthropic 端点')} />
          <EndpointPill label="模型列表" value={`${apiBaseUrl}/v1/models`} onCopy={(value) => void copyText(value, '模型列表端点')} />
          <div className="inline-flex h-8 items-center rounded-md border bg-background px-3 text-xs text-muted-foreground">
            已禁用 {formatNumber(revokedKeys)}
          </div>
          {(expiringKeys > 0 || expiredKeys > 0) && (
            <div className="inline-flex h-8 items-center gap-1.5 rounded-md border border-amber-300 bg-amber-50 px-3 text-xs text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
              <AlertTriangle className="h-3.5 w-3.5" />7 天内过期 {formatNumber(expiringKeys)} · 已过期 {formatNumber(expiredKeys)}
            </div>
          )}
        </div>
      </div>

      {isError && <ErrorNotice message={`API 密钥加载失败：${errorMessage(error)}`} actionLabel="重新加载" onAction={() => void refetch()} />}
      {usersError && access.canCreate && <ErrorNotice message={`用户列表加载失败：${errorMessage(usersQueryError)}`} actionLabel="重试" onAction={() => void refetchUsers()} />}
      {teamsError && access.canManagePolicy && <ErrorNotice message={`项目列表加载失败：${errorMessage(teamsQueryError)}`} actionLabel="重试" onAction={() => void refetchTeams()} />}

      <Card className="overflow-hidden">
        {access.canManageTeams && (
          <div className="border-b bg-muted/20 px-5 py-4">
            <TeamStrip
              teams={teams}
              form={teamForm}
              pending={upsertTeam.isPending}
              invalidBudget={teamHasInvalidBudget}
              error={errorMessage(upsertTeam.error)}
              onChange={(patch) => setTeamForm((current) => ({ ...current, ...patch }))}
              onCreate={handleCreateTeam}
            />
          </div>
        )}
        {mutationError && (
          <div className="border-b px-5 py-3">
            <ErrorNotice message={mutationError} />
          </div>
        )}
        <CardContent className="p-0">
          <div className="hidden lg:block">
            <Table className="min-w-[1190px] 2xl:min-w-[1360px]">
            <TableHeader className="bg-muted/40">
              <TableRow className="hover:bg-transparent">
                <TableHead className="sticky left-0 z-20 w-[190px] border-r bg-muted/95 px-5">名称</TableHead>
                <TableHead className="w-[190px]">密钥标识</TableHead>
                <TableHead className="w-[150px]">用户</TableHead>
                <TableHead className="w-[240px]">项目/标签</TableHead>
                <TableHead className="w-[165px] text-right">用量</TableHead>
                <TableHead className="w-[220px]">费用限制</TableHead>
                <TableHead className="w-[150px]">过期时间</TableHead>
                <TableHead className="w-[110px]">状态</TableHead>
                <TableHead className="w-[180px]">上次使用时间</TableHead>
                <TableHead className="hidden w-[170px] 2xl:table-cell">创建时间</TableHead>
                <TableHead className="sticky right-0 z-20 w-[180px] border-l bg-muted text-center">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                Array.from({ length: 5 }).map((_, i) => (
                  <TableRow key={`skeleton-${i}`} className="h-[94px]">
                    <TableCell colSpan={11}>
                      <Skeleton className="h-6 w-full" />
                    </TableCell>
                  </TableRow>
                ))
              ) : filteredKeys.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={11} className="h-28">
                    <EmptyState
                      icon={KeyRound}
                      title={isError ? '无法加载 API 密钥' : filtersActive ? '没有匹配的 API 密钥' : '暂无 API 密钥'}
                      description={isError
                        ? '检查网络或服务状态后重新加载。'
                        : filtersActive
                        ? '没有匹配的 API 密钥，请调整筛选条件'
                        : isAdmin
                          ? '点击「创建密钥」按钮签发第一个 API 密钥'
                          : '当前账号没有可查看的 API 密钥'}
                      action={isError
                        ? <Button variant="outline" onClick={() => void refetch()}>重新加载</Button>
                        : filtersActive ? <Button variant="outline" onClick={resetFilters}>清除筛选</Button> : undefined}
                    />
                  </TableCell>
                </TableRow>
              ) : keyWindow.items.map((key) => (
                <TableRow key={key.id} className="h-[94px]">
                  <TableCell className="sticky left-0 z-10 border-r bg-card px-5 font-medium">
                    <div className="space-y-1">
                      <p className="truncate text-base">{key.name}</p>
                      <p className="text-xs text-muted-foreground">{key.id}</p>
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-1.5">
                      <code className="rounded-md bg-cyan-50 px-2.5 py-1 text-xs font-semibold text-cyan-700 dark:bg-cyan-950/40 dark:text-cyan-300">
                        {key.keyPreview || key.keyPrefix}
                      </code>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={() => void copyText(key.keyPreview || key.keyPrefix, '密钥标识')}
                        aria-label={`复制 ${key.name} 密钥标识`}
                      >
                        <Copy className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="space-y-1">
                      <p className="truncate font-medium">{key.username || key.userId}</p>
                      <p className="truncate text-xs text-muted-foreground">{key.userId}</p>
                    </div>
                  </TableCell>
                  <TableCell>
                    <GroupCell group={key.group} teamName={key.teamName} />
                  </TableCell>
                  <TableCell className="text-right">
                    <UsageCell apiKey={key} />
                  </TableCell>
                  <TableCell>
                    <RateLimitCell apiKey={key} />
                  </TableCell>
                  <TableCell>
                    <ExpiresCell value={key.expiresAt} />
                  </TableCell>
                  <TableCell>
                    <EffectiveStatusBadge apiKey={key} />
                  </TableCell>
                  <TableCell className="hidden text-sm text-muted-foreground 2xl:table-cell">
                    {key.lastUsedAt ? formatDate(key.lastUsedAt) : '从未使用'}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {formatDate(key.createdAt)}
                  </TableCell>
                  <TableCell className="sticky right-0 z-10 border-l bg-card">
                    <div className="flex justify-center gap-1">
                      {access.canEdit && (
                        <ActionButton
                          icon={Pencil}
                          label="编辑"
                          onClick={() => openEditDialog(key)}
                        />
                      )}
                      {((key.status === 'active' && access.canRevoke) || (key.status === 'revoked' && access.canRestore)) && (
                        <ActionButton
                          icon={key.status === 'active' ? ShieldOff : apiKeyExpiryState(key) === 'expired' ? CalendarClock : ShieldCheck}
                          label={key.status === 'active' ? '禁用' : apiKeyExpiryState(key) === 'expired' ? '已过期' : '恢复'}
                          className={key.status === 'active' ? undefined : 'text-emerald-600 hover:text-emerald-700'}
                          disabled={revokeApiKey.isPending || updateApiKey.isPending || (key.status === 'revoked' && apiKeyExpiryState(key) === 'expired')}
                          onClick={() => handleToggleStatus(key)}
                        />
                      )}
                      {access.canDelete && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-12 w-11 flex-col gap-1 px-1 text-xs text-destructive"
                          onClick={() => requestDeleteKey(key)}
                        >
                          <Trash2 className="h-4 w-4" />
                          删除
                        </Button>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
            </Table>
          </div>

          <div className="lg:hidden">
            {isLoading ? (
              <div className="space-y-3 p-4" aria-label="正在加载 API 密钥">
                {Array.from({ length: 4 }).map((_, index) => <Skeleton key={index} className="h-44 w-full" />)}
              </div>
            ) : filteredKeys.length === 0 ? (
              <EmptyState
                icon={KeyRound}
                title={isError ? '无法加载 API 密钥' : filtersActive ? '没有匹配的 API 密钥' : '暂无 API 密钥'}
                description={isError ? '检查网络或服务状态后重新加载。' : filtersActive ? '尝试清除筛选或更换关键词。' : isAdmin ? '创建第一把密钥并立即安全保存。' : '当前账号没有可查看的 API 密钥。'}
                action={isError
                  ? <Button variant="outline" onClick={() => void refetch()}>重新加载</Button>
                  : filtersActive ? <Button variant="outline" onClick={resetFilters}>清除筛选</Button> : undefined}
              />
            ) : (
              <div className="divide-y">
                {keyWindow.items.map((key) => (
                  <ApiKeyMobileCard
                    key={key.id}
                    apiKey={key}
                    canEdit={access.canEdit}
                    canToggle={(key.status === 'active' && access.canRevoke) || (key.status === 'revoked' && access.canRestore && apiKeyExpiryState(key) !== 'expired')}
                    canDelete={access.canDelete}
                    pending={revokeApiKey.isPending || updateApiKey.isPending || deleteApiKey.isPending}
                    onCopy={() => void copyText(key.keyPreview || key.keyPrefix, '密钥标识')}
                    onEdit={() => openEditDialog(key)}
                    onToggle={() => handleToggleStatus(key)}
                    onDelete={() => requestDeleteKey(key)}
                  />
                ))}
              </div>
            )}
          </div>
        </CardContent>
        {filteredKeys.length > 0 && (
          <CardFooter className="border-t px-5 py-3">
            <PaginationBar
              total={filteredKeys.length}
              page={keyWindow.currentPage}
              pageSize={keysPageSize}
              totalPages={keyWindow.totalPages}
              start={keyWindow.start}
              end={keyWindow.end}
              totalLabel="个 API 密钥"
              onPageChange={handleKeysPageChange}
              onPageSizeChange={handleKeysPageSizeChange}
            />
          </CardFooter>
        )}
      </Card>

      {access.canCreate && (
        <Dialog
          open={showCreateDialog}
          onOpenChange={(open) => {
            if (!open && newKey) {
              toast.warning('请先保存完整密钥，再点击“已保存，关闭”')
              return
            }
            setShowCreateDialog(open)
            if (!open) setNewKey(null)
          }}
        >
          <DialogContent>
          <DialogHeader>
            <DialogTitle>创建 API 密钥</DialogTitle>
            <DialogDescription>为可用用户签发密钥；访问范围留空时继承系统和项目策略。</DialogDescription>
          </DialogHeader>

          {newKey ? (
            <div className="space-y-4">
              <div role="status" className="rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-100">
                <p className="font-semibold">立即复制并安全保存</p>
                <p className="mt-1">完整密钥只显示这一次。不要通过聊天、邮件或工单发送。</p>
              </div>
              <Label htmlFor="new-api-key">新 API 密钥</Label>
              <div className="flex gap-2">
                <Input id="new-api-key" value={newKey} readOnly className="font-mono text-xs" onFocus={(event) => event.currentTarget.select()} />
                <Button variant="outline" size="icon" onClick={() => void copyText(newKey, 'API 密钥')} aria-label="复制新 API 密钥">
                  <Copy className="h-4 w-4" />
                </Button>
              </div>
              <div className="space-y-3 rounded-md border bg-muted/20 p-3">
                <div>
                  <p className="text-sm font-medium">接入配置</p>
                  <p className="mt-1 text-xs text-muted-foreground">复制后把 YOUR_MODEL 替换为模型列表端点返回的模型 ID，或管理员提供的稳定路由别名。OpenAI-compatible 是上游适配能力，不是客户端入口。</p>
                </div>
                <CopySnippet
                  title="Claude Code / Anthropic SDK"
                  value={`ANTHROPIC_BASE_URL=${apiBaseUrl}\nANTHROPIC_AUTH_TOKEN=${newKey}\nANTHROPIC_MODEL=YOUR_MODEL`}
                  onCopy={(value) => void copyText(value, 'Claude Code 配置')}
                />
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              {createApiKey.error && <ErrorNotice message={errorMessage(createApiKey.error)} />}
              <div className="space-y-2">
                <Label htmlFor="create-key-user">用户 <span className="text-destructive" aria-hidden="true">*</span></Label>
                <Select value={form.userId} onValueChange={(value) => setForm({ ...form, userId: value })}>
                  <SelectTrigger id="create-key-user" aria-required="true"><SelectValue placeholder={usersLoading ? '正在加载用户…' : '选择可用用户'} /></SelectTrigger>
                  <SelectContent>
                    {eligibleUsers.map((user) => (
                      <SelectItem key={user.id} value={user.id}>{user.username} · {user.email}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {!usersLoading && eligibleUsers.length === 0 && <p className="text-xs text-muted-foreground">暂无可签发密钥的可用用户，请先创建或恢复用户。</p>}
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-key-name">名称 <span className="text-destructive" aria-hidden="true">*</span></Label>
                <Input id="create-key-name" required value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="例如：Alice 的 Claude Code" />
                <p className="text-xs text-muted-foreground">使用能识别人员、设备或用途的名称，便于泄露时快速定位。</p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-key-group">标签（可选）</Label>
                <Input id="create-key-group" value={form.group} onChange={(event) => setForm({ ...form, group: event.target.value })} placeholder="例如：研发 / CI" />
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-key-team">项目（可选）</Label>
                <Select value={form.teamId || NO_TEAM} onValueChange={(value) => setForm({ ...form, teamId: value === NO_TEAM ? '' : value })}>
                  <SelectTrigger id="create-key-team"><SelectValue placeholder="选择项目" /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NO_TEAM}>不绑定项目</SelectItem>
                    {teams.map((team) => (
                      <SelectItem key={team.id} value={team.id}>{team.name}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="create-key-models">允许模型（可选）</Label>
                  <Input id="create-key-models" value={form.allowedModels} onChange={(event) => setForm({ ...form, allowedModels: event.target.value })} placeholder="mimo*, claude-sonnet*" />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="create-key-providers">允许上游 Provider（可选）</Label>
                  <Input id="create-key-providers" value={form.allowedProviders} onChange={(event) => setForm({ ...form, allowedProviders: event.target.value })} placeholder="mimo, openai" />
                </div>
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground">多个值用逗号或空格分隔，支持后缀 <code>*</code>；留空表示不增加密钥级限制。</p>
            </div>
          )}

          <DialogFooter>
            {newKey ? (
              <Button onClick={() => { setNewKey(null); setShowCreateDialog(false) }}>已保存，关闭</Button>
            ) : (
              <>
                <Button variant="outline" onClick={() => setShowCreateDialog(false)}>取消</Button>
                <Button onClick={handleCreate} disabled={!form.userId || !form.name.trim() || usersLoading || createApiKey.isPending}>
                  <KeyRound className="mr-2 h-4 w-4" />
                  {createApiKey.isPending ? '创建中…' : '创建密钥'}
                </Button>
              </>
            )}
          </DialogFooter>
          </DialogContent>
        </Dialog>
      )}

      {access.canEdit && (
        <Dialog open={!!editingKey} onOpenChange={(open) => { if (!open) closeEditDialog() }}>
          <DialogContent className="max-h-[90vh] max-w-[640px] gap-0 overflow-hidden p-0 sm:rounded-2xl">
          <DialogHeader className="border-b px-7 py-5">
            <DialogTitle className="text-xl">编辑密钥</DialogTitle>
            <DialogDescription className="sr-only">更新 API 密钥设置</DialogDescription>
          </DialogHeader>

          <div className="max-h-[calc(90vh-146px)] space-y-5 overflow-y-auto px-7 py-5">
            {updateApiKey.error && <ErrorNotice message={errorMessage(updateApiKey.error)} />}
            {editHasInvalidUsdLimit && <ErrorNotice message="费用限额必须是非负数。" />}
            <div className="space-y-2">
              <Label htmlFor="edit-key-name">名称</Label>
              <Input
                id="edit-key-name"
                className="h-12 rounded-xl"
                value={editForm.name}
                onChange={(event) => updateEditForm({ name: event.target.value })}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="edit-key-group">标签</Label>
              <Input
                id="edit-key-group"
                className="h-12 rounded-xl"
                value={editForm.group}
                onChange={(event) => updateEditForm({ group: event.target.value })}
                placeholder="留空表示无标签"
              />
            </div>

            {!isAdmin && (
              <p className="rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">
                普通用户只能修改密钥名称和标签；访问策略、状态与费用限额由管理员维护。
              </p>
            )}

            {isAdmin && (
              <>
                <div className="space-y-2">
                  <Label htmlFor="edit-key-team">项目</Label>
                  <Select
                    value={editForm.teamId || NO_TEAM}
                    onValueChange={(value) => updateEditForm({ teamId: value === NO_TEAM ? '' : value })}
                  >
                    <SelectTrigger id="edit-key-team" className="h-12 rounded-xl bg-background">
                      <SelectValue placeholder="选择项目" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={NO_TEAM}>不绑定项目</SelectItem>
                      {teams.map((team) => (
                        <SelectItem key={team.id} value={team.id}>{team.name}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="edit-key-models">允许模型</Label>
                    <Input
                      id="edit-key-models"
                      className="h-12 rounded-xl"
                      value={editForm.allowedModels}
                      onChange={(event) => updateEditForm({ allowedModels: event.target.value })}
                      placeholder="留空表示不限"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="edit-key-providers">允许上游 Provider</Label>
                    <Input
                      id="edit-key-providers"
                      className="h-12 rounded-xl"
                      value={editForm.allowedProviders}
                      onChange={(event) => updateEditForm({ allowedProviders: event.target.value })}
                      placeholder="留空表示不限"
                    />
                  </div>
                </div>

                <div className="space-y-2">
                  <Label htmlFor="edit-key-status">状态</Label>
                  <Select value={editForm.status} onValueChange={(value) => updateEditForm({ status: value as ApiKey['status'] })}>
                    <SelectTrigger id="edit-key-status" className="h-12 rounded-xl bg-background">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="active">启用</SelectItem>
                      <SelectItem value="revoked">已禁用</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <SettingSwitch
                  id="edit-key-ip-restricted"
                  label="IP 限制"
                  checked={editForm.ipRestricted}
                  onCheckedChange={(checked) => updateEditForm({ ipRestricted: checked })}
                />

                {editForm.ipRestricted && (
                  <div className="space-y-2">
                    <Label htmlFor="edit-key-allowed-ips">允许 IP / CIDR</Label>
                    <Input
                      id="edit-key-allowed-ips"
                      className="h-12 rounded-xl"
                      value={editForm.allowedIps}
                      onChange={(event) => updateEditForm({ allowedIps: event.target.value })}
                      placeholder="127.0.0.1, 10.0.0.0/8"
                    />
                    <p className="text-xs text-muted-foreground">开启后，只有匹配列表的客户端 IP 可以使用此密钥。</p>
                  </div>
                )}

                <div className="space-y-2">
                  <Label htmlFor="edit-key-spend-limit">总费用额度</Label>
                  <CurrencyInput
                    id="edit-key-spend-limit"
                    value={editForm.spendLimitUsd}
                    onChange={(value) => updateEditForm({ spendLimitUsd: value })}
                    placeholder="输入 USD 额度限制"
                  />
                </div>

                <div className="space-y-2">
                  <SettingSwitch
                    id="edit-key-rate-limited"
                    label="启用周期费用限额"
                    checked={editForm.rateLimited}
                    onCheckedChange={(checked) => updateEditForm({ rateLimited: checked })}
                  />
                  <p className="text-xs text-muted-foreground">
                    {editForm.rateLimited
                      ? '以下 USD 限额按滚动 5 小时、24 小时、7 天和 30 天窗口，在每次调用前检查。'
                      : '当前未启用；以下周期限额仅保存配置，不会参与请求检查。'}
                  </p>
                </div>

                <LimitField
                  id="edit-key-five-hour-limit"
                  label="5 小时限额 (USD)"
                  value={editForm.fiveHourLimitUsd}
                  disabled={!editForm.rateLimited}
                  onChange={(value) => updateEditForm({ fiveHourLimitUsd: value })}
                />

                <LimitField
                  id="edit-key-daily-limit"
                  label="24 小时限额 (USD)"
                  value={editForm.dailyLimitUsd}
                  disabled={!editForm.rateLimited}
                  onChange={(value) => updateEditForm({ dailyLimitUsd: value })}
                />

                <LimitField
                  id="edit-key-weekly-limit"
                  label="7 天限额 (USD)"
                  value={editForm.weeklyLimitUsd}
                  disabled={!editForm.rateLimited}
                  onChange={(value) => updateEditForm({ weeklyLimitUsd: value })}
                />

                <LimitField
                  id="edit-key-monthly-limit"
                  label="30 天限额 (USD)"
                  value={editForm.monthlyLimitUsd}
                  disabled={!editForm.rateLimited}
                  onChange={(value) => updateEditForm({ monthlyLimitUsd: value })}
                />

                <div className="space-y-2">
                  <Label htmlFor="edit-key-expires">过期时间</Label>
                  <Input
                    id="edit-key-expires"
                    className="h-12 rounded-xl"
                    type="datetime-local"
                    value={editForm.expiresAt}
                    onChange={(event) => updateEditForm({ expiresAt: event.target.value })}
                  />
                  {editExpiresInPast && (
                    <p role="alert" className="text-xs text-destructive">该时间已过去；请选择未来时间、清空有效期或将状态设为“已禁用”。</p>
                  )}
                </div>
              </>
            )}
          </div>

          <DialogFooter className="border-t bg-background px-7 py-5">
            <Button variant="outline" onClick={closeEditDialog}>取消</Button>
            <Button onClick={handleUpdate} disabled={!editForm.name.trim() || editHasInvalidUsdLimit || (isAdmin && editForm.status === 'active' && editExpiresInPast) || updateApiKey.isPending}>
              {updateApiKey.isPending ? '保存中…' : '保存更改'}
            </Button>
          </DialogFooter>
          </DialogContent>
        </Dialog>
      )}

      <ConfirmDialog
        open={!!confirmAction}
        title={confirmAction?.title || ''}
        description={confirmAction?.description || ''}
        confirmLabel={confirmAction?.confirmLabel}
        destructive={confirmAction?.destructive}
        pending={deleteApiKey.isPending || revokeApiKey.isPending || updateApiKey.isPending}
        onCancel={() => setConfirmAction(null)}
        onConfirm={() => confirmAction?.onConfirm()}
      />
    </div>
  )
}

function EndpointPill({
  label,
  tag,
  value,
  onCopy,
}: {
  label: string
  tag?: string
  value: string
  onCopy: (value: string) => void
}) {
  return (
    <div className="inline-flex h-8 max-w-full items-center gap-2 rounded-md border bg-background px-3 text-xs shadow-sm">
      <span className="font-medium text-foreground">{label}</span>
      {tag && (
        <span className="rounded bg-emerald-50 px-1.5 py-0.5 font-medium text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300">
          {tag}
        </span>
      )}
      <span className="max-w-[280px] truncate font-mono text-muted-foreground">{value}</span>
      <Button variant="ghost" size="icon" className="h-6 w-6" onClick={() => onCopy(value)} aria-label={`复制 ${label}`}>
        <Copy className="h-3.5 w-3.5" />
      </Button>
      <Zap className="h-3.5 w-3.5 text-muted-foreground" />
    </div>
  )
}

function CopySnippet({ title, value, onCopy }: { title: string; value: string; onCopy: (value: string) => void }) {
  return (
    <div className="overflow-hidden rounded-md border bg-background">
      <div className="flex items-center justify-between gap-3 border-b px-3 py-2">
        <p className="text-xs font-medium">{title}</p>
        <Button type="button" variant="ghost" size="sm" className="h-7" onClick={() => onCopy(value)}>
          <Copy className="mr-1.5 h-3.5 w-3.5" />复制配置
        </Button>
      </div>
      <pre className="overflow-x-auto whitespace-pre p-3 text-[11px] leading-5 text-muted-foreground">{value}</pre>
    </div>
  )
}

function TeamStrip({
  teams,
  form,
  pending,
  invalidBudget,
  error,
  onChange,
  onCreate,
}: {
  teams: Team[]
  form: TeamForm
  pending: boolean
  invalidBudget: boolean
  error: string
  onChange: (patch: Partial<TeamForm>) => void
  onCreate: () => void
}) {
  const [teamPage, setTeamPage] = useState(1)
  const [teamPageSize, setTeamPageSize] = useState(4)
  const [editorOpen, setEditorOpen] = useState(false)
  const teamWindow = paginateItems(teams, teamPage, teamPageSize)

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold">项目</h3>
          <p className="text-xs text-muted-foreground">按项目管理预算、模型和上游 Provider 访问范围；预算采用滚动窗口。</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {teamWindow.items.map((team) => (
            <Badge key={team.id} variant="outline" className="gap-1.5 rounded-md bg-background px-2.5 py-1">
              <FolderKanban className="h-3.5 w-3.5" />
              {team.name}
              <span className="text-muted-foreground">{team.activeApiKeys} 个密钥</span>
            </Badge>
          ))}
          {teams.length === 0 && <span className="text-xs text-muted-foreground">暂无项目</span>}
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => setEditorOpen((open) => !open)}
            aria-expanded={editorOpen}
            aria-controls="project-create-form"
          >
            <Plus className="mr-1.5 h-3.5 w-3.5" />
            {editorOpen ? '收起创建' : '新建项目'}
          </Button>
        </div>
      </div>
      {teams.length > 4 && (
        <PaginationBar
          total={teams.length}
          page={teamWindow.currentPage}
          pageSize={teamPageSize}
          totalPages={teamWindow.totalPages}
          start={teamWindow.start}
          end={teamWindow.end}
          totalLabel="个项目"
          pageSizeOptions={TEAM_PAGE_SIZE_OPTIONS}
          className="rounded-md border bg-background px-3 py-2"
          onPageChange={(page) => setTeamPage(Math.min(Math.max(page, 1), teamWindow.totalPages))}
          onPageSizeChange={(pageSize) => {
            setTeamPageSize(pageSize)
            setTeamPage(1)
          }}
        />
      )}
      {editorOpen && (
        <div id="project-create-form" className="space-y-3 rounded-md border bg-background p-3">
          {error && <ErrorNotice message={error} />}
          {invalidBudget && <ErrorNotice message="项目预算必须是非负数。" />}
          <div className="grid gap-2 md:grid-cols-[1.1fr_.7fr_.7fr_1fr_1fr_auto]">
            <Input aria-label="项目名称" value={form.name} onChange={(event) => onChange({ name: event.target.value })} placeholder="项目名称" />
            <Input aria-label="项目滚动 24 小时预算（USD）" type="number" min="0" value={form.dailyLimitUsd} onChange={(event) => onChange({ dailyLimitUsd: event.target.value })} placeholder="24h 预算 USD" />
            <Input aria-label="项目滚动 30 天预算（USD）" type="number" min="0" value={form.monthlyLimitUsd} onChange={(event) => onChange({ monthlyLimitUsd: event.target.value })} placeholder="30 天预算 USD" />
            <Input aria-label="项目模型白名单" value={form.allowedModels} onChange={(event) => onChange({ allowedModels: event.target.value })} placeholder="模型白名单（留空不限）" />
            <Input aria-label="项目上游 Provider 白名单" value={form.allowedProviders} onChange={(event) => onChange({ allowedProviders: event.target.value })} placeholder="Provider 白名单（留空不限）" />
            <Button onClick={onCreate} disabled={!form.name.trim() || invalidBudget || pending}>
              <Plus className="mr-2 h-4 w-4" />
              {pending ? '添加中…' : '添加项目'}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

function GroupCell({ group, teamName }: { group?: string | null; teamName?: string | null }) {
  if (!group && !teamName) {
    return (
      <span className="text-sm text-muted-foreground">无项目或标签</span>
    )
  }

  return (
    <div className="flex flex-wrap items-center gap-2">
      {teamName && (
        <Badge
          variant="outline"
          className="border-sky-200 bg-sky-50 font-medium text-sky-700 shadow-none dark:border-sky-900 dark:bg-sky-950/40 dark:text-sky-300"
        >
          <FolderKanban className="mr-1 h-3.5 w-3.5" />
          {teamName}
        </Badge>
      )}
      {group && (
        <Badge
          variant="outline"
          className="border-emerald-200 bg-emerald-50 font-medium text-emerald-700 shadow-none dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300"
        >
          <KeyRound className="mr-1 h-3.5 w-3.5" />
          {group}
        </Badge>
      )}
    </div>
  )
}

function ApiKeyMobileCard({
  apiKey,
  canEdit,
  canToggle,
  canDelete,
  pending,
  onCopy,
  onEdit,
  onToggle,
  onDelete,
}: {
  apiKey: ApiKey
  canEdit: boolean
  canToggle: boolean
  canDelete: boolean
  pending: boolean
  onCopy: () => void
  onEdit: () => void
  onToggle: () => void
  onDelete: () => void
}) {
  return (
    <article className="space-y-4 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate font-semibold">{apiKey.name}</h3>
          <p className="mt-0.5 truncate text-xs text-muted-foreground">{apiKey.username || apiKey.userId}</p>
        </div>
        <EffectiveStatusBadge apiKey={apiKey} />
      </div>
      <div className="flex items-center gap-2 rounded-md bg-muted/50 p-2">
        <code className="min-w-0 flex-1 truncate text-xs">{apiKey.keyPreview || apiKey.keyPrefix}</code>
        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onCopy} aria-label={`复制 ${apiKey.name} 密钥标识`}>
          <Copy className="h-3.5 w-3.5" />
        </Button>
      </div>
      <GroupCell group={apiKey.group} teamName={apiKey.teamName} />
      <div className="grid grid-cols-2 gap-3 text-sm">
        <div><p className="text-xs text-muted-foreground">今日请求</p><p className="mt-1 font-semibold tabular-nums">{formatNumber(apiKey.requestsToday ?? 0)}</p></div>
        <div><p className="text-xs text-muted-foreground">今日 Tokens</p><p className="mt-1 font-semibold tabular-nums">{formatNumber(apiKey.tokensToday ?? 0)}</p></div>
        <div><p className="text-xs text-muted-foreground">访问限制</p><p className="mt-1 truncate font-medium">{rateLimitSummary(apiKey)}</p></div>
        <div><p className="text-xs text-muted-foreground">有效期</p><div className="mt-1"><ExpiresCell value={apiKey.expiresAt} /></div></div>
      </div>
      <p className="text-xs text-muted-foreground">上次使用：{apiKey.lastUsedAt ? formatDate(apiKey.lastUsedAt) : '从未使用'}</p>
      {(canEdit || canToggle || canDelete) && (
        <div className="flex flex-wrap gap-2 border-t pt-3">
          {canEdit && <Button variant="outline" size="sm" onClick={onEdit}><Pencil className="mr-1.5 h-4 w-4" />编辑</Button>}
          {canToggle && (
            <Button variant="outline" size="sm" disabled={pending} onClick={onToggle} className={apiKey.status === 'active' ? 'text-destructive' : 'text-emerald-600'}>
              {apiKey.status === 'active' ? <ShieldOff className="mr-1.5 h-4 w-4" /> : <ShieldCheck className="mr-1.5 h-4 w-4" />}
              {apiKey.status === 'active' ? '禁用' : '恢复'}
            </Button>
          )}
          {canDelete && <Button variant="ghost" size="sm" disabled={pending} onClick={onDelete} className="ml-auto text-destructive"><Trash2 className="mr-1.5 h-4 w-4" />删除</Button>}
        </div>
      )}
    </article>
  )
}

function UsageCell({ apiKey }: { apiKey: ApiKey }) {
  return (
    <div className="flex justify-end">
      <div className="grid min-w-[118px] grid-cols-[auto_auto] gap-x-3 gap-y-1 text-sm tabular-nums">
        <span className="text-xs text-muted-foreground">今日请求</span>
        <span className="text-right font-semibold text-foreground">{formatNumber(apiKey.requestsToday ?? 0)} req</span>
        <span className="text-xs text-muted-foreground">Tokens</span>
        <span className="text-right font-medium text-muted-foreground">{formatNumber(apiKey.tokensToday ?? 0)}</span>
      </div>
    </div>
  )
}

function RateLimitCell({ apiKey }: { apiKey: ApiKey }) {
  const summary = rateLimitSummary(apiKey)
  const hasCustomLimit = summary !== '遵循项目/系统策略'

  return (
    <div className="min-w-[190px]">
      <div className="flex items-center gap-2 whitespace-nowrap">
        <span
          className={cn(
            'inline-flex h-6 items-center rounded-md px-2 text-xs font-medium',
            hasCustomLimit
              ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300'
              : 'bg-muted text-muted-foreground'
          )}
        >
          {hasCustomLimit ? '密钥级' : '无额外限制'}
        </span>
        <span className="min-w-0 text-xs font-medium leading-relaxed text-foreground/75" title={summary}>{summary}</span>
      </div>
    </div>
  )
}

function ExpiresCell({ value }: { value: string | null }) {
  if (!value) {
    return <span className="text-sm text-muted-foreground">永久有效</span>
  }

  const expiry = apiKeyExpiryState({ expiresAt: value })
  return (
    <div className={cn('flex items-center gap-2 text-sm', expiry === 'expired' ? 'text-red-600 dark:text-red-400' : expiry === 'expiring' ? 'text-amber-600 dark:text-amber-400' : undefined)}>
      <CalendarClock className="h-4 w-4" />
      <span>{formatDate(value)}{expiry === 'expired' ? ' · 已过期' : expiry === 'expiring' ? ' · 即将过期' : ''}</span>
    </div>
  )
}

function EffectiveStatusBadge({ apiKey }: { apiKey: ApiKey }) {
  const expiry = apiKeyExpiryState(apiKey)
  if (apiKey.status === 'active' && expiry === 'expired') {
    return <Badge variant="destructive">已过期</Badge>
  }
  if (apiKey.status === 'active' && expiry === 'expiring') {
    return <Badge variant="outline" className="border-amber-300 bg-amber-50 text-amber-800 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">即将过期</Badge>
  }
  if (apiKey.status === 'active') {
    return (
      <Badge
        variant="outline"
        className="border-emerald-200 bg-emerald-50 text-emerald-700 shadow-none dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300"
      >
        活跃
      </Badge>
    )
  }

  return <StatusBadge status={apiKey.status} />
}

function ActionButton({
  icon: Icon,
  label,
  disabled,
  className,
  onClick,
}: {
  icon: ElementType
  label: string
  disabled?: boolean
  className?: string
  onClick: () => void
}) {
  return (
    <Button
      variant="ghost"
      size="sm"
      className={cn('h-12 w-11 flex-col gap-1 px-1 text-xs text-muted-foreground', className, disabled && 'opacity-40')}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon className="h-4 w-4" />
      {label}
    </Button>
  )
}

function ErrorNotice({ message, actionLabel, onAction }: { message: string; actionLabel?: string; onAction?: () => void }) {
  if (!message) return null
  return (
    <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      <span>{message}</span>
      {actionLabel && onAction && (
        <Button variant="outline" size="sm" onClick={onAction}>
          <RotateCw className="mr-2 h-4 w-4" />{actionLabel}
        </Button>
      )}
    </div>
  )
}

function errorMessage(error: unknown): string {
  if (!error) return ''
  return error instanceof Error ? error.message : String(error)
}

function SettingSwitch({
  id,
  label,
  checked,
  onCheckedChange,
}: {
  id: string
  label: string
  checked: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <div className="flex min-h-10 items-center justify-between gap-4">
      <Label htmlFor={id} className="text-base font-normal">{label}</Label>
      <Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  )
}

function LimitField({
  id,
  label,
  value,
  disabled,
  onChange,
}: {
  id: string
  label: string
  value: string
  disabled?: boolean
  onChange: (value: string) => void
}) {
  return (
    <div className={cn('space-y-2', disabled && 'opacity-60')}>
      <Label htmlFor={id}>{label}</Label>
      <CurrencyInput id={id} value={value} disabled={disabled} onChange={onChange} placeholder="0" />
    </div>
  )
}

function CurrencyInput({
  id,
  value,
  placeholder,
  disabled,
  onChange,
}: {
  id: string
  value: string
  placeholder?: string
  disabled?: boolean
  onChange: (value: string) => void
}) {
  return (
    <div className="relative">
      <DollarSign className="absolute left-4 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        id={id}
        className="h-12 rounded-xl pl-10"
        inputMode="decimal"
        min="0"
        step="0.0001"
        type="number"
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  )
}

function apiKeyToEditForm(apiKey: ApiKey): EditApiKeyForm {
  return {
    name: apiKey.name,
    group: apiKey.group || '',
    teamId: apiKey.teamId || '',
    allowedModels: (apiKey.allowedModels || []).join(', '),
    allowedProviders: (apiKey.allowedProviders || []).join(', '),
    status: apiKey.status,
    expiresAt: dateTimeLocalFromValue(apiKey.expiresAt),
    ipRestricted: apiKey.ipRestricted ?? false,
    allowedIps: (apiKey.allowedIps || []).join(', '),
    spendLimitUsd: limitInput(apiKey.spendLimitUsd, ''),
    rateLimited: apiKey.rateLimited ?? false,
    fiveHourLimitUsd: limitInput(apiKey.fiveHourLimitUsd),
    dailyLimitUsd: limitInput(apiKey.dailyLimitUsd),
    weeklyLimitUsd: limitInput(apiKey.weeklyLimitUsd),
    monthlyLimitUsd: limitInput(apiKey.monthlyLimitUsd),
  }
}

function limitInput(value: number | undefined, emptyValue = '0'): string {
  if (value === undefined || value === null) return emptyValue
  return String(value)
}

function parseUsdLimit(value: string): number {
  const parsed = Number(value)
  if (!Number.isFinite(parsed)) return 0
  return Math.max(parsed, 0)
}

function isUsdInputValid(value: string): boolean {
  if (!value.trim()) return true
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed >= 0
}

function parseAllowedIps(value: string): string[] {
  return Array.from(new Set(value.split(/[\s,]+/).map((item) => item.trim()).filter(Boolean)))
}

function parsePolicyList(value: string): string[] {
  return Array.from(new Set(value.split(/[\s,]+/).map((item) => item.trim()).filter(Boolean)))
}

function formatUsd(value: number): string {
  return `$${value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 4 })}`
}

function rateLimitSummary(apiKey: ApiKey): string {
  const periodicLimits = apiKey.rateLimited ? [
    apiKey.fiveHourLimitUsd ? `5h ${formatUsd(apiKey.fiveHourLimitUsd)}` : '',
    apiKey.dailyLimitUsd ? `24h ${formatUsd(apiKey.dailyLimitUsd)}` : '',
    apiKey.weeklyLimitUsd ? `7天 ${formatUsd(apiKey.weeklyLimitUsd)}` : '',
    apiKey.monthlyLimitUsd ? `30天 ${formatUsd(apiKey.monthlyLimitUsd)}` : '',
  ] : []
  const limits = [
    ...periodicLimits,
    apiKey.spendLimitUsd ? `额度 ${formatUsd(apiKey.spendLimitUsd)}` : '',
  ].filter(Boolean)

  if (limits.length > 0) return limits.join(' · ')
  if (apiKey.ipRestricted) return 'IP 白名单'
  if (apiKey.rateLimited) return '周期费用限额已启用'
  return '遵循项目/系统策略'
}

function dateTimeLocalFromValue(value: string | null): string {
  if (!value) return ''
  const timestamp = /^\d+$/.test(value) ? Number(value) : new Date(value).getTime()
  if (!Number.isFinite(timestamp)) return ''
  const date = new Date(timestamp)
  const localDate = new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
  return localDate.toISOString().slice(0, 16)
}

function localDateTimeToMillis(value: string): string {
  if (!value) return ''
  const timestamp = new Date(value).getTime()
  return Number.isFinite(timestamp) ? String(timestamp) : ''
}
