import { useState } from 'react'
import { useUsers, useCreateUser, useDeleteUser, useUserApiKeys, useCreateApiKey, useRevokeApiKey } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { StatusBadge } from '@/components/shared/StatusBadge'
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
import { UserPlus, Key, Trash2, Copy, MoreHorizontal } from 'lucide-react'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import type { UserRole } from '@/types'

export function UsersPage() {
  const { data: users = [], isLoading } = useUsers()
  const createUser = useCreateUser()
  const deleteUser = useDeleteUser()
  const createApiKey = useCreateApiKey()
  const revokeApiKey = useRevokeApiKey()

  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [showDetailUser, setShowDetailUser] = useState<string | null>(null)
  const [newKeyResult, setNewKeyResult] = useState<string | null>(null)

  const [form, setForm] = useState({ username: '', email: '', role: 'user' as UserRole, status: 'active' as 'active' | 'disabled' | 'suspended' })
  const [keyName, setKeyName] = useState('')

  const { data: userApiKeys = [] } = useUserApiKeys(showDetailUser || '')

  const handleCreateUser = () => {
    createUser.mutate(form, {
      onSuccess: () => {
        setShowCreateDialog(false)
        setForm({ username: '', email: '', role: 'user', status: 'active' })
      },
    })
  }

  const handleCreateKey = () => {
    if (!showDetailUser) return
    createApiKey.mutate({ userId: showDetailUser, name: keyName || '默认密钥' }, {
      onSuccess: (key) => {
        setNewKeyResult(`${key.keyPrefix}${Math.random().toString(36).slice(2, 18)}`)
        setKeyName('')
      },
    })
  }

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
                        <DropdownMenuItem onClick={() => setShowDetailUser(user.id)}>
                          <Key className="mr-2 h-4 w-4" />
                          管理 API Keys
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          className="text-destructive"
                          onClick={() => deleteUser.mutate(user.id)}
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
            <div className="space-y-2">
              <Label>用户名</Label>
              <Input value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} placeholder="输入用户名" />
            </div>
            <div className="space-y-2">
              <Label>邮箱</Label>
              <Input value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} placeholder="输入邮箱" />
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
                      <TableCell className="font-mono text-xs">{key.keyPrefix}</TableCell>
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
                            onClick={() => revokeApiKey.mutate(key.id)}
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
    </div>
  )
}
