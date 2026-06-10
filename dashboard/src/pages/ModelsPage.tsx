import { useState } from 'react'
import { useProviders, useAliases, useCreateAlias, useDeleteAlias, useUpdateDefaultProvider } from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { PROVIDER_PROTOCOL_LABELS } from '@/lib/constants'
import { Plus, Trash2, ChevronDown, ChevronRight, Settings, ArrowUpDown } from 'lucide-react'

export function ModelsPage() {
  const { data: providers = [], isLoading } = useProviders()
  const { data: aliases = [] } = useAliases()
  const createAlias = useCreateAlias()
  const deleteAlias = useDeleteAlias()
  const updateDefault = useUpdateDefaultProvider()

  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)
  const [showAliasDialog, setShowAliasDialog] = useState(false)
  const [aliasForm, setAliasForm] = useState({ alias: '', target: '' })
  const [defaultProvider, setDefaultProvider] = useState('mimo')

  const activeProviders = providers.filter((p) => p.status === 'active')

  if (isLoading) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  return (
    <div className="space-y-6">
      <PageHeader title="模型管理" description="管理提供商、模型别名和路由配置" />

      <Tabs defaultValue="providers">
        <TabsList>
          <TabsTrigger value="providers">提供商</TabsTrigger>
          <TabsTrigger value="aliases">别名</TabsTrigger>
          <TabsTrigger value="switcher">模型切换</TabsTrigger>
        </TabsList>

        {/* Providers Tab */}
        <TabsContent value="providers" className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {providers.map((provider) => (
              <Card key={provider.id} className="overflow-hidden">
                <CardHeader className="pb-3">
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">{provider.displayName}</CardTitle>
                    <StatusBadge status={provider.status} />
                  </div>
                  <div className="flex items-center gap-2">
                    <Badge variant="outline">{PROVIDER_PROTOCOL_LABELS[provider.protocol]}</Badge>
                    <span className="text-xs text-muted-foreground">{provider.models.length} 模型</span>
                  </div>
                </CardHeader>
                <CardContent className="pt-0">
                  <p className="text-xs text-muted-foreground mb-2 truncate">{provider.baseUrl}</p>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="w-full justify-between"
                    onClick={() => setExpandedProvider(expandedProvider === provider.id ? null : provider.id)}
                  >
                    <span>模型列表</span>
                    {expandedProvider === provider.id ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                  </Button>
                  {expandedProvider === provider.id && (
                    <div className="mt-2 space-y-1">
                      {provider.models.map((model) => (
                        <div key={model} className="flex items-center justify-between rounded-md border px-3 py-1.5">
                          <span className="text-sm font-mono">{model}</span>
                          <Switch defaultChecked={provider.status === 'active'} />
                        </div>
                      ))}
                    </div>
                  )}
                </CardContent>
              </Card>
            ))}
          </div>
        </TabsContent>

        {/* Aliases Tab */}
        <TabsContent value="aliases" className="space-y-4">
          <div className="flex justify-end">
            <Button onClick={() => setShowAliasDialog(true)}>
              <Plus className="mr-2 h-4 w-4" />
              新建别名
            </Button>
          </div>

          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>别名</TableHead>
                    <TableHead>目标</TableHead>
                    <TableHead>解析提供商</TableHead>
                    <TableHead>解析模型</TableHead>
                    <TableHead className="w-12"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {aliases.map((alias) => (
                    <TableRow key={alias.alias}>
                      <TableCell className="font-mono font-medium">{alias.alias}</TableCell>
                      <TableCell className="text-muted-foreground">{alias.target}</TableCell>
                      <TableCell>{alias.resolvedProvider}</TableCell>
                      <TableCell className="font-mono text-sm">{alias.resolvedModel}</TableCell>
                      <TableCell>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 text-destructive"
                          onClick={() => deleteAlias.mutate(alias.alias)}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Model Switcher Tab */}
        <TabsContent value="switcher" className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Settings className="h-4 w-4" />
                默认提供商
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <p className="text-sm text-muted-foreground">
                当未知模型请求到达时，将使用默认提供商进行路由。提供商优先级按配置顺序排列。
              </p>
              <div className="space-y-2">
                <Label>默认提供商</Label>
                <Select value={defaultProvider} onValueChange={(v) => { setDefaultProvider(v); updateDefault.mutate(v) }}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {activeProviders.map((p) => (
                      <SelectItem key={p.id} value={p.id}>{p.displayName}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label>提供商优先级（按配置顺序）</Label>
                <div className="space-y-1">
                  {providers.map((provider, index) => (
                    <div key={provider.id} className="flex items-center gap-3 rounded-md border px-3 py-2">
                      <span className="text-sm text-muted-foreground w-6">{index + 1}</span>
                      <span className="flex-1 text-sm font-medium">{provider.displayName}</span>
                      <StatusBadge status={provider.status} />
                      <ArrowUpDown className="h-4 w-4 text-muted-foreground" />
                    </div>
                  ))}
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      {/* Create Alias Dialog */}
      <Dialog open={showAliasDialog} onOpenChange={setShowAliasDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建别名</DialogTitle>
            <DialogDescription>创建模型别名以简化路由配置</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label>别名</Label>
              <Input value={aliasForm.alias} onChange={(e) => setAliasForm({ ...aliasForm, alias: e.target.value })} placeholder="例如: sonnet" />
            </div>
            <div className="space-y-2">
              <Label>目标</Label>
              <Input value={aliasForm.target} onChange={(e) => setAliasForm({ ...aliasForm, target: e.target.value })} placeholder="例如: openrouter:anthropic/claude-sonnet-4" />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAliasDialog(false)}>取消</Button>
            <Button onClick={() => {
              createAlias.mutate(aliasForm, { onSuccess: () => { setShowAliasDialog(false); setAliasForm({ alias: '', target: '' }) } })
            }} disabled={createAlias.isPending}>创建</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
