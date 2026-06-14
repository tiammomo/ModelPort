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
import { Progress } from '@/components/ui/progress'
import { cn, formatDate, formatNumber } from '@/lib/utils'
import { paginateItems } from '@/lib/pagination'
import { CalendarClock, Copy, DollarSign, FolderKanban, KeyRound, Pencil, Plus, RotateCw, Search, ShieldCheck, ShieldOff, Trash2, Zap } from 'lucide-react'
import type { ApiKey, Team } from '@/types'
import { toast } from 'sonner'

const ALL = '__all__'
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
  const { data: apiKeys = [], isLoading, refetch } = useApiKeys()
  const { data: users = [] } = useUsers()
  const { data: teams = [] } = useTeams()
  const createApiKey = useCreateApiKey()
  const upsertTeam = useUpsertTeam()
  const revokeApiKey = useRevokeApiKey()
  const updateApiKey = useUpdateApiKey()
  const deleteApiKey = useDeleteApiKey()

  const [search, setSearch] = useState('')
  const [status, setStatus] = useState(ALL)
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

  const filteredKeys = useMemo(() => apiKeys.filter((key) => {
    const haystack = `${key.name} ${key.username || ''} ${key.keyPreview || key.keyPrefix} ${key.group || ''}`.toLowerCase()
    if (search && !haystack.includes(search.toLowerCase())) return false
    if (status !== ALL && key.status !== status) return false
    if (group === NO_GROUP && key.group) return false
    if (group !== ALL && group !== NO_GROUP && key.group !== group) return false
    return true
  }), [apiKeys, group, search, status])
  const keyWindow = paginateItems(filteredKeys, keysPage, keysPageSize)

  const activeKeys = apiKeys.filter((key) => key.status === 'active').length
  const revokedKeys = apiKeys.length - activeKeys
  const requestsToday = apiKeys.reduce((sum, key) => sum + (key.requestsToday ?? 0), 0)
  const tokensToday = apiKeys.reduce((sum, key) => sum + (key.tokensToday ?? 0), 0)
  const apiBaseUrl = import.meta.env.VITE_API_BASE_URL || window.location.origin

  const handleCreate = () => {
    const user = users.find((item) => item.id === form.userId)
    if (!user || !form.name.trim()) return
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
        setNewKey(key.key || null)
        setForm({ userId: '', name: '', group: '', teamId: '', allowedModels: '', allowedProviders: '' })
      },
      onError: (error) => toast.error(error instanceof Error ? error.message : '创建密钥失败'),
    })
  }

  const handleCreateTeam = () => {
    if (!teamForm.name.trim()) return
    upsertTeam.mutate({
      name: teamForm.name.trim(),
      dailyLimitUsd: parseUsdLimit(teamForm.dailyLimitUsd),
      monthlyLimitUsd: parseUsdLimit(teamForm.monthlyLimitUsd),
      allowedModels: parsePolicyList(teamForm.allowedModels),
      allowedProviders: parsePolicyList(teamForm.allowedProviders),
      status: 'active',
    }, {
      onSuccess: () => setTeamForm({ name: '', dailyLimitUsd: '', monthlyLimitUsd: '', allowedModels: '', allowedProviders: '' }),
      onError: (error) => toast.error(error instanceof Error ? error.message : '创建项目失败'),
    })
  }

  const openCreateDialog = () => {
    setNewKey(null)
    setShowCreateDialog(true)
  }

  const openEditDialog = (apiKey: ApiKey) => {
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
      setConfirmAction({
        title: '禁用 API 密钥',
        description: `禁用后，${apiKey.name} 将无法继续调用 API。`,
        confirmLabel: '禁用',
        destructive: true,
        onConfirm: () => revokeApiKey.mutate(apiKey.id, {
          onSettled: () => setConfirmAction(null),
          onError: (error) => toast.error(error instanceof Error ? error.message : '禁用密钥失败'),
        }),
      })
      return
    }

    updateApiKey.mutate({
      keyId: apiKey.id,
      data: { status: 'active' },
    })
  }

  const handleUpdate = () => {
    if (!editingKey || !editForm.name.trim()) return

    updateApiKey.mutate({
      keyId: editingKey.id,
      data: {
        name: editForm.name.trim(),
        group: editForm.group.trim(),
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
      },
    }, {
      onSuccess: closeEditDialog,
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新密钥失败'),
    })
  }

  const copyText = (text: string) => {
    void navigator.clipboard.writeText(text)
  }

  const mutationError = errorMessage(deleteApiKey.error || revokeApiKey.error || updateApiKey.error)

  const handleKeysPageChange = (page: number) => {
    setKeysPage(Math.min(Math.max(page, 1), keyWindow.totalPages))
  }

  const handleKeysPageSizeChange = (pageSize: number) => {
    setKeysPageSize(pageSize)
    setKeysPage(1)
  }

  return (
    <div className="space-y-6">
      <div className="rounded-md border bg-card p-4 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="text-2xl font-bold tracking-tight">API 密钥</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {formatNumber(activeKeys)} 个活跃 / {formatNumber(apiKeys.length)} 个总数 · 今日 {formatNumber(requestsToday)} 请求 · {formatNumber(tokensToday)} tokens
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <Button variant="outline" size="icon" onClick={() => refetch()} aria-label="刷新 API 密钥">
              <RotateCw className="h-4 w-4" />
            </Button>
            <Button onClick={openCreateDialog}>
              <Plus className="mr-2 h-4 w-4" />
              创建密钥
            </Button>
          </div>
        </div>

        <div className="mt-5 flex flex-wrap items-center gap-3">
          <div className="relative min-w-[260px] flex-1">
            <Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
            <Input
              className="h-11 rounded-md bg-background pl-9"
              placeholder="搜索名称、用户或 Key..."
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
            <SelectTrigger className="h-11 w-[180px] bg-background"><SelectValue placeholder="全部分组" /></SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL}>全部分组</SelectItem>
              <SelectItem value={NO_GROUP}>无分组</SelectItem>
              {groups.map((item) => (
                <SelectItem key={item} value={item || ''}>{item}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={status}
            onValueChange={(value) => {
              setStatus(value)
              setKeysPage(1)
            }}
          >
            <SelectTrigger className="h-11 w-[160px] bg-background"><SelectValue placeholder="全部状态" /></SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL}>全部状态</SelectItem>
              <SelectItem value="active">活跃</SelectItem>
              <SelectItem value="revoked">已禁用</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="mt-4 flex flex-wrap gap-2">
          <EndpointPill label="API 端点" tag="默认" value={`${apiBaseUrl}/v1/messages`} onCopy={copyText} />
          <EndpointPill label="管理端点" value={`${apiBaseUrl}/admin/api-keys`} onCopy={copyText} />
          <div className="inline-flex h-8 items-center rounded-md border bg-background px-3 text-xs text-muted-foreground">
            已禁用 {formatNumber(revokedKeys)}
          </div>
        </div>
      </div>

      <Card className="overflow-hidden">
        <div className="border-b bg-muted/20 px-5 py-4">
          <TeamStrip
            teams={teams}
            form={teamForm}
            pending={upsertTeam.isPending}
            error={errorMessage(upsertTeam.error)}
            onChange={(patch) => setTeamForm((current) => ({ ...current, ...patch }))}
            onCreate={handleCreateTeam}
          />
        </div>
        {mutationError && (
          <div className="border-b px-5 py-3">
            <ErrorNotice message={mutationError} />
          </div>
        )}
        <CardContent className="p-0">
          <Table className="min-w-[1360px]">
            <TableHeader className="bg-muted/40">
              <TableRow className="hover:bg-transparent">
                <TableHead className="sticky left-0 z-20 w-[190px] border-r bg-muted/95 px-5">名称</TableHead>
                <TableHead className="w-[190px]">API 密钥</TableHead>
                <TableHead className="w-[150px]">用户</TableHead>
                <TableHead className="w-[240px]">项目/分组</TableHead>
                <TableHead className="w-[165px] text-right">用量</TableHead>
                <TableHead className="w-[220px]">速率限制</TableHead>
                <TableHead className="w-[150px]">过期时间</TableHead>
                <TableHead className="w-[110px]">状态</TableHead>
                <TableHead className="w-[180px]">上次使用时间</TableHead>
                <TableHead className="w-[170px]">创建时间</TableHead>
                <TableHead className="sticky right-0 z-20 w-[230px] border-l bg-muted/95 text-center">操作</TableHead>
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
                      title="暂无 API 密钥"
                      description={search || status !== ALL || group !== ALL ? '没有匹配的 API 密钥，请调整筛选条件' : '点击「创建密钥」按钮签发第一个 API 密钥'}
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
                        onClick={() => copyText(key.keyPreview || key.keyPrefix)}
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
                    <SoftStatusBadge status={key.status} />
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {key.lastUsedAt ? formatDate(key.lastUsedAt) : '从未使用'}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {formatDate(key.createdAt)}
                  </TableCell>
                  <TableCell className="sticky right-0 z-10 border-l bg-card">
                    <div className="flex justify-center gap-1">
                      <ActionButton
                        icon={Copy}
                        label="复制"
                        onClick={() => copyText(key.keyPreview || key.keyPrefix)}
                      />
                      <ActionButton
                        icon={Pencil}
                        label="编辑"
                        onClick={() => openEditDialog(key)}
                      />
                      <ActionButton
                        icon={key.status === 'active' ? ShieldOff : ShieldCheck}
                        label={key.status === 'active' ? '禁用' : '恢复'}
                        className={key.status === 'active' ? undefined : 'text-emerald-600 hover:text-emerald-700'}
                        disabled={revokeApiKey.isPending || updateApiKey.isPending}
                        onClick={() => handleToggleStatus(key)}
                      />
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-12 w-11 flex-col gap-1 px-1 text-xs text-destructive"
                        onClick={() => setConfirmAction({
                          title: '删除 API 密钥',
                          description: `删除 ${key.name} 后无法恢复，相关调用记录仍会保留。`,
                          confirmLabel: '删除',
                          destructive: true,
                          onConfirm: () => deleteApiKey.mutate(key.id, {
                            onSettled: () => setConfirmAction(null),
                            onError: (error) => toast.error(error instanceof Error ? error.message : '删除密钥失败'),
                          }),
                        })}
                      >
                        <Trash2 className="h-4 w-4" />
                        删除
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
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
      </Card>

      <Dialog open={showCreateDialog} onOpenChange={(open) => { setShowCreateDialog(open); if (!open) setNewKey(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>创建 API 密钥</DialogTitle>
            <DialogDescription>为用户签发 ModelPort 密钥。完整密钥只会展示一次。</DialogDescription>
          </DialogHeader>

          {newKey ? (
            <div className="space-y-2">
              <Label>新 API 密钥</Label>
              <div className="flex gap-2">
                <Input value={newKey} readOnly className="font-mono text-xs" />
                <Button variant="outline" size="icon" onClick={() => copyText(newKey)}>
                  <Copy className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              {createApiKey.error && <ErrorNotice message={errorMessage(createApiKey.error)} />}
              <div className="space-y-2">
                <Label>用户</Label>
                <Select value={form.userId} onValueChange={(value) => setForm({ ...form, userId: value })}>
                  <SelectTrigger><SelectValue placeholder="选择用户" /></SelectTrigger>
                  <SelectContent>
                    {users.map((user) => (
                      <SelectItem key={user.id} value={user.id}>{user.username}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label>名称</Label>
                <Input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="Claude Code" />
              </div>
              <div className="space-y-2">
                <Label>分组</Label>
                <Input value={form.group} onChange={(event) => setForm({ ...form, group: event.target.value })} placeholder="研发池 / Claude Code / CI" />
              </div>
              <div className="space-y-2">
                <Label>项目/团队</Label>
                <Select value={form.teamId || NO_TEAM} onValueChange={(value) => setForm({ ...form, teamId: value === NO_TEAM ? '' : value })}>
                  <SelectTrigger><SelectValue placeholder="选择项目" /></SelectTrigger>
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
                  <Label>允许模型</Label>
                  <Input value={form.allowedModels} onChange={(event) => setForm({ ...form, allowedModels: event.target.value })} placeholder="mimo*, claude-sonnet*" />
                </div>
                <div className="space-y-2">
                  <Label>允许供应商</Label>
                  <Input value={form.allowedProviders} onChange={(event) => setForm({ ...form, allowedProviders: event.target.value })} placeholder="mimo, openai" />
                </div>
              </div>
            </div>
          )}

          <DialogFooter>
            {newKey ? (
              <Button onClick={() => setShowCreateDialog(false)}>完成</Button>
            ) : (
              <>
                <Button variant="outline" onClick={() => setShowCreateDialog(false)}>取消</Button>
                <Button onClick={handleCreate} disabled={!form.userId || !form.name.trim() || createApiKey.isPending}>
                  <KeyRound className="mr-2 h-4 w-4" />
                  创建
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!editingKey} onOpenChange={(open) => { if (!open) closeEditDialog() }}>
        <DialogContent className="max-h-[90vh] max-w-[640px] gap-0 overflow-hidden p-0 sm:rounded-2xl">
          <DialogHeader className="border-b px-7 py-5">
            <DialogTitle className="text-xl">编辑密钥</DialogTitle>
            <DialogDescription className="sr-only">更新 API 密钥设置</DialogDescription>
          </DialogHeader>

          <div className="max-h-[calc(90vh-146px)] space-y-5 overflow-y-auto px-7 py-5">
            {updateApiKey.error && <ErrorNotice message={errorMessage(updateApiKey.error)} />}
            <div className="space-y-2">
              <Label>名称</Label>
              <Input
                className="h-12 rounded-xl"
                value={editForm.name}
                onChange={(event) => updateEditForm({ name: event.target.value })}
              />
            </div>

            <div className="space-y-2">
              <Label>分组</Label>
              <Select
                value={editForm.group || NO_GROUP}
                onValueChange={(value) => updateEditForm({ group: value === NO_GROUP ? '' : value })}
              >
                <SelectTrigger className="h-12 rounded-xl bg-background">
                  <SelectValue placeholder="选择分组" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_GROUP}>无分组</SelectItem>
                  {Array.from(new Set([...groups, editForm.group].filter((item): item is string => Boolean(item)))).map((item) => (
                    <SelectItem key={item} value={item}>{item}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>项目/团队</Label>
              <Select
                value={editForm.teamId || NO_TEAM}
                onValueChange={(value) => updateEditForm({ teamId: value === NO_TEAM ? '' : value })}
              >
                <SelectTrigger className="h-12 rounded-xl bg-background">
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
                <Label>允许模型</Label>
                <Input
                  className="h-12 rounded-xl"
                  value={editForm.allowedModels}
                  onChange={(event) => updateEditForm({ allowedModels: event.target.value })}
                  placeholder="留空表示不限"
                />
              </div>
              <div className="space-y-2">
                <Label>允许供应商</Label>
                <Input
                  className="h-12 rounded-xl"
                  value={editForm.allowedProviders}
                  onChange={(event) => updateEditForm({ allowedProviders: event.target.value })}
                  placeholder="留空表示不限"
                />
              </div>
            </div>

            <div className="space-y-2">
              <Label>状态</Label>
              <Select value={editForm.status} onValueChange={(value) => updateEditForm({ status: value as ApiKey['status'] })}>
                <SelectTrigger className="h-12 rounded-xl bg-background">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="active">启用</SelectItem>
                  <SelectItem value="revoked">已禁用</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <SettingSwitch
              label="IP 限制"
              checked={editForm.ipRestricted}
              onCheckedChange={(checked) => updateEditForm({ ipRestricted: checked })}
            />

            {editForm.ipRestricted && (
              <div className="space-y-2">
                <Label>允许 IP / CIDR</Label>
                <Input
                  className="h-12 rounded-xl"
                  value={editForm.allowedIps}
                  onChange={(event) => updateEditForm({ allowedIps: event.target.value })}
                  placeholder="127.0.0.1, 10.0.0.0/8"
                />
                <p className="text-xs text-muted-foreground">开启后，只有匹配列表的客户端 IP 可以使用此密钥。</p>
              </div>
            )}

            <div className="space-y-2">
              <Label>额度限制</Label>
              <CurrencyInput
                value={editForm.spendLimitUsd}
                onChange={(value) => updateEditForm({ spendLimitUsd: value })}
                placeholder="输入 USD 额度限制"
              />
            </div>

            <SettingSwitch
              label="速率限制"
              checked={editForm.rateLimited}
              onCheckedChange={(checked) => updateEditForm({ rateLimited: checked })}
            />

            <LimitField
              label="5 小时限额 (USD)"
              value={editForm.fiveHourLimitUsd}
              onChange={(value) => updateEditForm({ fiveHourLimitUsd: value })}
            />

            <LimitField
              label="日限额 (USD)"
              value={editForm.dailyLimitUsd}
              onChange={(value) => updateEditForm({ dailyLimitUsd: value })}
              used={0}
            />

            <LimitField
              label="7 天限额 (USD)"
              value={editForm.weeklyLimitUsd}
              onChange={(value) => updateEditForm({ weeklyLimitUsd: value })}
            />

            <LimitField
              label="月限额 (USD)"
              value={editForm.monthlyLimitUsd}
              onChange={(value) => updateEditForm({ monthlyLimitUsd: value })}
            />

            <div className="space-y-2">
              <Label>过期时间</Label>
              <Input
                className="h-12 rounded-xl"
                type="datetime-local"
                value={editForm.expiresAt}
                onChange={(event) => updateEditForm({ expiresAt: event.target.value })}
              />
            </div>
          </div>

          <DialogFooter className="border-t bg-background px-7 py-5">
            <Button variant="outline" onClick={closeEditDialog}>取消</Button>
            <Button onClick={handleUpdate} disabled={!editForm.name.trim() || updateApiKey.isPending}>
              更新
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={!!confirmAction}
        title={confirmAction?.title || ''}
        description={confirmAction?.description || ''}
        confirmLabel={confirmAction?.confirmLabel}
        destructive={confirmAction?.destructive}
        pending={deleteApiKey.isPending || revokeApiKey.isPending}
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

function TeamStrip({
  teams,
  form,
  pending,
  error,
  onChange,
  onCreate,
}: {
  teams: Team[]
  form: TeamForm
  pending: boolean
  error: string
  onChange: (patch: Partial<TeamForm>) => void
  onCreate: () => void
}) {
  const [teamPage, setTeamPage] = useState(1)
  const [teamPageSize, setTeamPageSize] = useState(4)
  const teamWindow = paginateItems(teams, teamPage, teamPageSize)

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold">项目/团队</h3>
          <p className="text-xs text-muted-foreground">按项目管理预算、模型和供应商访问范围。</p>
        </div>
        <div className="flex flex-wrap gap-2">
          {teamWindow.items.map((team) => (
            <Badge key={team.id} variant="outline" className="gap-1.5 rounded-md bg-background px-2.5 py-1">
              <FolderKanban className="h-3.5 w-3.5" />
              {team.name}
              <span className="text-muted-foreground">{team.activeApiKeys} keys</span>
            </Badge>
          ))}
          {teams.length === 0 && <span className="text-xs text-muted-foreground">暂无项目</span>}
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
      {error && <ErrorNotice message={error} />}
      <div className="grid gap-2 md:grid-cols-[1.1fr_.7fr_.7fr_1fr_1fr_auto]">
        <Input value={form.name} onChange={(event) => onChange({ name: event.target.value })} placeholder="项目名称" />
        <Input type="number" min="0" value={form.dailyLimitUsd} onChange={(event) => onChange({ dailyLimitUsd: event.target.value })} placeholder="日预算 USD" />
        <Input type="number" min="0" value={form.monthlyLimitUsd} onChange={(event) => onChange({ monthlyLimitUsd: event.target.value })} placeholder="月预算 USD" />
        <Input value={form.allowedModels} onChange={(event) => onChange({ allowedModels: event.target.value })} placeholder="模型白名单" />
        <Input value={form.allowedProviders} onChange={(event) => onChange({ allowedProviders: event.target.value })} placeholder="供应商白名单" />
        <Button onClick={onCreate} disabled={!form.name.trim() || pending}>
          <Plus className="mr-2 h-4 w-4" />
          添加
        </Button>
      </div>
    </div>
  )
}

function GroupCell({ group, teamName }: { group?: string | null; teamName?: string | null }) {
  if (!group && !teamName) {
    return (
      <span className="text-sm text-muted-foreground">无分组</span>
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
  const hasCustomLimit = summary !== '未单独限制'

  return (
    <div className="min-w-[190px] space-y-2">
      <div className="flex items-center gap-2 whitespace-nowrap">
        <span
          className={cn(
            'inline-flex h-6 items-center rounded-md px-2 text-xs font-medium',
            hasCustomLimit
              ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300'
              : 'bg-muted text-muted-foreground'
          )}
        >
          {hasCustomLimit ? '自定义' : '继承全局'}
        </span>
        <span className="min-w-0 truncate text-xs font-medium text-foreground/75">{summary}</span>
      </div>
      <div className="flex items-center gap-2">
        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
          <div
            className={cn('h-full rounded-full', hasCustomLimit ? 'bg-emerald-500' : 'bg-muted-foreground/20')}
            style={{ width: hasCustomLimit ? '42%' : '18%' }}
          />
        </div>
        <span className="w-8 text-right text-[11px] text-muted-foreground">{hasCustomLimit ? '限制' : '默认'}</span>
      </div>
    </div>
  )
}

function ExpiresCell({ value }: { value: string | null }) {
  if (!value) {
    return <span className="text-sm text-muted-foreground">永久有效</span>
  }

  return (
    <div className="flex items-center gap-2 text-sm">
      <CalendarClock className="h-4 w-4 text-muted-foreground" />
      <span>{formatDate(value)}</span>
    </div>
  )
}

function SoftStatusBadge({ status }: { status: ApiKey['status'] }) {
  if (status === 'active') {
    return (
      <Badge
        variant="outline"
        className="border-emerald-200 bg-emerald-50 text-emerald-700 shadow-none dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300"
      >
        活跃
      </Badge>
    )
  }

  return <StatusBadge status={status} />
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

function ErrorNotice({ message }: { message: string }) {
  if (!message) return null
  return (
    <div className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      {message}
    </div>
  )
}

function errorMessage(error: unknown): string {
  if (!error) return ''
  return error instanceof Error ? error.message : String(error)
}

function SettingSwitch({
  label,
  checked,
  onCheckedChange,
}: {
  label: string
  checked: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <div className="flex min-h-10 items-center justify-between gap-4">
      <Label className="text-base font-normal">{label}</Label>
      <Switch checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  )
}

function LimitField({
  label,
  value,
  used,
  onChange,
}: {
  label: string
  value: string
  used?: number
  onChange: (value: string) => void
}) {
  const limit = parseUsdLimit(value)
  const percent = limit > 0 && used !== undefined ? Math.min((used / limit) * 100, 100) : 0

  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <CurrencyInput value={value} onChange={onChange} placeholder="0" />
      {used !== undefined && (
        <div className="space-y-2">
          <div className="rounded-md bg-muted px-4 py-2 text-sm text-muted-foreground">
            <span className="text-foreground">{formatUsd(used)}</span>
            <span> / {formatUsd(limit)}</span>
          </div>
          <Progress value={percent} className="h-1.5 bg-muted" />
        </div>
      )}
    </div>
  )
}

function CurrencyInput({
  value,
  placeholder,
  onChange,
}: {
  value: string
  placeholder?: string
  onChange: (value: string) => void
}) {
  return (
    <div className="relative">
      <DollarSign className="absolute left-4 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        className="h-12 rounded-xl pl-10"
        inputMode="decimal"
        min="0"
        step="0.0001"
        type="number"
        value={value}
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
  const limits = [
    apiKey.dailyLimitUsd ? `日 ${formatUsd(apiKey.dailyLimitUsd)}` : '',
    apiKey.weeklyLimitUsd ? `7天 ${formatUsd(apiKey.weeklyLimitUsd)}` : '',
    apiKey.monthlyLimitUsd ? `月 ${formatUsd(apiKey.monthlyLimitUsd)}` : '',
    apiKey.fiveHourLimitUsd ? `5h ${formatUsd(apiKey.fiveHourLimitUsd)}` : '',
    apiKey.spendLimitUsd ? `额度 ${formatUsd(apiKey.spendLimitUsd)}` : '',
  ].filter(Boolean)

  if (limits.length > 0) return limits[0]
  if (apiKey.ipRestricted) return 'IP 白名单'
  if (apiKey.rateLimited) return '速率已启用'
  return '未单独限制'
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
