import { useMemo, useState } from 'react'
import { useApiKeys, useCreateApiKey, useDeleteApiKey, useRevokeApiKey, useUsers } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { TableToolbar } from '@/components/shared/TableToolbar'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Card, CardContent } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { formatDate, formatNumber } from '@/lib/utils'
import { Copy, KeyRound, Plus, RotateCw, Search, ShieldOff, Trash2 } from 'lucide-react'

export function ApiKeysPage() {
  const { data: apiKeys = [], isLoading, refetch } = useApiKeys()
  const { data: users = [] } = useUsers()
  const createApiKey = useCreateApiKey()
  const revokeApiKey = useRevokeApiKey()
  const deleteApiKey = useDeleteApiKey()

  const [search, setSearch] = useState('')
  const [status, setStatus] = useState('__all__')
  const [group, setGroup] = useState('__all__')
  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [newKey, setNewKey] = useState<string | null>(null)
  const [form, setForm] = useState({
    userId: '',
    name: '',
    group: '',
  })

  const groups = useMemo(() => {
    return Array.from(new Set(apiKeys.map((key) => key.group).filter(Boolean))).sort()
  }, [apiKeys])

  const filteredKeys = apiKeys.filter((key) => {
    const haystack = `${key.name} ${key.username || ''} ${key.keyPreview || key.keyPrefix} ${key.group || ''}`.toLowerCase()
    if (search && !haystack.includes(search.toLowerCase())) return false
    if (status !== '__all__' && key.status !== status) return false
    if (group !== '__all__' && key.group !== group) return false
    return true
  })

  const activeKeys = apiKeys.filter((key) => key.status === 'active').length

  const handleCreate = () => {
    const user = users.find((item) => item.id === form.userId)
    if (!user || !form.name.trim()) return
    createApiKey.mutate({
      userId: user.id,
      username: user.username,
      name: form.name.trim(),
      group: form.group.trim() || undefined,
    }, {
      onSuccess: (key) => {
        setNewKey(key.key || null)
        setForm({ userId: '', name: '', group: '' })
      },
    })
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="API Keys"
        description={`${formatNumber(activeKeys)} active / ${formatNumber(apiKeys.length)} total`}
        action={{ label: 'Create API Key', onClick: () => { setNewKey(null); setShowCreateDialog(true) }, icon: Plus }}
      />

      <TableToolbar
        actions={(
          <Button variant="outline" size="icon" onClick={() => refetch()}>
            <RotateCw className="h-4 w-4" />
          </Button>
        )}
      >
        <div className="relative min-w-[240px] flex-1">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            className="pl-8"
            placeholder="Search name, user, or key..."
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>
        <Select value={group} onValueChange={setGroup}>
          <SelectTrigger className="w-[180px]"><SelectValue placeholder="All Groups" /></SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">All Groups</SelectItem>
            {groups.map((item) => (
              <SelectItem key={item} value={item || ''}>{item}</SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={status} onValueChange={setStatus}>
          <SelectTrigger className="w-[160px]"><SelectValue placeholder="All Status" /></SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">All Status</SelectItem>
            <SelectItem value="active">Active</SelectItem>
            <SelectItem value="revoked">Revoked</SelectItem>
          </SelectContent>
        </Select>
      </TableToolbar>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>API Key</TableHead>
                <TableHead>User</TableHead>
                <TableHead>Group</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="text-right">Today</TableHead>
                <TableHead>Last Used</TableHead>
                <TableHead className="w-28 text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={8} className="h-24 text-center text-muted-foreground">Loading...</TableCell>
                </TableRow>
              ) : filteredKeys.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={8} className="h-24 text-center text-muted-foreground">No API keys</TableCell>
                </TableRow>
              ) : filteredKeys.map((key) => (
                <TableRow key={key.id}>
                  <TableCell className="font-medium">{key.name}</TableCell>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <code className="rounded bg-muted px-2 py-1 text-xs text-primary">{key.keyPreview || key.keyPrefix}</code>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={() => navigator.clipboard.writeText(key.keyPreview || key.keyPrefix)}
                      >
                        <Copy className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </TableCell>
                  <TableCell>{key.username || key.userId}</TableCell>
                  <TableCell>
                    {key.group ? <Badge variant="outline">{key.group}</Badge> : <span className="text-muted-foreground">No group</span>}
                  </TableCell>
                  <TableCell><StatusBadge status={key.status} /></TableCell>
                  <TableCell className="text-right text-sm">
                    {formatNumber(key.requestsToday ?? 0)} req · {formatNumber(key.tokensToday ?? 0)} tok
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {key.lastUsedAt ? formatDate(key.lastUsedAt) : 'Never'}
                  </TableCell>
                  <TableCell>
                    <div className="flex justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8"
                        disabled={key.status !== 'active'}
                        onClick={() => revokeApiKey.mutate(key.id)}
                      >
                        <ShieldOff className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-8 w-8 text-destructive"
                        onClick={() => deleteApiKey.mutate(key.id)}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <Dialog open={showCreateDialog} onOpenChange={(open) => { setShowCreateDialog(open); if (!open) setNewKey(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create API Key</DialogTitle>
            <DialogDescription>Issue a ModelPort key for a user. The full key is shown once.</DialogDescription>
          </DialogHeader>

          {newKey ? (
            <div className="space-y-2">
              <Label>New API Key</Label>
              <div className="flex gap-2">
                <Input value={newKey} readOnly className="font-mono text-xs" />
                <Button variant="outline" size="icon" onClick={() => navigator.clipboard.writeText(newKey)}>
                  <Copy className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ) : (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>User</Label>
                <Select value={form.userId} onValueChange={(value) => setForm({ ...form, userId: value })}>
                  <SelectTrigger><SelectValue placeholder="Select user" /></SelectTrigger>
                  <SelectContent>
                    {users.map((user) => (
                      <SelectItem key={user.id} value={user.id}>{user.username}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label>Name</Label>
                <Input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} placeholder="Claude Code" />
              </div>
              <div className="space-y-2">
                <Label>Group</Label>
                <Input value={form.group} onChange={(event) => setForm({ ...form, group: event.target.value })} placeholder="150 quota pro" />
              </div>
            </div>
          )}

          <DialogFooter>
            {newKey ? (
              <Button onClick={() => setShowCreateDialog(false)}>Done</Button>
            ) : (
              <>
                <Button variant="outline" onClick={() => setShowCreateDialog(false)}>Cancel</Button>
                <Button onClick={handleCreate} disabled={!form.userId || !form.name.trim() || createApiKey.isPending}>
                  <KeyRound className="mr-2 h-4 w-4" />
                  Create
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
