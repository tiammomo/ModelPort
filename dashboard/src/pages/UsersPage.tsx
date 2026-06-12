import { useState } from 'react'
import { useUsers, useCreateUser, useUpdateUser, useDeleteUser, useUserApiKeys, useCreateApiKey, useRevokeApiKey } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { ConfirmDialog } from '@/components/shared/ConfirmDialog'
import { Card, CardContent } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { formatDate, formatNumber } from '@/lib/utils'
import { ROLE_LABELS } from '@/lib/constants'
import { UserPlus, Key, Trash2, Copy, MoreHorizontal, Pencil } from 'lucide-react'
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
  const { data: users = [], isLoading } = useUsers()
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

  const [form, setForm] = useState({
    username: '',
    email: '',
    password: '',
    role: 'user' as UserRole,
    status: 'active' as 'active' | 'disabled' | 'suspended',
  })
  const [editForm, setEditForm] = useState(emptyEditForm)
  const [keyName, setKeyName] = useState('')

  const { data: userApiKeys = [] } = useUserApiKeys(showDetailUser || '')

  const handleCreateUser = () => {
    createUser.mutate(form, {
      onSuccess: () => {
        setShowCreateDialog(false)
        setForm({ username: '', email: '', password: '', role: 'user', status: 'active' })
      },
    })
  }

  const openEditUser = (user: User) => {
    setEditingUser(user)
    setEditForm({
      email: user.email,
      password: '',
      role: user.role,
      status: user.status,
    })
  }

  const closeEditUser = () => {
    setEditingUser(null)
    setEditForm(emptyEditForm)
  }

  const handleUpdateUser = () => {
    if (!editingUser || !editForm.email.trim()) return
    updateUser.mutate({
      id: editingUser.id,
      data: {
        email: editForm.email.trim(),
        role: editForm.role,
        status: editForm.status,
        ...(editForm.password ? { password: editForm.password } : {}),
      },
    }, {
      onSuccess: closeEditUser,
    })
  }

  const handleCreateKey = () => {
    if (!showDetailUser) return
    const user = users.find((item) => item.id === showDetailUser)
    createApiKey.mutate({ userId: showDetailUser, username: user?.username, name: keyName || '默认密钥' }, {
      onSuccess: (key) => {
        setNewKeyResult(key.key || key.keyPreview || key.keyPrefix)
        setKeyName('')
      },
    })
  }

  const mutationError = errorMessage(deleteUser.error || revokeApiKey.error)

  const roleBadgeVariant = (role: string) => {
    if (role === 'admin') return 'default'
    if (role === 'viewer') return 'secondary'
    return 'outline'
  }

  if (isLoading) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="用户管理"
        description="管理系统用户和 API 密钥"
        action={{ label: '新建用户', onClick: () => setShowCreateDialog(true), icon: UserPlus }}
      />

      {mutationError && <ErrorNotice message={mutationError} />}

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>用户名</TableHead>
                <TableHead>邮箱</TableHead>
                <TableHead>角色</TableHead>
                <TableHead>状态</TableHead>
                <TableHead className="text-center">API Keys</TableHead>
                <TableHead className="text-right">24h 请求</TableHead>
                <TableHead>最后登录</TableHead>
                <TableHead className="w-12"></TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {users.map((user) => (
                <TableRow key={user.id}>
                  <TableCell className="font-medium">{user.username}</TableCell>
                  <TableCell className="text-muted-foreground">{user.email}</TableCell>
                  <TableCell>
                    <Badge variant={roleBadgeVariant(user.role)}>{ROLE_LABELS[user.role]}</Badge>
                  </TableCell>
                  <TableCell><StatusBadge status={user.status} /></TableCell>
                  <TableCell className="text-center">{user.apiKeyCount}</TableCell>
                  <TableCell className="text-right">{formatNumber(user.requestCount24h)}</TableCell>
                  <TableCell className="text-muted-foreground text-sm">
                    {user.lastLoginAt ? formatDate(user.lastLoginAt) : '从未登录'}
                  </TableCell>
                  <TableCell>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-8 w-8">
                          <MoreHorizontal className="h-4 w-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onClick={() => openEditUser(user)}>
                          <Pencil className="mr-2 h-4 w-4" />
                          编辑用户
                        </DropdownMenuItem>
                        <DropdownMenuItem onClick={() => setShowDetailUser(user.id)}>
                          <Key className="mr-2 h-4 w-4" />
                          管理 API Keys
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          className="text-destructive"
                          onClick={() => setConfirmAction({
                            title: '删除用户',
                            description: `删除 ${user.username} 后，其相关 API 密钥会被回收，操作不可撤销。`,
                            confirmLabel: '删除',
                            destructive: true,
                            onConfirm: () => deleteUser.mutate(user.id, { onSettled: () => setConfirmAction(null) }),
                          })}
                        >
                          <Trash2 className="mr-2 h-4 w-4" />
                          删除用户
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Create User Dialog */}
      <Dialog open={showCreateDialog} onOpenChange={setShowCreateDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建用户</DialogTitle>
            <DialogDescription>创建一个新的系统用户</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {createUser.error && <ErrorNotice message={errorMessage(createUser.error)} />}
            <div className="space-y-2">
              <Label>用户名</Label>
              <Input value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} placeholder="输入用户名" />
            </div>
            <div className="space-y-2">
              <Label>邮箱</Label>
              <Input value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} placeholder="输入邮箱" />
            </div>
            <div className="space-y-2">
              <Label>初始密码</Label>
              <Input
                type="password"
                value={form.password}
                onChange={(e) => setForm({ ...form, password: e.target.value })}
                placeholder="至少 12 个字符"
                autoComplete="new-password"
              />
            </div>
            <div className="space-y-2">
              <Label>角色</Label>
              <Select value={form.role} onValueChange={(v) => setForm({ ...form, role: v as UserRole })}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="admin">管理员</SelectItem>
                  <SelectItem value="user">普通用户</SelectItem>
                  <SelectItem value="viewer">只读用户</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowCreateDialog(false)}>取消</Button>
            <Button onClick={handleCreateUser} disabled={createUser.isPending}>创建</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit User Dialog */}
      <Dialog open={!!editingUser} onOpenChange={(open) => { if (!open) closeEditUser() }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>编辑用户</DialogTitle>
            <DialogDescription>调整用户权限、邮箱和登录密码</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {updateUser.error && <ErrorNotice message={errorMessage(updateUser.error)} />}
            <div className="space-y-2">
              <Label>用户名</Label>
              <Input value={editingUser?.username || ''} disabled />
            </div>
            <div className="space-y-2">
              <Label>邮箱</Label>
              <Input
                value={editForm.email}
                onChange={(e) => setEditForm({ ...editForm, email: e.target.value })}
                placeholder="输入邮箱"
              />
            </div>
            <div className="space-y-2">
              <Label>新密码</Label>
              <Input
                type="password"
                value={editForm.password}
                onChange={(e) => setEditForm({ ...editForm, password: e.target.value })}
                placeholder="留空不修改"
                autoComplete="new-password"
              />
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label>角色</Label>
                <Select value={editForm.role} onValueChange={(value) => setEditForm({ ...editForm, role: value as UserRole })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="admin">管理员</SelectItem>
                    <SelectItem value="user">普通用户</SelectItem>
                    <SelectItem value="viewer">只读用户</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label>状态</Label>
                <Select value={editForm.status} onValueChange={(value) => setEditForm({ ...editForm, status: value as UserStatus })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="active">活跃</SelectItem>
                    <SelectItem value="disabled">禁用</SelectItem>
                    <SelectItem value="suspended">已暂停</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={closeEditUser}>取消</Button>
            <Button onClick={handleUpdateUser} disabled={!editForm.email.trim() || updateUser.isPending}>
              保存
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* User Detail / API Keys Dialog */}
      <Dialog open={!!showDetailUser} onOpenChange={() => { setShowDetailUser(null); setNewKeyResult(null) }}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>API 密钥管理</DialogTitle>
            <DialogDescription>
              管理用户 {users.find((u) => u.id === showDetailUser)?.username} 的 API 密钥
            </DialogDescription>
          </DialogHeader>

          {newKeyResult ? (
            <div className="space-y-2">
              <Label>新密钥已生成（仅显示一次）</Label>
              <div className="flex gap-2">
                <Input value={newKeyResult} readOnly className="font-mono text-xs" />
                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => navigator.clipboard.writeText(newKeyResult)}
                >
                  <Copy className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              {createApiKey.error && <ErrorNotice message={errorMessage(createApiKey.error)} />}
              <div className="flex gap-2">
                <Input
                  placeholder="密钥名称"
                  value={keyName}
                  onChange={(e) => setKeyName(e.target.value)}
                />
                <Button onClick={handleCreateKey} disabled={createApiKey.isPending}>
                  <Key className="mr-2 h-4 w-4" />
                  生成
                </Button>
              </div>

              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>名称</TableHead>
                    <TableHead>密钥前缀</TableHead>
                    <TableHead>状态</TableHead>
                    <TableHead>最后使用</TableHead>
                    <TableHead className="w-12"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {userApiKeys.map((key) => (
                    <TableRow key={key.id}>
                      <TableCell className="font-medium">{key.name}</TableCell>
                      <TableCell className="font-mono text-xs">{key.keyPreview || key.keyPrefix}</TableCell>
                      <TableCell><StatusBadge status={key.status} /></TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {key.lastUsedAt ? formatDate(key.lastUsedAt) : '从未使用'}
                      </TableCell>
                      <TableCell>
                        {key.status === 'active' && (
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 text-destructive"
                            onClick={() => setConfirmAction({
                              title: '禁用 API 密钥',
                              description: `禁用 ${key.name} 后，它将不能继续调用 API。`,
                              confirmLabel: '禁用',
                              destructive: true,
                              onConfirm: () => revokeApiKey.mutate(key.id, { onSettled: () => setConfirmAction(null) }),
                            })}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
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
        pending={deleteUser.isPending || revokeApiKey.isPending}
        onCancel={() => setConfirmAction(null)}
        onConfirm={() => confirmAction?.onConfirm()}
      />
    </div>
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
