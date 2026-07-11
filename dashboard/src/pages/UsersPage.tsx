import { useMemo, useState } from 'react'
import { useUsers, useCreateUser, useUpdateUser, useDeleteUser, useUserApiKeys, useCreateApiKey, useRevokeApiKey } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { ConfirmDialog } from '@/components/shared/ConfirmDialog'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { EmptyState } from '@/components/shared/EmptyState'
import { PaginationBar } from '@/components/shared/PaginationBar'
import { Skeleton } from '@/components/shared/Skeleton'
import { toast } from 'sonner'
import { Card, CardContent, CardFooter } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn, formatDate, formatNumber } from '@/lib/utils'
import { paginateItems } from '@/lib/pagination'
import { ROLE_LABELS } from '@/lib/constants'
import { filterUsers, isCreateUserFormValid, isUserEmailValid, isUserFilterActive, type UserRoleFilter, type UserStatusFilter } from '@/features/users/user-view'
import { useAuthStore } from '@/stores'
import { Copy, Key, MoreHorizontal, Pencil, RotateCw, Search, ShieldOff, Trash2, UserPlus, UsersRound, X } from 'lucide-react'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import type { User, UserRole } from '@/types'

type UserStatus = User['status']

const emptyEditForm = {
  email: '',
  password: '',
  role: 'user' as UserRole,
  status: 'active' as UserStatus,
}

interface ConfirmAction {
  title: string
  description: string
  confirmLabel: string
  destructive?: boolean
  onConfirm: () => void
}

