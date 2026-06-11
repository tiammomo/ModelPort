import { Fragment, useMemo, useState } from 'react'
import {
  useProviders,
  useAliases,
  useCreateAlias,
  useDeleteAlias,
  useDiscoverProviderModels,
  useUpdateDefaultProvider,
} from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { TableToolbar } from '@/components/shared/TableToolbar'
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
import { PROVIDER_PROTOCOL_LABELS } from '@/lib/constants'
import { formatNumber } from '@/lib/utils'
import {
  MODEL_FAMILIES,
  PROVIDER_TEMPLATES,
  guessModelFamily,
  providerEnv,
  providerToml,
  type ProviderTemplate,
} from '@/lib/model-catalog'
import {
  ArrowUpDown,
  ChevronDown,
  ChevronRight,
  Copy,
  FileText,
  KeyRound,
  Layers3,
  Plus,
  Route,
  Search,
  Settings,
  Trash2,
} from 'lucide-react'
import type { Provider } from '@/types'

interface ModelChannel {
  provider: Provider
  routeName: string
  priority: number
}

interface ModelRow {
  model: string
  family: string
  channels: ModelChannel[]
  activeChannels: number
  defaultChannel: ModelChannel
}

const ALL = '__all__'

export function ModelsPage() {
  const { data: providers = [], isLoading } = useProviders()
  const { data: aliases = [] } = useAliases()
  const createAlias = useCreateAlias()
  const deleteAlias = useDeleteAlias()
  const discoverModels = useDiscoverProviderModels()
  const updateDefault = useUpdateDefaultProvider()

  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)
  const [expandedModel, setExpandedModel] = useState<string | null>(null)
  const [discoveringProvider, setDiscoveringProvider] = useState<string | null>(null)
  const [showAliasDialog, setShowAliasDialog] = useState(false)
  const [selectedTemplate, setSelectedTemplate] = useState<ProviderTemplate | null>(null)
  const [aliasForm, setAliasForm] = useState({ alias: '', target: '' })
  const [defaultProvider, setDefaultProvider] = useState(providers[0]?.id || 'mimo')
  const [search, setSearch] = useState('')
  const [family, setFamily] = useState(ALL)

  const configuredProviderIds = useMemo(() => new Set(providers.map((provider) => provider.id)), [providers])
  const activeProviders = providers.filter((provider) => provider.status === 'active')
  const totalConfiguredModels = providers.reduce((sum, provider) => sum + provider.models.length, 0)

  const modelRows = useMemo<ModelRow[]>(() => {
    const rows = new Map<string, ModelChannel[]>()

    providers.forEach((provider, priority) => {
      provider.models.forEach((model) => {
        const channels = rows.get(model) || []
        channels.push({
          provider,
          routeName: `${provider.id}:${model}`,
          priority,
        })
        rows.set(model, channels)
      })
    })

    return Array.from(rows.entries())
      .map(([model, channels]) => {
        const sortedChannels = [...channels].sort((a, b) => a.priority - b.priority)
        return {
          model,
          family: guessModelFamily(model),
          channels: sortedChannels,
          activeChannels: sortedChannels.filter((channel) => channel.provider.status === 'active').length,
          defaultChannel: sortedChannels[0],
        }
      })
      .sort((a, b) => a.family.localeCompare(b.family) || a.model.localeCompare(b.model))
  }, [providers])

  const filteredModelRows = modelRows.filter((row) => {
    const haystack = [
      row.model,
      row.family,
      row.channels.map((channel) => channel.provider.displayName).join(' '),
      row.channels.map((channel) => channel.provider.id).join(' '),
    ].join(' ').toLowerCase()

    if (search && !haystack.includes(search.toLowerCase())) return false
    if (family !== ALL && row.family !== family) return false
    return true
  })

  const templateRows = PROVIDER_TEMPLATES.map((template) => ({
    ...template,
    configured: configuredProviderIds.has(template.id),
  }))

  const copyText = async (text: string) => {
    await navigator.clipboard.writeText(text)
  }

  const openAliasDialog = (alias = '', target = '') => {
    setAliasForm({ alias, target })
    setShowAliasDialog(true)
  }

  const handleDiscoverModels = (providerId: string) => {
    setDiscoveringProvider(providerId)
    discoverModels.mutate(providerId, {
      onSettled: () => setDiscoveringProvider(null),
    })
  }

  if (isLoading) {
    return <div className="flex items-center justify-center h-64 text-muted-foreground">加载中...</div>
  }

  return (
    <div className="space-y-6">
      <PageHeader title="模型管理" description="按模型查看所有渠道，生成供应商配置和路由别名" />

      <div className="grid gap-4 md:grid-cols-3">
        <Card>
          <CardContent className="flex items-center gap-3 p-4">
            <div className="flex h-10 w-10 items-center justify-center rounded-md bg-primary/10 text-primary">
              <Layers3 className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm text-muted-foreground">已配置模型</p>
              <p className="text-2xl font-semibold">{formatNumber(modelRows.length)}</p>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="flex items-center gap-3 p-4">
            <div className="flex h-10 w-10 items-center justify-center rounded-md bg-green-500/10 text-green-600">
              <KeyRound className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm text-muted-foreground">活跃供应商</p>
              <p className="text-2xl font-semibold">{activeProviders.length} / {providers.length}</p>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="flex items-center gap-3 p-4">
            <div className="flex h-10 w-10 items-center justify-center rounded-md bg-blue-500/10 text-blue-600">
              <Route className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm text-muted-foreground">渠道映射</p>
              <p className="text-2xl font-semibold">{formatNumber(totalConfiguredModels)}</p>
            </div>
          </CardContent>
        </Card>
      </div>

      <Tabs defaultValue="library">
        <TabsList>
          <TabsTrigger value="library">模型库</TabsTrigger>
          <TabsTrigger value="templates">一键配置</TabsTrigger>
          <TabsTrigger value="providers">供应商</TabsTrigger>
          <TabsTrigger value="aliases">别名</TabsTrigger>
          <TabsTrigger value="routing">路由优先级</TabsTrigger>
        </TabsList>

        <TabsContent value="library" className="space-y-4">
          <TableToolbar>
            <div className="relative min-w-[240px] flex-1">
              <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                className="pl-8"
                placeholder="搜索模型、供应商或渠道..."
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </div>
            <Select value={family} onValueChange={setFamily}>
              <SelectTrigger className="w-[180px]"><SelectValue placeholder="全部模型系列" /></SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>全部模型系列</SelectItem>
                {MODEL_FAMILIES.map((item) => (
                  <SelectItem key={item} value={item}>{item}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </TableToolbar>

          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>模型</TableHead>
                    <TableHead>系列</TableHead>
                    <TableHead>默认渠道</TableHead>
                    <TableHead className="text-center">供应商</TableHead>
                    <TableHead className="text-right">路由</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filteredModelRows.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={5} className="h-24 text-center text-muted-foreground">没有匹配的模型</TableCell>
                    </TableRow>
                  ) : filteredModelRows.map((row) => (
                    <Fragment key={row.model}>
                      <TableRow>
                        <TableCell>
                          <div className="flex items-center gap-2">
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-7 w-7"
                              onClick={() => setExpandedModel(expandedModel === row.model ? null : row.model)}
                            >
                              {expandedModel === row.model ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                            </Button>
                            <span className="font-mono text-sm font-medium">{row.model}</span>
                          </div>
                        </TableCell>
                        <TableCell><Badge variant="outline">{row.family}</Badge></TableCell>
                        <TableCell>
                          <div className="space-y-1">
                            <p className="text-sm font-medium">{row.defaultChannel.provider.displayName}</p>
                            <p className="text-xs text-muted-foreground">{row.defaultChannel.provider.id}</p>
                          </div>
                        </TableCell>
                        <TableCell className="text-center">
                          <Badge variant={row.activeChannels > 0 ? 'success' : 'secondary'}>
                            {row.activeChannels} / {row.channels.length} 活跃
                          </Badge>
                        </TableCell>
                        <TableCell className="text-right">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => void copyText(row.defaultChannel.routeName)}
                          >
                            <Copy className="mr-2 h-4 w-4" />
                            {row.defaultChannel.routeName}
                          </Button>
                        </TableCell>
                      </TableRow>
                      {expandedModel === row.model && (
                        <TableRow key={`${row.model}-channels`}>
                          <TableCell colSpan={5} className="bg-muted/30 p-4">
                            <div className="grid gap-3 md:grid-cols-2">
                              {row.channels.map((channel) => (
                                <div key={channel.routeName} className="rounded-md border bg-background p-3">
                                  <div className="flex items-start justify-between gap-3">
                                    <div className="min-w-0">
                                      <p className="font-medium">{channel.provider.displayName}</p>
                                      <p className="truncate text-xs text-muted-foreground">{channel.provider.baseUrl}</p>
                                    </div>
                                    <StatusBadge status={channel.provider.status} />
                                  </div>
                                  <div className="mt-3 flex flex-wrap items-center gap-2">
                                    <Badge variant="outline">{PROVIDER_PROTOCOL_LABELS[channel.provider.protocol]}</Badge>
                                    <code className="rounded bg-muted px-2 py-1 text-xs">{channel.routeName}</code>
                                  </div>
                                  <div className="mt-3 flex flex-wrap gap-2">
                                    <Button variant="outline" size="sm" onClick={() => void copyText(channel.routeName)}>
                                      <Copy className="mr-2 h-4 w-4" />
                                      复制路由名
                                    </Button>
                                    <Button variant="ghost" size="sm" onClick={() => openAliasDialog(row.model, channel.routeName)}>
                                      <Plus className="mr-2 h-4 w-4" />
                                      设为别名
                                    </Button>
                                  </div>
                                </div>
                              ))}
                            </div>
                          </TableCell>
                        </TableRow>
                      )}
                    </Fragment>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="templates" className="space-y-4">
          <TableToolbar>
            <div className="text-sm text-muted-foreground">
              选择模板后复制 TOML 或 env 配置，重启后即可出现在模型库里。
            </div>
          </TableToolbar>
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {templateRows.map((template) => (
              <Card key={template.id} className="overflow-hidden">
                <CardHeader className="pb-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <CardTitle className="truncate text-base">{template.displayName}</CardTitle>
                      <div className="mt-2 flex flex-wrap gap-2">
                        <Badge variant="outline">{template.family}</Badge>
                        <Badge variant="outline">{PROVIDER_PROTOCOL_LABELS[template.protocol]}</Badge>
                        {template.configured && <Badge variant="success">已配置</Badge>}
                      </div>
                    </div>
                    <Button size="sm" onClick={() => setSelectedTemplate(template)}>
                      <FileText className="mr-2 h-4 w-4" />
                      配置
                    </Button>
                  </div>
                </CardHeader>
                <CardContent className="space-y-3 pt-0">
                  <p className="line-clamp-2 text-sm text-muted-foreground">{template.notes}</p>
                  <div className="flex flex-wrap gap-2">
                    {template.models.slice(0, 4).map((model) => (
                      <code key={model} className="rounded bg-muted px-2 py-1 text-xs">{model}</code>
                    ))}
                    {template.models.length > 4 && (
                      <span className="text-xs text-muted-foreground">+{template.models.length - 4}</span>
                    )}
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        </TabsContent>

        <TabsContent value="providers" className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {providers.map((provider) => {
              const isDiscovering = discoveringProvider === provider.id && discoverModels.isPending
              const lastTest = provider.lastTest
              const testTone = lastTest?.success
                ? 'text-xs text-green-700 dark:text-green-300'
                : 'text-xs text-red-700 dark:text-red-300'
              const testSummary = lastTest?.success && typeof lastTest.modelCount === 'number'
                ? `已发现 ${lastTest.modelCount} 个模型`
                : lastTest?.message

              return (
                <Card key={provider.id} className="overflow-hidden">
                  <CardHeader className="pb-3">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <CardTitle className="truncate text-base">{provider.displayName}</CardTitle>
                        <div className="mt-2 flex flex-wrap items-center gap-2">
                          <Badge variant="outline">{PROVIDER_PROTOCOL_LABELS[provider.protocol]}</Badge>
                          <span className="text-xs text-muted-foreground">{provider.models.length} 模型</span>
                        </div>
                      </div>
                      <StatusBadge status={provider.status} />
                    </div>
                  </CardHeader>
                  <CardContent className="pt-0">
                    <p className="mb-2 truncate text-xs text-muted-foreground">{provider.baseUrl}</p>
                    <div className="mb-3 flex flex-wrap gap-2">
                      <Badge variant={provider.hasApiKey || !provider.apiKeyRequired ? 'success' : 'secondary'}>
                        {provider.hasApiKey || !provider.apiKeyRequired ? '可路由' : '缺少密钥'}
                      </Badge>
                      {provider.fidelityMode && <Badge variant="outline">{fidelityModeLabel(provider.fidelityMode)}</Badge>}
                      {provider.passthroughUnknownModels && <Badge variant="warning">透传未知模型</Badge>}
                    </div>
                    <div className="grid grid-cols-[1fr_auto] gap-2">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => handleDiscoverModels(provider.id)}
                        disabled={isDiscovering}
                      >
                        <Search className="mr-2 h-4 w-4" />
                        {isDiscovering ? '发现中' : '发现模型'}
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="justify-between"
                        onClick={() => setExpandedProvider(expandedProvider === provider.id ? null : provider.id)}
                      >
                        <span>列表</span>
                        {expandedProvider === provider.id ? <ChevronDown className="ml-2 h-4 w-4" /> : <ChevronRight className="ml-2 h-4 w-4" />}
                      </Button>
                    </div>
                    {lastTest && testSummary && (
                      <p className={`mt-2 truncate ${testTone}`}>{testSummary}</p>
                    )}
                    {expandedProvider === provider.id && (
                      <div className="mt-3 space-y-1">
                        {provider.models.map((model) => (
                          <div key={model} className="flex items-center justify-between gap-3 rounded-md border px-3 py-1.5">
                            <span className="min-w-0 truncate font-mono text-sm">{model}</span>
                            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => void copyText(`${provider.id}:${model}`)}>
                              <Copy className="h-3.5 w-3.5" />
                            </Button>
                          </div>
                        ))}
                      </div>
                    )}
                  </CardContent>
                </Card>
              )
            })}
          </div>
        </TabsContent>

        <TabsContent value="aliases" className="space-y-4">
          <TableToolbar
            actions={(
              <Button onClick={() => openAliasDialog()}>
                <Plus className="mr-2 h-4 w-4" />
                新建别名
              </Button>
            )}
          >
            <div className="text-sm text-muted-foreground">
              共 {aliases.length} 个模型别名；别名目标可以写成 provider:model。
            </div>
          </TableToolbar>

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

        <TabsContent value="routing" className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base">
                <Settings className="h-4 w-4" />
                默认提供商
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <p className="text-sm text-muted-foreground">
                同名模型会按供应商优先级解析；需要固定渠道时使用 provider:model，例如 openai:gpt-5.5。
              </p>
              <div className="space-y-2">
                <Label>默认提供商</Label>
                <Select value={defaultProvider} onValueChange={(value) => { setDefaultProvider(value); updateDefault.mutate(value) }}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {activeProviders.map((provider) => (
                      <SelectItem key={provider.id} value={provider.id}>{provider.displayName}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label>供应商优先级</Label>
                <div className="space-y-1">
                  {providers.map((provider, index) => (
                    <div key={provider.id} className="flex items-center gap-3 rounded-md border px-3 py-2">
                      <span className="w-6 text-sm text-muted-foreground">{index + 1}</span>
                      <span className="min-w-0 flex-1 truncate text-sm font-medium">{provider.displayName}</span>
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

      <Dialog open={showAliasDialog} onOpenChange={setShowAliasDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建别名</DialogTitle>
            <DialogDescription>创建模型别名以简化路由配置</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label>别名</Label>
              <Input value={aliasForm.alias} onChange={(event) => setAliasForm({ ...aliasForm, alias: event.target.value })} placeholder="例如: sonnet" />
            </div>
            <div className="space-y-2">
              <Label>目标</Label>
              <Input value={aliasForm.target} onChange={(event) => setAliasForm({ ...aliasForm, target: event.target.value })} placeholder="例如: openrouter:anthropic/claude-sonnet-4.6" />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAliasDialog(false)}>取消</Button>
            <Button onClick={() => {
              createAlias.mutate(aliasForm, { onSuccess: () => { setShowAliasDialog(false); setAliasForm({ alias: '', target: '' }) } })
            }} disabled={createAlias.isPending || !aliasForm.alias || !aliasForm.target}>创建</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!selectedTemplate} onOpenChange={() => setSelectedTemplate(null)}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{selectedTemplate?.displayName}</DialogTitle>
            <DialogDescription>
              复制到 config.toml 或 .env，重启 ModelPort 后生效。密钥仍建议放在环境变量里。
            </DialogDescription>
          </DialogHeader>
          {selectedTemplate && (
            <div className="grid gap-4 lg:grid-cols-2">
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <Label>TOML provider</Label>
                  <Button variant="outline" size="sm" onClick={() => void copyText(providerToml(selectedTemplate))}>
                    <Copy className="mr-2 h-4 w-4" />
                    一键复制
                  </Button>
                </div>
                <pre className="max-h-[340px] overflow-auto rounded-md bg-muted p-3 text-xs">{providerToml(selectedTemplate)}</pre>
              </div>
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <Label>环境变量</Label>
                  <Button variant="outline" size="sm" onClick={() => void copyText(providerEnv(selectedTemplate))}>
                    <Copy className="mr-2 h-4 w-4" />
                    一键复制
                  </Button>
                </div>
                <pre className="rounded-md bg-muted p-3 text-xs">{providerEnv(selectedTemplate)}</pre>
                <div className="rounded-md border p-3 text-sm text-muted-foreground">
                  <p className="font-medium text-foreground">默认模型</p>
                  <p className="mt-1 font-mono text-xs">{selectedTemplate.defaultModel}</p>
                  <p className="mt-3 font-medium text-foreground">建议别名</p>
                  <p className="mt-1 font-mono text-xs">{selectedTemplate.family.toLowerCase()} = "{selectedTemplate.id}:{selectedTemplate.defaultModel}"</p>
                </div>
              </div>
            </div>
          )}
          <DialogFooter>
            <Button onClick={() => setSelectedTemplate(null)}>完成</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function fidelityModeLabel(value: NonNullable<Provider['fidelityMode']>) {
  if (value === 'strict') return '严格无损'
  if (value === 'stability') return '稳定优先'
  return '尽量无损'
}