export function UsersPage() {
  const currentUser = useAuthStore((state) => state.currentUser)
  const { data: users = [], isLoading, isError, error, refetch } = useUsers()
  const createUser = useCreateUser()
  const updateUser = useUpdateUser()
  const deleteUser = useDeleteUser()
  const createApiKey = useCreateApiKey()
  const revokeApiKey = useRevokeApiKey()

  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [showDetailUser, setShowDetailUser] = useState<string | null>(null)
  const [editingUser, setEditingUser] = useState<User | null>(null)
  const [confirmAction, setConfirmAction] = useState<ConfirmAction | null>(null)
  const [newKeyResult, setNewKeyResult] = useState<string | null>(null)
  const [search, setSearch] = useState('')
  const [roleFilter, setRoleFilter] = useState<UserRoleFilter>('all')
  const [statusFilter, setStatusFilter] = useState<UserStatusFilter>('all')
  const [usersPage, setUsersPage] = useState(1)
  const [usersPageSize, setUsersPageSize] = useState(20)
  const [userKeysPage, setUserKeysPage] = useState(1)
  const [userKeysPageSize, setUserKeysPageSize] = useState(10)

  const [form, setForm] = useState({
    username: '',
    email: '',
    password: '',
    role: 'user' as UserRole,
    status: 'active' as UserStatus,
  })
  const [editForm, setEditForm] = useState(emptyEditForm)
  const [keyName, setKeyName] = useState('')

  const {
    data: userApiKeys = [],
    isLoading: userKeysLoading,
    isError: userKeysError,
    error: userKeysQueryError,
    refetch: refetchUserKeys,
  } = useUserApiKeys(showDetailUser || '')
  const filteredUsers = useMemo(() => filterUsers(users, {
    search,
    role: roleFilter,
    status: statusFilter,
  }), [roleFilter, search, statusFilter, users])
  const usersWindow = useMemo(
    () => paginateItems(filteredUsers, usersPage, usersPageSize),
    [filteredUsers, usersPage, usersPageSize],
  )
  const userKeysWindow = useMemo(
    () => paginateItems(userApiKeys, userKeysPage, userKeysPageSize),
    [userApiKeys, userKeysPage, userKeysPageSize],
  )
  const activeUsers = users.filter((user) => user.status === 'active').length
  const adminUsers = users.filter((user) => user.role === 'admin' && user.status === 'active').length
  const disabledUsers = users.length - activeUsers
  const detailUser = users.find((user) => user.id === showDetailUser)
  const filtersActive = isUserFilterActive({ search, role: roleFilter, status: statusFilter })
  const editingUserAccessLocked = Boolean(editingUser && (
    editingUser.id === currentUser?.id
    || (editingUser.role === 'admin' && editingUser.status === 'active' && adminUsers <= 1)
  ))

  const canDeleteUser = (user: User) => user.id !== currentUser?.id && !(user.role === 'admin' && adminUsers <= 1)

  const handleCreateUser = () => {
    if (!isCreateUserFormValid(form)) return
    createUser.mutate({
      ...form,
      username: form.username.trim(),
      email: form.email.trim(),
    }, {
      onSuccess: () => {
        setShowCreateDialog(false)
        setForm({ username: '', email: '', password: '', role: 'user', status: 'active' })
        toast.success('用户已创建')
      },
      onError: (mutationError) => toast.error(errorMessage(mutationError) || '创建用户失败'),
    })
  }

  const openEditUser = (user: User) => {
    updateUser.reset()
    setEditingUser(user)
    setEditForm({ email: user.email, password: '', role: user.role, status: user.status })
  }

  const closeEditUser = () => {
    setEditingUser(null)
    setEditForm(emptyEditForm)
  }

  const persistUserUpdate = () => {
    if (!editingUser || !isUserEmailValid(editForm.email)) return
    if (editingUserAccessLocked && (editForm.role !== editingUser.role || editForm.status !== editingUser.status)) return
    updateUser.mutate({
      id: editingUser.id,
      data: {
        email: editForm.email.trim(),
        role: editForm.role,
        status: editForm.status,
        ...(editForm.password ? { password: editForm.password } : {}),
      },
    }, {
      onSuccess: () => {
        closeEditUser()
        toast.success('用户设置已保存')
      },
      onSettled: () => setConfirmAction(null),
      onError: (mutationError) => toast.error(errorMessage(mutationError) || '更新用户失败'),
    })
  }

  const handleUpdateUser = () => {
    if (!editingUser || !isUserEmailValid(editForm.email)) return
    if (editingUser.status === 'active' && editForm.status !== 'active') {
      setConfirmAction({
        title: `${editForm.status === 'suspended' ? '暂停' : '禁用'}用户 ${editingUser.username}？`,
        description: '该用户会立即退出登录，现有 API 密钥和配额将被回收，之后恢复用户状态也不会自动恢复这些资源。',
        confirmLabel: editForm.status === 'suspended' ? '确认暂停' : '确认禁用',
        destructive: true,
        onConfirm: persistUserUpdate,
      })
      return
    }
    persistUserUpdate()
  }

  const handleCreateKey = () => {
    if (!showDetailUser || !detailUser || detailUser.status !== 'active') return
    createApiKey.mutate({
      userId: showDetailUser,
      username: detailUser.username,
      name: keyName.trim() || '默认密钥',
    }, {
      onSuccess: (key) => {
        setKeyName('')
        if (key.key) {
          setNewKeyResult(key.key)
          toast.success('API 密钥已生成')
        } else {
          toast.error('密钥已创建，但服务端未返回完整密钥；请禁用该密钥后重新生成')
        }
      },
      onError: (mutationError) => toast.error(errorMessage(mutationError) || '生成密钥失败'),
    })
  }

  const mutationError = errorMessage(deleteUser.error || revokeApiKey.error)

  const openUserKeys = (userId: string) => {
    createApiKey.reset()
    setShowDetailUser(userId)
    setUserKeysPage(1)
    setNewKeyResult(null)
    setKeyName('')
  }

  const openCreateUserDialog = () => {
    createUser.reset()
    setShowCreateDialog(true)
  }

  const resetFilters = () => {
    setSearch('')
    setRoleFilter('all')
    setStatusFilter('all')
    setUsersPage(1)
  }

  const copyText = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text)
      toast.success(`${label}已复制`)
    } catch {
      toast.error('复制失败，请手动复制')
    }
  }

  const requestDeleteUser = (user: User) => {
    setConfirmAction({
      title: `删除用户 ${user.username}？`,
      description: `该用户将无法登录，其 ${formatNumber(user.apiKeyCount)} 个 API 密钥会被回收。请求日志仍会保留，此操作不可撤销。`,
      confirmLabel: '永久删除',
      destructive: true,
      onConfirm: () => deleteUser.mutate(user.id, {
        onSuccess: () => toast.success('用户已删除'),
        onSettled: () => setConfirmAction(null),
        onError: (mutationError) => toast.error(errorMessage(mutationError) || '删除用户失败'),
      }),
    })
  }

  const requestRevokeKey = (keyId: string, keyNameValue: string) => {
    setConfirmAction({
      title: `禁用密钥 ${keyNameValue}？`,
      description: '禁用立即生效，使用该密钥的客户端会停止调用；历史请求记录不会删除。',
      confirmLabel: '禁用密钥',
      destructive: true,
      onConfirm: () => revokeApiKey.mutate(keyId, {
        onSuccess: () => toast.success('API 密钥已禁用'),
        onSettled: () => setConfirmAction(null),
        onError: (mutationError) => toast.error(errorMessage(mutationError) || '禁用密钥失败'),
      }),
    })
  }

  if (isLoading) return <LoadingPage />

  return (
    <div className="space-y-6">
      <PageHeader
        title="用户管理"
        description="管理登录身份、角色权限和用户所属 API 密钥"
        action={{ label: '新建用户', onClick: openCreateUserDialog, icon: UserPlus }}
      />

      {isError && (
        <ErrorNotice
          message={`用户列表加载失败：${errorMessage(error)}`}
          actionLabel="重新加载"
          onAction={() => void refetch()}
        />
      )}
      {mutationError && <ErrorNotice message={mutationError} />}

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3" aria-label="用户概览">
        <SummaryCard label="全部用户" value={users.length} hint="已创建的登录身份" />
        <SummaryCard label="可用用户" value={activeUsers} hint={`${disabledUsers} 个不可登录`} />
        <SummaryCard label="活跃管理员" value={adminUsers} hint="具备完整管理权限" tone={adminUsers <= 1 ? 'warning' : 'default'} className="col-span-2 sm:col-span-1" />
      </div>

      <Card className="overflow-hidden">
        <div className="border-b bg-muted/20 p-4">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
            <div className="relative min-w-0 flex-1">
              <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                aria-label="搜索用户"
                className="bg-background pl-9"
                placeholder="搜索用户名、邮箱或用户 ID"
                value={search}
                onChange={(event) => { setSearch(event.target.value); setUsersPage(1) }}
              />
            </div>
            <Select value={roleFilter} onValueChange={(value) => { setRoleFilter(value as UserRoleFilter); setUsersPage(1) }}>
              <SelectTrigger className="w-full bg-background lg:w-[150px]" aria-label="按角色筛选">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">全部角色</SelectItem>
                <SelectItem value="admin">管理员</SelectItem>
                <SelectItem value="user">普通用户</SelectItem>
                <SelectItem value="viewer">只读用户</SelectItem>
              </SelectContent>
            </Select>
            <Select value={statusFilter} onValueChange={(value) => { setStatusFilter(value as UserStatusFilter); setUsersPage(1) }}>
              <SelectTrigger className="w-full bg-background lg:w-[150px]" aria-label="按状态筛选">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="all">全部状态</SelectItem>
                <SelectItem value="active">可用</SelectItem>
                <SelectItem value="disabled">已禁用</SelectItem>
                <SelectItem value="suspended">已暂停</SelectItem>
              </SelectContent>
            </Select>
            {filtersActive && (
              <Button variant="ghost" onClick={resetFilters} className="justify-start lg:justify-center">
                <X className="mr-2 h-4 w-4" />清除筛选
              </Button>
            )}
          </div>
          <p className="mt-2 text-xs text-muted-foreground" aria-live="polite">
            显示 {formatNumber(filteredUsers.length)} / {formatNumber(users.length)} 个用户
          </p>
        </div>

        <CardContent className="p-0">
          {filteredUsers.length === 0 ? (
            <EmptyState
              icon={UsersRound}
              title={isError ? '无法加载用户' : filtersActive ? '没有匹配的用户' : '暂无用户'}
              description={isError ? '检查网络或服务状态后重新加载。' : filtersActive ? '尝试清除筛选或更换关键词。' : '创建第一个用户并为其分配合适的角色。'}
              action={isError
                ? <Button variant="outline" onClick={() => void refetch()}>重新加载</Button>
                : filtersActive
                ? <Button variant="outline" onClick={resetFilters}>清除筛选</Button>
                : <Button onClick={openCreateUserDialog}>新建用户</Button>}
            />
          ) : (
            <>
              <div className="hidden md:block">
                <Table className="min-w-[980px]">
                  <TableHeader>
                    <TableRow>
                      <TableHead>用户</TableHead>
                      <TableHead>角色</TableHead>
                      <TableHead>状态</TableHead>
                      <TableHead className="text-center">API 密钥</TableHead>
                      <TableHead className="text-right">24h 请求</TableHead>
                      <TableHead>最后登录</TableHead>
                      <TableHead className="w-12"><span className="sr-only">操作</span></TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {usersWindow.items.map((user) => (
                      <TableRow key={user.id}>
                        <TableCell>
                          <div className="space-y-0.5">
                            <p className="font-medium">{user.username}</p>
                            <p className="text-xs text-muted-foreground">{user.email}</p>
                          </div>
                        </TableCell>
                        <TableCell><Badge variant={roleBadgeVariant(user.role)}>{ROLE_LABELS[user.role]}</Badge></TableCell>
                        <TableCell><StatusBadge status={user.status} /></TableCell>
                        <TableCell className="text-center tabular-nums">{formatNumber(user.apiKeyCount)}</TableCell>
                        <TableCell className="text-right tabular-nums">{formatNumber(user.requestCount24h)}</TableCell>
                        <TableCell className="text-sm text-muted-foreground">
                          {user.lastLoginAt ? formatDate(user.lastLoginAt) : '从未登录'}
                        </TableCell>
                        <TableCell>
                          <UserActionMenu user={user} canDelete={canDeleteUser(user)} onEdit={openEditUser} onKeys={openUserKeys} onDelete={requestDeleteUser} />
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>

              <div className="divide-y md:hidden">
                {usersWindow.items.map((user) => (
                  <article key={user.id} className="space-y-4 p-4">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <h3 className="truncate font-semibold">{user.username}</h3>
                        <p className="truncate text-sm text-muted-foreground">{user.email}</p>
                      </div>
                      <StatusBadge status={user.status} />
                    </div>
                    <div className="grid grid-cols-3 gap-2 rounded-md bg-muted/50 p-3 text-center text-xs">
                      <div><p className="text-muted-foreground">角色</p><p className="mt-1 font-medium">{ROLE_LABELS[user.role]}</p></div>
                      <div><p className="text-muted-foreground">密钥</p><p className="mt-1 font-medium">{formatNumber(user.apiKeyCount)}</p></div>
                      <div><p className="text-muted-foreground">24h 请求</p><p className="mt-1 font-medium">{formatNumber(user.requestCount24h)}</p></div>
                    </div>
                    <p className="text-xs text-muted-foreground">最后登录：{user.lastLoginAt ? formatDate(user.lastLoginAt) : '从未登录'}</p>
                    <div className="grid grid-cols-3 gap-2">
                      <Button variant="outline" size="sm" onClick={() => openEditUser(user)}><Pencil className="mr-1.5 h-4 w-4" />编辑</Button>
                      <Button variant="outline" size="sm" onClick={() => openUserKeys(user.id)}><Key className="mr-1.5 h-4 w-4" />密钥</Button>
                      <Button
                        variant="outline"
                        size="sm"
                        className="text-destructive"
                        disabled={!canDeleteUser(user)}
                        aria-label={canDeleteUser(user) ? `删除用户 ${user.username}` : `无法删除当前账号或最后一名管理员 ${user.username}`}
                        onClick={() => requestDeleteUser(user)}
                      ><Trash2 className="mr-1.5 h-4 w-4" />删除</Button>
                    </div>
                  </article>
                ))}
              </div>
            </>
          )}
        </CardContent>
        {filteredUsers.length > 0 && (
          <CardFooter className="border-t px-4 py-3">
            <PaginationBar
              total={filteredUsers.length}
              page={usersWindow.currentPage}
              pageSize={usersPageSize}
              totalPages={usersWindow.totalPages}
              start={usersWindow.start}
              end={usersWindow.end}
              totalLabel="个用户"
              onPageChange={(page) => setUsersPage(Math.min(Math.max(page, 1), usersWindow.totalPages))}
              onPageSizeChange={(pageSize) => { setUsersPageSize(pageSize); setUsersPage(1) }}
            />
          </CardFooter>
        )}
      </Card>

      <Dialog open={showCreateDialog} onOpenChange={setShowCreateDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建用户</DialogTitle>
            <DialogDescription>创建登录身份。后续可在用户详情中签发 API 密钥。</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {createUser.error && <ErrorNotice message={errorMessage(createUser.error)} />}
            <FormField id="create-username" label="用户名" required>
              <Input id="create-username" required value={form.username} onChange={(event) => setForm({ ...form, username: event.target.value })} placeholder="例如：alice" autoComplete="username" />
            </FormField>
            <FormField id="create-email" label="邮箱" required>
              <Input id="create-email" required type="email" value={form.email} onChange={(event) => setForm({ ...form, email: event.target.value })} placeholder="alice@example.com" autoComplete="email" />
              {form.email && !isUserEmailValid(form.email) && <p role="alert" className="text-xs text-destructive">请输入有效的邮箱地址。</p>}
            </FormField>
            <FormField id="create-password" label="初始密码" required description="至少 12 个字符。请通过安全渠道交给用户，并建议首次登录后更换。">
              <Input id="create-password" required type="password" value={form.password} onChange={(event) => setForm({ ...form, password: event.target.value })} placeholder="至少 12 个字符" autoComplete="new-password" aria-describedby="create-password-help" />
            </FormField>
            <FormField id="create-role" label="角色" description={roleDescription(form.role)}>
              <Select value={form.role} onValueChange={(value) => setForm({ ...form, role: value as UserRole })}>
                <SelectTrigger id="create-role" aria-describedby="create-role-help"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="admin">管理员</SelectItem>
                  <SelectItem value="user">普通用户</SelectItem>
                  <SelectItem value="viewer">只读用户</SelectItem>
                </SelectContent>
              </Select>
            </FormField>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowCreateDialog(false)}>取消</Button>
            <Button onClick={handleCreateUser} disabled={!isCreateUserFormValid(form) || createUser.isPending}>
              {createUser.isPending ? '创建中…' : '创建用户'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!editingUser} onOpenChange={(open) => { if (!open) closeEditUser() }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>编辑用户</DialogTitle>
            <DialogDescription>角色和状态变更会立即影响用户权限。</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {updateUser.error && <ErrorNotice message={errorMessage(updateUser.error)} />}
            <FormField id="edit-username" label="用户名">
              <Input id="edit-username" value={editingUser?.username || ''} disabled />
            </FormField>
            <FormField id="edit-email" label="邮箱" required>
              <Input id="edit-email" required type="email" value={editForm.email} onChange={(event) => setEditForm({ ...editForm, email: event.target.value })} autoComplete="email" />
              {editForm.email && !isUserEmailValid(editForm.email) && <p role="alert" className="text-xs text-destructive">请输入有效的邮箱地址。</p>}
            </FormField>
            <FormField id="edit-password" label="新密码" description="留空即保留当前密码；设置时至少输入 12 个字符。">
              <Input id="edit-password" type="password" value={editForm.password} onChange={(event) => setEditForm({ ...editForm, password: event.target.value })} placeholder="不修改密码" autoComplete="new-password" aria-describedby="edit-password-help" />
            </FormField>
            {editForm.password && (
              <div role="status" className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-xs leading-relaxed text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
                保存新密码会注销该用户的全部会话{editingUser?.id === currentUser?.id ? '，当前控制台也需要重新登录' : ''}。
              </div>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField id="edit-role" label="角色" description={roleDescription(editForm.role)}>
                <Select disabled={editingUserAccessLocked} value={editForm.role} onValueChange={(value) => setEditForm({ ...editForm, role: value as UserRole })}>
                  <SelectTrigger id="edit-role" aria-describedby="edit-role-help"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="admin">管理员</SelectItem>
                    <SelectItem value="user">普通用户</SelectItem>
                    <SelectItem value="viewer">只读用户</SelectItem>
                  </SelectContent>
                </Select>
              </FormField>
              <FormField id="edit-status" label="状态" description={statusDescription(editForm.status)}>
                <Select disabled={editingUserAccessLocked} value={editForm.status} onValueChange={(value) => setEditForm({ ...editForm, status: value as UserStatus })}>
                  <SelectTrigger id="edit-status" aria-describedby="edit-status-help"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="active">可用</SelectItem>
                    <SelectItem value="disabled">禁用</SelectItem>
                    <SelectItem value="suspended">暂停</SelectItem>
                  </SelectContent>
                </Select>
              </FormField>
            </div>
            {editingUserAccessLocked && (
              <div role="status" className="rounded-md bg-muted px-3 py-2 text-xs leading-relaxed text-muted-foreground">
                {editingUser?.id === currentUser?.id
                  ? '为避免锁定控制台，不能降低当前登录账号的角色或停用当前账号。'
                  : '系统必须保留至少一名可用管理员，因此不能修改此账号的角色或状态。'}
              </div>
            )}
            {editingUser && editingUser.status === 'active' && editForm.status !== 'active' && (
              <div role="alert" className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
                保存后该用户会立即退出登录，现有 API 密钥和配额会被回收；恢复用户状态时不会自动恢复这些资源。
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={closeEditUser}>取消</Button>
            <Button
              onClick={handleUpdateUser}
              disabled={!isUserEmailValid(editForm.email) || Boolean(editForm.password && editForm.password.length < 12) || updateUser.isPending}
            >
              {updateUser.isPending ? '保存中…' : '保存更改'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={!!showDetailUser}
        onOpenChange={(open) => {
          if (!open) {
            if (newKeyResult) {
              toast.warning('请先保存完整密钥，再返回密钥列表')
              return
            }
            setShowDetailUser(null)
            setNewKeyResult(null)
            setUserKeysPage(1)
            setKeyName('')
          }
        }}
      >
        <DialogContent className="max-h-[90vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{detailUser?.username || '用户'}的 API 密钥</DialogTitle>
            <DialogDescription>密钥创建后仅展示一次；禁用操作会立即中断后续调用。</DialogDescription>
          </DialogHeader>

          {newKeyResult ? (
            <div className="space-y-4">
              <div role="status" className="rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-100">
                <p className="font-semibold">立即保存此密钥</p>
                <p className="mt-1">完整密钥只显示这一次，关闭窗口后无法再次查看。</p>
              </div>
              <FormField id="new-user-key" label="新 API 密钥">
                <div className="flex gap-2">
                  <Input id="new-user-key" value={newKeyResult} readOnly className="font-mono text-xs" onFocus={(event) => event.currentTarget.select()} />
                  <Button variant="outline" size="icon" onClick={() => void copyText(newKeyResult, 'API 密钥')} aria-label="复制新 API 密钥">
                    <Copy className="h-4 w-4" />
                  </Button>
                </div>
              </FormField>
              <DialogFooter>
                <Button onClick={() => { setNewKeyResult(null); void refetchUserKeys() }}>已保存，返回列表</Button>
              </DialogFooter>
            </div>
          ) : (
            <div className="space-y-4">
              {createApiKey.error && <ErrorNotice message={errorMessage(createApiKey.error)} />}
              {detailUser?.status !== 'active' && (
                <div role="status" className="rounded-md border border-amber-300 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-100">
                  当前用户不可用，无法签发新密钥。先将用户状态恢复为“可用”。
                </div>
              )}
              <div className="flex flex-col gap-2 sm:flex-row">
                <Input
                  aria-label="新密钥名称"
                  placeholder="密钥名称，例如：本地开发"
                  value={keyName}
                  onChange={(event) => setKeyName(event.target.value)}
                  disabled={detailUser?.status !== 'active'}
                />
                <Button onClick={handleCreateKey} disabled={detailUser?.status !== 'active' || createApiKey.isPending}>
                  <Key className="mr-2 h-4 w-4" />
                  {createApiKey.isPending ? '生成中…' : '生成密钥'}
                </Button>
              </div>

              {userKeysError ? (
                <ErrorNotice message={`密钥列表加载失败：${errorMessage(userKeysQueryError)}`} actionLabel="重试" onAction={() => void refetchUserKeys()} />
              ) : userKeysLoading ? (
                <div className="space-y-2" aria-label="正在加载 API 密钥">
                  {Array.from({ length: 3 }).map((_, index) => <Skeleton key={index} className="h-12 w-full" />)}
                </div>
              ) : userApiKeys.length === 0 ? (
                <EmptyState icon={Key} title="暂无 API 密钥" description="为此用户生成第一把密钥。" className="py-8" />
              ) : (
                <>
                  <Table className="min-w-[620px]">
                    <TableHeader>
                      <TableRow>
                        <TableHead>名称</TableHead>
                        <TableHead>密钥标识</TableHead>
                        <TableHead>状态</TableHead>
                        <TableHead>最后使用</TableHead>
                        <TableHead className="w-12"><span className="sr-only">操作</span></TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {userKeysWindow.items.map((key) => (
                        <TableRow key={key.id}>
                          <TableCell className="font-medium">{key.name}</TableCell>
                          <TableCell className="font-mono text-xs">{key.keyPreview || key.keyPrefix}</TableCell>
                          <TableCell><StatusBadge status={key.status} /></TableCell>
                          <TableCell className="text-sm text-muted-foreground">{key.lastUsedAt ? formatDate(key.lastUsedAt) : '从未使用'}</TableCell>
                          <TableCell>
                            {key.status === 'active' && (
                              <Button variant="ghost" size="icon" className="h-8 w-8 text-destructive" onClick={() => requestRevokeKey(key.id, key.name)} aria-label={`禁用密钥 ${key.name}`}>
                                <ShieldOff className="h-4 w-4" />
                              </Button>
                            )}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                  <PaginationBar
                    total={userApiKeys.length}
                    page={userKeysWindow.currentPage}
                    pageSize={userKeysPageSize}
                    totalPages={userKeysWindow.totalPages}
                    start={userKeysWindow.start}
                    end={userKeysWindow.end}
                    totalLabel="个 API 密钥"
                    pageSizeOptions={[5, 10, 20, 50]}
                    className="border-t pt-3"
                    onPageChange={(page) => setUserKeysPage(Math.min(Math.max(page, 1), userKeysWindow.totalPages))}
                    onPageSizeChange={(pageSize) => { setUserKeysPageSize(pageSize); setUserKeysPage(1) }}
                  />
                </>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={!!confirmAction}
        title={confirmAction?.title || ''}
        description={confirmAction?.description || ''}
        confirmLabel={confirmAction?.confirmLabel}
        destructive={confirmAction?.destructive}
        pending={deleteUser.isPending || revokeApiKey.isPending || updateUser.isPending}
        onCancel={() => setConfirmAction(null)}
        onConfirm={() => confirmAction?.onConfirm()}
      />
    </div>
  )
}

function SummaryCard({ label, value, hint, tone = 'default', className }: { label: string; value: number; hint: string; tone?: 'default' | 'warning'; className?: string }) {
  return (
    <Card className={cn(tone === 'warning' && 'border-amber-300 dark:border-amber-900', className)}>
      <CardContent className="p-4">
        <p className="text-sm text-muted-foreground">{label}</p>
        <p className="mt-1 text-2xl font-semibold tabular-nums">{formatNumber(value)}</p>
        <p className="mt-1 text-xs text-muted-foreground">{hint}</p>
      </CardContent>
    </Card>
  )
}

function UserActionMenu({ user, canDelete, onEdit, onKeys, onDelete }: {
  user: User
  canDelete: boolean
  onEdit: (user: User) => void
  onKeys: (userId: string) => void
  onDelete: (user: User) => void
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" className="h-8 w-8" aria-label={`打开 ${user.username} 的操作菜单`}>
          <MoreHorizontal className="h-4 w-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => onEdit(user)}><Pencil className="mr-2 h-4 w-4" />编辑用户</DropdownMenuItem>
        <DropdownMenuItem onClick={() => onKeys(user.id)}><Key className="mr-2 h-4 w-4" />管理 API 密钥</DropdownMenuItem>
        <DropdownMenuItem disabled={!canDelete} className="text-destructive" onClick={() => onDelete(user)}>
          <Trash2 className="mr-2 h-4 w-4" />{canDelete ? '删除用户' : '无法删除当前/最后管理员'}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function FormField({ id, label, required, description, children }: {
  id: string
  label: string
  required?: boolean
  description?: string
  children: React.ReactNode
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}{required && <span className="ml-1 text-destructive" aria-hidden="true">*</span>}</Label>
      {children}
      {description && <p id={`${id}-help`} className="text-xs leading-relaxed text-muted-foreground">{description}</p>}
    </div>
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

function roleBadgeVariant(role: string): 'default' | 'secondary' | 'outline' {
  if (role === 'admin') return 'default'
  if (role === 'viewer') return 'secondary'
  return 'outline'
}

function roleDescription(role: UserRole): string {
  if (role === 'admin') return '可管理系统配置、用户、模型、配额和所有密钥。'
  if (role === 'viewer') return '仅查看被授权的数据，不能创建或修改资源。'
  return '可查看、重命名、禁用或删除自己的现有 API 密钥，不能访问管理页面。'
}

function statusDescription(status: UserStatus): string {
  if (status === 'active') return '允许登录和使用所属 API 密钥。'
  if (status === 'suspended') return '停止登录并回收现有密钥和配额；恢复后需重新配置。'
  return '停止登录并回收现有密钥和配额，适合长期停用。'
}

function errorMessage(error: unknown): string {
  if (!error) return ''
  return error instanceof Error ? error.message : String(error)
}
