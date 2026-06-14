import { Fragment, useMemo, useState } from 'react'
import {
  useProviders,
  useAliases,
  useCreateAlias,
  useCreateProvider,
  useDeleteProvider,
  useDeleteAlias,
  useDiscoverProviderModels,
  useSetProviderDisabled,
  useToggleModel,
  useUpdateDefaultModel,
  useUpdateDefaultProvider,
  useUpdateProvider,
} from '@/hooks'
import { PageHeader } from '@/components/shared/PageHeader'
import { TableToolbar } from '@/components/shared/TableToolbar'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { PaginationBar } from '@/components/shared/PaginationBar'
import { toast } from 'sonner'
import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Switch } from '@/components/ui/switch'
import { PROVIDER_PROTOCOL_LABELS } from '@/lib/constants'
import { cn, formatNumber, formatRelativeTime } from '@/lib/utils'
import { paginateItems } from '@/lib/pagination'
import { ApiError } from '@/lib/api-client'
import {
  MODEL_FAMILIES,
  PROVIDER_TEMPLATES,
  guessModelFamily,
  providerEnv,
  providerToml,
  type ProviderTemplate,
} from '@/lib/model-catalog'
import {
  AlertTriangle,
  ArrowUpDown,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  FileText,
  KeyRound,
  Layers3,
  ListChecks,
  Loader2,
  Pencil,
  Power,
  PowerOff,
  Plus,
  RefreshCw,
  Route,
  Search,
  Settings,
  Trash2,
} from 'lucide-react'
import type {
  FidelityMode,
  MaxTokensField,
  Provider,
  ProviderDeleteBlocked,
  ProviderModelInventory,
  ProviderProtocol,
  ProviderWritePayload,
} from '@/types'

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

interface ProviderFormState {
  id: string
  displayName: string
  protocol: ProviderProtocol
  baseUrl: string
  apiKeyEnv: string
  apiKeyRequired: boolean
  defaultModel: string
  models: string
  modelPrefixes: string
  passthroughUnknownModels: boolean
  maxTokensField: MaxTokensField
  deduplicateStreamText: boolean
  bufferStreamText: boolean
  fidelityMode: FidelityMode
  disabled: boolean
}

interface ProviderInventoryGroup {
  title: string
  brand: string
  originClassName: string
  items: ProviderModelInventory[]
}

const ALL = '__all__'
const DEFAULT_PROVIDER_FORM: ProviderFormState = {
  id: '',
  displayName: '',
  protocol: 'openai-compat',
  baseUrl: '',
  apiKeyEnv: '',
  apiKeyRequired: true,
  defaultModel: '',
  models: '',
  modelPrefixes: '',
  passthroughUnknownModels: false,
  maxTokensField: 'max_completion_tokens',
  deduplicateStreamText: false,
  bufferStreamText: false,
  fidelityMode: 'best_effort',
  disabled: false,
}
const PROVIDER_BRAND_NAMES: Record<string, string> = {
  deepseek: 'DeepSeek',
  deepseek_openai: 'DeepSeek',
  mimo: '小米 MiMo',
  openai: 'OpenAI',
  anthropic: 'Anthropic Claude',
  openrouter: 'OpenRouter',
  gemini: 'Google Gemini',
  dashscope: '阿里云百炼 Qwen',
  kimi: 'Moonshot Kimi',
  zhipu: '智谱 GLM',
  xai: 'xAI Grok',
  groq: 'Groq',
  mistral: 'Mistral AI',
  ark: '火山方舟 Doubao',
  ollama: 'Ollama',
  sglang: 'SGLang',
  vllm: 'vLLM',
  llamacpp: 'llama.cpp',
}
const OFFICIAL_PROVIDER_HOSTS: Record<string, string[]> = {
  deepseek: ['api.deepseek.com'],
  deepseek_openai: ['api.deepseek.com'],
  mimo: ['api.xiaomimimo.com'],
  openai: ['api.openai.com'],
  anthropic: ['api.anthropic.com'],
  gemini: ['generativelanguage.googleapis.com'],
  dashscope: ['dashscope.aliyuncs.com'],
  kimi: ['api.moonshot.cn'],
  zhipu: ['open.bigmodel.cn'],
  xai: ['api.x.ai'],
  groq: ['api.groq.com'],
  mistral: ['api.mistral.ai'],
  ark: ['ark.cn-beijing.volces.com'],
}
const LOCAL_PROVIDER_IDS = new Set(['ollama', 'local_sglang', 'local_vllm', 'local_llamacpp'])
const AGGREGATOR_PROVIDER_IDS = new Set(['openrouter'])
const MODEL_FAMILY_BRAND_NAMES: Record<string, string> = {
  OpenAI: 'OpenAI',
  Claude: 'Anthropic Claude',
  DeepSeek: 'DeepSeek',
  Gemini: 'Google Gemini',
  Qwen: 'Qwen',
  Kimi: 'Moonshot Kimi',
  GLM: '智谱 GLM',
  Grok: 'xAI Grok',
  Llama: 'Llama',
  Mistral: 'Mistral AI',
  Doubao: 'Doubao',
  Mimo: '小米 MiMo',
  Local: '本地模型',
  Custom: '自定义模型',
}

export function ModelsPage() {
  const { data: providers = [], isLoading } = useProviders()
  const { data: aliases = [] } = useAliases()
  const createAlias = useCreateAlias()
  const deleteAlias = useDeleteAlias()
  const discoverModels = useDiscoverProviderModels()
  const createProvider = useCreateProvider()
  const updateProvider = useUpdateProvider()
  const setProviderDisabled = useSetProviderDisabled()
  const deleteProvider = useDeleteProvider()
  const toggleModel = useToggleModel()
  const updateDefaultModel = useUpdateDefaultModel()
  const updateDefault = useUpdateDefaultProvider()

  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)
  const [expandedModel, setExpandedModel] = useState<string | null>(null)
  const [discoveringProvider, setDiscoveringProvider] = useState<string | null>(null)
  const [showAliasDialog, setShowAliasDialog] = useState(false)
  const [showProviderDialog, setShowProviderDialog] = useState(false)
  const [selectedTemplate, setSelectedTemplate] = useState<ProviderTemplate | null>(null)
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null)
  const [providerForm, setProviderForm] = useState<ProviderFormState>(DEFAULT_PROVIDER_FORM)
  const [deleteTarget, setDeleteTarget] = useState<Provider | null>(null)
  const [deleteBlock, setDeleteBlock] = useState<ProviderDeleteBlocked | null>(null)
  const [aliasForm, setAliasForm] = useState({ alias: '', target: '' })
  const [defaultProvider, setDefaultProvider] = useState(providers[0]?.id || 'mimo')
  const [search, setSearch] = useState('')
  const [family, setFamily] = useState(ALL)
  const [modelPage, setModelPage] = useState(1)
  const [modelPageSize, setModelPageSize] = useState(20)
  const [aliasPage, setAliasPage] = useState(1)
  const [aliasPageSize, setAliasPageSize] = useState(20)

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

  const filteredModelRows = useMemo(() => modelRows.filter((row) => {
    const haystack = [
      row.model,
      row.family,
      row.channels.map((channel) => channel.provider.displayName).join(' '),
      row.channels.map((channel) => modelRouteTitle(channel.provider, row.model)).join(' '),
      row.channels.map((channel) => channel.provider.id).join(' '),
    ].join(' ').toLowerCase()

    if (search && !haystack.includes(search.toLowerCase())) return false
    if (family !== ALL && row.family !== family) return false
    return true
  }), [modelRows, search, family])

  const modelWindow = paginateItems(filteredModelRows, modelPage, modelPageSize)
  const aliasWindow = paginateItems(aliases, aliasPage, aliasPageSize)

  const templateRows = PROVIDER_TEMPLATES.map((template) => ({
    ...template,
    configured: configuredProviderIds.has(template.id),
  }))
  const modelMutationKey = toggleModel.isPending && toggleModel.variables
    ? `${toggleModel.variables.providerId}:${toggleModel.variables.model}`
    : null
  const defaultModelMutationKey = updateDefaultModel.isPending && updateDefaultModel.variables
    ? `${updateDefaultModel.variables.providerId}:${updateDefaultModel.variables.model}`
    : null

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
      onSuccess: (result) => toast.success(`已发现 ${result.modelCount} 个模型`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '发现模型失败'),
    })
  }

  const openCreateProviderDialog = () => {
    setEditingProvider(null)
    setProviderForm(DEFAULT_PROVIDER_FORM)
    setShowProviderDialog(true)
  }

  const openEditProviderDialog = (provider: Provider) => {
    setEditingProvider(provider)
    setProviderForm(providerToForm(provider))
    setShowProviderDialog(true)
  }

  const closeProviderDialog = () => {
    setShowProviderDialog(false)
    setEditingProvider(null)
    setProviderForm(DEFAULT_PROVIDER_FORM)
  }

  const handleSubmitProvider = () => {
    const payload = providerPayloadFromForm(providerForm, !editingProvider)
    const options = {
      onSuccess: (provider: Provider) => {
        toast.success(editingProvider ? `已更新供应商 ${provider.displayName}` : `已新增供应商 ${provider.displayName}`)
        closeProviderDialog()
      },
      onError: (error: unknown) => toast.error(error instanceof Error ? error.message : '保存供应商失败'),
    }

    if (editingProvider) {
      updateProvider.mutate({ providerId: editingProvider.id, data: payload }, options)
    } else {
      createProvider.mutate(payload, options)
    }
  }

  const handleSetProviderDisabled = (provider: Provider) => {
    const disabled = provider.status !== 'disabled'
    setProviderDisabled.mutate({ providerId: provider.id, disabled }, {
      onSuccess: () => toast.success(disabled ? `已禁用 ${provider.displayName}` : `已恢复 ${provider.displayName}`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新供应商状态失败'),
    })
  }

  const handleDeleteProvider = (force = false) => {
    if (!deleteTarget) return
    deleteProvider.mutate({ providerId: deleteTarget.id, force }, {
      onSuccess: () => {
        toast.success(`已删除供应商 ${deleteTarget.displayName}`)
        setDeleteTarget(null)
        setDeleteBlock(null)
      },
      onError: (error) => {
        const blocked = providerDeleteBlockedFromError(error)
        if (blocked) {
          setDeleteBlock(blocked)
          return
        }
        toast.error(error instanceof Error ? error.message : '删除供应商失败')
      },
    })
  }

  const handleToggleProviderModel = (provider: Provider, model: string, enabled: boolean) => {
    toggleModel.mutate({ providerId: provider.id, model, enabled }, {
      onSuccess: () => toast.success(enabled ? `已启用 ${model}` : `已禁用 ${model}`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新模型状态失败'),
    })
  }

  const handleSetDefaultModel = (provider: Provider, model: string) => {
    updateDefaultModel.mutate({ providerId: provider.id, model }, {
      onSuccess: () => toast.success(`默认模型已设为 ${model}`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新默认模型失败'),
    })
  }

  const handleModelPageChange = (page: number) => {
    setModelPage(Math.min(Math.max(page, 1), modelWindow.totalPages))
    setExpandedModel(null)
  }

  const handleModelPageSizeChange = (pageSize: number) => {
    setModelPageSize(pageSize)
    setModelPage(1)
    setExpandedModel(null)
  }

  const handleAliasPageChange = (page: number) => {
    setAliasPage(Math.min(Math.max(page, 1), aliasWindow.totalPages))
  }

  const handleAliasPageSizeChange = (pageSize: number) => {
    setAliasPageSize(pageSize)
    setAliasPage(1)
  }

  if (isLoading) {
    return <LoadingPage />
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
                onChange={(event) => {
                  setSearch(event.target.value)
                  setModelPage(1)
                  setExpandedModel(null)
                }}
              />
            </div>
            <Select
              value={family}
              onValueChange={(value) => {
                setFamily(value)
                setModelPage(1)
                setExpandedModel(null)
              }}
            >
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
                  ) : modelWindow.items.map((row) => (
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
                            <p className="text-sm font-medium">{modelRouteTitle(row.defaultChannel.provider, row.model)}</p>
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
                                      <p className="font-medium">{modelRouteTitle(channel.provider, row.model)}</p>
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
            <CardFooter className="border-t px-4 py-3">
              <PaginationBar
                total={filteredModelRows.length}
                page={modelWindow.currentPage}
                pageSize={modelPageSize}
                totalPages={modelWindow.totalPages}
                start={modelWindow.start}
                end={modelWindow.end}
                totalLabel="个模型"
                onPageChange={handleModelPageChange}
                onPageSizeChange={handleModelPageSizeChange}
              />
            </CardFooter>
          </Card>
        </TabsContent>

        <TabsContent value="templates" className="space-y-4">
          <TableToolbar>
            <div className="text-sm text-muted-foreground">
              选择模板后复制 TOML 或 env 配置，重启后即可出现在模型库里。
            </div>
          </TableToolbar>
          <div className="grid items-start gap-4 md:grid-cols-2 xl:grid-cols-3">
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
          <TableToolbar
            actions={(
              <Button onClick={openCreateProviderDialog}>
                <Plus className="mr-2 h-4 w-4" />
                新增供应商
              </Button>
            )}
          >
            <div className="text-sm text-muted-foreground">
              发现模型会读取供应商模型列表并进入运行时可路由列表；这里可以新增、编辑、禁用或删除供应商。
            </div>
          </TableToolbar>
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {providers.map((provider) => (
              <ProviderCard
                key={provider.id}
                provider={provider}
                expanded={expandedProvider === provider.id}
                discovering={discoveringProvider === provider.id && discoverModels.isPending}
                onDiscover={() => handleDiscoverModels(provider.id)}
                onToggleList={() => setExpandedProvider(expandedProvider === provider.id ? null : provider.id)}
                onEdit={() => openEditProviderDialog(provider)}
                onToggleProvider={() => handleSetProviderDisabled(provider)}
                onDelete={() => { setDeleteTarget(provider); setDeleteBlock(null) }}
                onCopy={copyText}
                onAlias={openAliasDialog}
                onToggleModel={(model, enabled) => handleToggleProviderModel(provider, model, enabled)}
                onSetDefaultModel={(model) => handleSetDefaultModel(provider, model)}
                modelMutationKey={modelMutationKey}
                defaultModelMutationKey={defaultModelMutationKey}
              />
            ))}
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
                  {aliasWindow.items.map((alias) => (
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
                          onClick={() => deleteAlias.mutate(alias.alias, {
                            onError: (error) => toast.error(error instanceof Error ? error.message : '删除别名失败'),
                          })}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
            <CardFooter className="border-t px-4 py-3">
              <PaginationBar
                total={aliases.length}
                page={aliasWindow.currentPage}
                pageSize={aliasPageSize}
                totalPages={aliasWindow.totalPages}
                start={aliasWindow.start}
                end={aliasWindow.end}
                totalLabel="个别名"
                onPageChange={handleAliasPageChange}
                onPageSizeChange={handleAliasPageSizeChange}
              />
            </CardFooter>
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
                <Select value={defaultProvider} onValueChange={(value) => { setDefaultProvider(value); updateDefault.mutate(value, { onError: (error) => toast.error(error instanceof Error ? error.message : '更新默认供应商失败') }) }}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {activeProviders.map((provider) => (
                      <SelectItem key={provider.id} value={provider.id}>{providerDisplayTitle(provider)}</SelectItem>
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
                      <span className="min-w-0 flex-1 truncate text-sm font-medium">{providerDisplayTitle(provider)}</span>
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
              createAlias.mutate(aliasForm, {
                onSuccess: () => { setShowAliasDialog(false); setAliasForm({ alias: '', target: '' }) },
                onError: (error) => toast.error(error instanceof Error ? error.message : '创建别名失败'),
              })
            }} disabled={createAlias.isPending || !aliasForm.alias || !aliasForm.target}>创建</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={showProviderDialog} onOpenChange={(open) => { if (open) setShowProviderDialog(true); else closeProviderDialog() }}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{editingProvider ? '编辑供应商' : '新增供应商'}</DialogTitle>
            <DialogDescription>
              供应商配置会写入控制面存储并立即参与运行时路由，无需重启后端。
            </DialogDescription>
          </DialogHeader>
          <ScrollArea className="max-h-[68vh] pr-3">
            <div className="grid gap-4 md:grid-cols-2">
              <Field label="供应商 ID">
                <Input
                  value={providerForm.id}
                  disabled={!!editingProvider}
                  onChange={(event) => setProviderForm({ ...providerForm, id: event.target.value })}
                  placeholder="例如: siliconflow"
                />
              </Field>
              <Field label="显示名称">
                <Input
                  value={providerForm.displayName}
                  onChange={(event) => setProviderForm({ ...providerForm, displayName: event.target.value })}
                  placeholder="例如: 第三方 · OpenAI"
                />
              </Field>
              <Field label="协议">
                <Select value={providerForm.protocol} onValueChange={(value) => setProviderForm({ ...providerForm, protocol: value as ProviderProtocol })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="openai-compat">OpenAI 兼容</SelectItem>
                    <SelectItem value="anthropic">Anthropic</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <Field label="API Key 环境变量">
                <Input
                  value={providerForm.apiKeyEnv}
                  onChange={(event) => setProviderForm({ ...providerForm, apiKeyEnv: event.target.value })}
                  placeholder="例如: SILICONFLOW_API_KEY"
                />
              </Field>
              <Field label="Base URL" className="md:col-span-2">
                <Input
                  value={providerForm.baseUrl}
                  onChange={(event) => setProviderForm({ ...providerForm, baseUrl: event.target.value })}
                  placeholder="https://example.com/v1"
                />
              </Field>
              <Field label="默认模型">
                <Input
                  value={providerForm.defaultModel}
                  onChange={(event) => setProviderForm({ ...providerForm, defaultModel: event.target.value })}
                  placeholder="例如: gpt-4o-mini"
                />
              </Field>
              <Field label="Max Tokens 字段">
                <Select value={providerForm.maxTokensField} onValueChange={(value) => setProviderForm({ ...providerForm, maxTokensField: value as MaxTokensField })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="max_completion_tokens">max_completion_tokens</SelectItem>
                    <SelectItem value="max_tokens">max_tokens</SelectItem>
                    <SelectItem value="both">both</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <Field label="模型列表" className="md:col-span-2">
                <textarea
                  className="min-h-24 w-full rounded-md border bg-background px-3 py-2 text-sm outline-none ring-offset-background placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                  value={providerForm.models}
                  onChange={(event) => setProviderForm({ ...providerForm, models: event.target.value })}
                  placeholder={'每行一个模型，或用逗号分隔\nmimo-v2.5-pro\ngpt-4o-mini'}
                />
              </Field>
              <Field label="模型前缀" className="md:col-span-2">
                <Input
                  value={providerForm.modelPrefixes}
                  onChange={(event) => setProviderForm({ ...providerForm, modelPrefixes: event.target.value })}
                  placeholder="可选，例如 openai/, anthropic/"
                />
              </Field>
              <Field label="保真模式">
                <Select value={providerForm.fidelityMode} onValueChange={(value) => setProviderForm({ ...providerForm, fidelityMode: value as FidelityMode })}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="best_effort">尽量无损</SelectItem>
                    <SelectItem value="strict">严格无损</SelectItem>
                    <SelectItem value="stability">稳定优先</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <div className="space-y-3 rounded-md border bg-muted/20 p-3 md:col-span-2">
                <SwitchRow
                  label="需要 API Key"
                  checked={providerForm.apiKeyRequired}
                  onCheckedChange={(apiKeyRequired) => setProviderForm({ ...providerForm, apiKeyRequired })}
                />
                <SwitchRow
                  label="透传未知模型"
                  checked={providerForm.passthroughUnknownModels}
                  onCheckedChange={(passthroughUnknownModels) => setProviderForm({ ...providerForm, passthroughUnknownModels })}
                />
                <SwitchRow
                  label="流式文本去重"
                  checked={providerForm.deduplicateStreamText}
                  onCheckedChange={(deduplicateStreamText) => setProviderForm({ ...providerForm, deduplicateStreamText })}
                />
                <SwitchRow
                  label="缓冲非流式文本"
                  checked={providerForm.bufferStreamText}
                  onCheckedChange={(bufferStreamText) => setProviderForm({ ...providerForm, bufferStreamText })}
                />
                <SwitchRow
                  label="保存后禁用"
                  checked={providerForm.disabled}
                  onCheckedChange={(disabled) => setProviderForm({ ...providerForm, disabled })}
                />
              </div>
            </div>
          </ScrollArea>
          <DialogFooter>
            <Button variant="outline" onClick={closeProviderDialog}>取消</Button>
            <Button
              onClick={handleSubmitProvider}
              disabled={createProvider.isPending || updateProvider.isPending || !providerForm.id || !providerForm.baseUrl || !providerForm.defaultModel}
            >
              {createProvider.isPending || updateProvider.isPending ? '保存中' : '保存'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!deleteTarget} onOpenChange={(open) => { if (!open) { setDeleteTarget(null); setDeleteBlock(null) } }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>删除供应商</DialogTitle>
            <DialogDescription>
              删除后该供应商不会再参与路由；如果仍被别名、API Key 或团队策略引用，需要先确认依赖。
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <p className="text-sm">
              确认删除 <span className="font-semibold">{deleteTarget?.displayName}</span>？
            </p>
            {deleteBlock && (
              <div className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">
                <div className="flex items-center gap-2 font-medium">
                  <AlertTriangle className="h-4 w-4" />
                  发现 {deleteBlock.dependencies.length} 个依赖
                </div>
                <div className="mt-3 max-h-48 space-y-2 overflow-auto">
                  {deleteBlock.dependencies.map((dependency, idx) => (
                    <div key={`${dependency.type}:${dependency.id}:${idx}`} className="rounded bg-background/70 px-2 py-1.5">
                      <span className="font-medium">{dependencyLabel(dependency.type)}</span>
                      {dependency.name || dependency.id ? <span className="ml-2 font-mono text-xs">{dependency.name || dependency.id}</span> : null}
                      {dependency.field && <span className="ml-2 text-xs opacity-75">{dependency.field}</span>}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => { setDeleteTarget(null); setDeleteBlock(null) }}>取消</Button>
            {deleteBlock ? (
              <Button variant="destructive" onClick={() => handleDeleteProvider(true)} disabled={deleteProvider.isPending}>
                强制删除
              </Button>
            ) : (
              <Button variant="destructive" onClick={() => handleDeleteProvider(false)} disabled={deleteProvider.isPending}>
                删除
              </Button>
            )}
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

function providerDisplayTitle(provider: Provider) {
  const identity = providerIdentity(provider)
  const groups = providerModelGroups(provider)
  if (groups.length > 1) return `${identity.origin} · 多模型渠道`
  if (groups.length === 1) return groups[0].title
  return `${identity.origin} · ${identity.brand}`
}

function providerIdentity(provider: Provider) {
  const origin = providerOrigin(provider)
  return {
    origin,
    brand: PROVIDER_BRAND_NAMES[provider.id] ?? compactProviderName(provider.displayName),
    originClassName: providerOriginClassName(origin),
  }
}

function modelRouteTitle(provider: Provider, model: string) {
  const origin = providerOrigin(provider)
  return `${origin} · ${modelOwnerBrand(model)}`
}

function modelOwnerBrand(model: string) {
  const family = guessModelFamily(model)
  return MODEL_FAMILY_BRAND_NAMES[family] ?? family
}

function providerModelGroups(provider: Provider) {
  const groups = new Map<string, { title: string; brand: string; originClassName: string; models: string[] }>()
  const origin = providerOrigin(provider)
  const originClassName = providerOriginClassName(origin)

  for (const model of provider.models) {
    const brand = modelOwnerBrand(model)
    const title = `${origin} · ${brand}`
    const group = groups.get(title) || { title, brand, originClassName, models: [] }
    group.models.push(model)
    groups.set(title, group)
  }

  return Array.from(groups.values()).sort((a, b) => b.models.length - a.models.length || a.brand.localeCompare(b.brand))
}

function providerInventoryGroups(provider: Provider): ProviderInventoryGroup[] {
  const inventory: ProviderModelInventory[] = provider.modelInventory && provider.modelInventory.length > 0
    ? provider.modelInventory
    : provider.models.map((model): ProviderModelInventory => ({
        model,
        status: 'active',
        default: model === provider.defaultModel,
      }))
  const groups = new Map<string, ProviderInventoryGroup>()
  const origin = providerOrigin(provider)
  const originClassName = providerOriginClassName(origin)

  for (const item of inventory) {
    const brand = item.family || modelOwnerBrand(item.model)
    const title = `${origin} · ${brand}`
    const group = groups.get(title) || { title, brand, originClassName, items: [] }
    group.items.push(item)
    groups.set(title, group)
  }

  return Array.from(groups.values()).sort((a, b) => b.items.length - a.items.length || a.brand.localeCompare(b.brand))
}

function providerOrigin(provider: Provider) {
  const host = providerHost(provider)
  if (LOCAL_PROVIDER_IDS.has(provider.id) || isLocalHost(host)) return '本地'
  if (provider.id === 'custom') return '自定义'
  if (AGGREGATOR_PROVIDER_IDS.has(provider.id)) return '聚合平台'
  if ((OFFICIAL_PROVIDER_HOSTS[provider.id] || []).some((officialHost) => hostMatches(host, officialHost))) {
    return '官方'
  }
  return '第三方'
}

function providerOriginClassName(origin: string) {
  if (origin === '官方') return 'border-emerald-200 bg-emerald-50 text-emerald-700'
  if (origin === '第三方') return 'border-amber-200 bg-amber-50 text-amber-700'
  if (origin === '本地') return 'border-sky-200 bg-sky-50 text-sky-700'
  if (origin === '聚合平台') return 'border-violet-200 bg-violet-50 text-violet-700'
  return 'border-slate-200 bg-slate-50 text-slate-700'
}

function providerHost(provider: Provider) {
  try {
    return new URL(provider.baseUrl).hostname.toLowerCase().replace(/^www\./, '')
  } catch {
    return ''
  }
}

function hostMatches(host: string, expected: string) {
  return host === expected || host.endsWith(`.${expected}`)
}

function isLocalHost(host: string) {
  return host === 'localhost' || host === '127.0.0.1' || host === '0.0.0.0' || host === '::1'
}

function compactProviderName(value: string) {
  return value
    .replace(/\bOfficial\b/gi, '')
    .replace(/\bOpenAI[- ]Compatible\b/gi, 'OpenAI 兼容')
    .replace(/\s+/g, ' ')
    .trim()
}

function providerToForm(provider: Provider): ProviderFormState {
  return {
    id: provider.id,
    displayName: provider.displayName,
    protocol: provider.protocol,
    baseUrl: provider.baseUrl,
    apiKeyEnv: provider.apiKeyEnv || '',
    apiKeyRequired: provider.apiKeyRequired,
    defaultModel: provider.defaultModel,
    models: provider.models.join('\n'),
    modelPrefixes: provider.modelPrefixes.join(', '),
    passthroughUnknownModels: provider.passthroughUnknownModels,
    maxTokensField: provider.maxTokensField,
    deduplicateStreamText: provider.deduplicateStreamText,
    bufferStreamText: provider.bufferStreamText,
    fidelityMode: provider.fidelityMode || 'best_effort',
    disabled: provider.status === 'disabled',
  }
}

function providerPayloadFromForm(form: ProviderFormState, includeId: boolean): ProviderWritePayload {
  return {
    ...(includeId ? { id: form.id.trim() } : {}),
    displayName: form.displayName.trim() || form.id.trim(),
    protocol: form.protocol,
    baseUrl: form.baseUrl.trim(),
    apiKeyEnv: form.apiKeyEnv.trim() || null,
    apiKeyRequired: form.apiKeyRequired,
    defaultModel: form.defaultModel.trim(),
    models: parseList(form.models),
    modelPrefixes: parseList(form.modelPrefixes),
    passthroughUnknownModels: form.passthroughUnknownModels,
    maxTokensField: form.maxTokensField,
    deduplicateStreamText: form.deduplicateStreamText,
    bufferStreamText: form.bufferStreamText,
    fidelityMode: form.fidelityMode,
    disabled: form.disabled,
  }
}

function parseList(value: string): string[] {
  return Array.from(new Set(
    value
      .split(/[\n,]/)
      .map((item) => item.trim())
      .filter(Boolean),
  ))
}

function providerDeleteBlockedFromError(error: unknown): ProviderDeleteBlocked | null {
  if (!(error instanceof ApiError) || error.status !== 409) return null
  const payload = error.payload as Partial<ProviderDeleteBlocked> | undefined
  if (!payload?.blocked || !Array.isArray(payload.dependencies)) return null
  return payload as ProviderDeleteBlocked
}

function dependencyLabel(type: string) {
  if (type === 'alias') return '模型别名'
  if (type === 'apiKey') return 'API 密钥'
  if (type === 'team') return '团队策略'
  if (type === 'route') return '路由配置'
  return type
}

function ProviderCard({
  provider,
  expanded,
  discovering,
  onDiscover,
  onToggleList,
  onEdit,
  onToggleProvider,
  onDelete,
  onCopy,
  onAlias,
  onToggleModel,
  onSetDefaultModel,
  modelMutationKey,
  defaultModelMutationKey,
}: {
  provider: Provider
  expanded: boolean
  discovering: boolean
  onDiscover: () => void
  onToggleList: () => void
  onEdit: () => void
  onToggleProvider: () => void
  onDelete: () => void
  onCopy: (value: string) => Promise<void>
  onAlias: (alias?: string, target?: string) => void
  onToggleModel: (model: string, enabled: boolean) => void
  onSetDefaultModel: (model: string) => void
  modelMutationKey: string | null
  defaultModelMutationKey: string | null
}) {
  const credentialReady = provider.hasApiKey || !provider.apiKeyRequired
  const routeReady = provider.status === 'active' && credentialReady
  const lastTest = provider.lastTest
  const discoveredCount = lastTest?.modelCount ?? lastTest?.models?.length
  const defaultRoute = `${provider.id}:${provider.defaultModel}`
  const runtimeStatus = provider.runtimeStatus || provider.health?.status
  const modelListId = `provider-models-${provider.id}`
  const identity = providerIdentity(provider)
  const displayTitle = providerDisplayTitle(provider)
  const modelGroups = providerModelGroups(provider)
  const inventoryGroups = providerInventoryGroups(provider)

  return (
    <Card className="overflow-hidden" data-testid={`provider-card-${provider.id}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="truncate text-base">{displayTitle}</CardTitle>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Badge variant="outline" className={identity.originClassName}>{identity.origin}</Badge>
              <Badge variant="outline">{PROVIDER_PROTOCOL_LABELS[provider.protocol]}</Badge>
              <code className="rounded bg-muted px-2 py-1 text-xs">{provider.id}</code>
              {runtimeStatus && <StatusBadge status={runtimeStatus} />}
            </div>
          </div>
          <StatusBadge status={provider.status} />
        </div>
      </CardHeader>
      <CardContent className="space-y-4 pt-0">
        <div className="space-y-2 rounded-md border bg-muted/30 p-3 text-sm">
          <InfoRow label="Base URL" value={provider.baseUrl} mono />
          <InfoRow label="默认模型" value={provider.defaultModel} mono />
          <InfoRow label="可路由列表" value={`${provider.models.length} 个模型`} />
          {modelGroups.length > 0 && (
            <div className="grid grid-cols-[72px_minmax(0,1fr)] gap-3 pt-1">
              <span className="text-xs text-muted-foreground">模型归属</span>
              <div className="flex min-w-0 flex-wrap gap-1.5">
                {modelGroups.map((group) => (
                  <Badge key={group.title} variant="outline" className={cn('font-medium', group.originClassName)}>
                    {group.title} · {group.models.length}
                  </Badge>
                ))}
              </div>
            </div>
          )}
        </div>

        <div className="flex flex-wrap gap-2">
          <Badge variant={routeReady ? 'success' : credentialReady ? 'secondary' : 'destructive'}>
            {routeReady ? '可路由' : credentialReady ? '未激活' : '缺少密钥'}
          </Badge>
          {provider.fidelityMode && <Badge variant="outline">{fidelityModeLabel(provider.fidelityMode)}</Badge>}
          {provider.passthroughUnknownModels && <Badge variant="warning">透传未知模型</Badge>}
        </div>

        <div className="grid gap-2 sm:grid-cols-2">
          <Button
            size="sm"
            onClick={onDiscover}
            disabled={discovering || !credentialReady}
            aria-label={`发现 ${displayTitle} 模型`}
          >
            {discovering ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <RefreshCw className="mr-2 h-4 w-4" />}
            {discovering ? '发现中' : '发现模型'}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={onToggleList}
            aria-expanded={expanded}
            aria-controls={modelListId}
            aria-label={`${expanded ? '收起' : '查看'} ${displayTitle} 模型列表`}
          >
            <ListChecks className="mr-2 h-4 w-4" />
            {expanded ? '收起列表' : '查看列表'}
            {expanded ? <ChevronDown className="ml-auto h-4 w-4" /> : <ChevronRight className="ml-auto h-4 w-4" />}
          </Button>
          <Button variant="outline" size="sm" onClick={onEdit}>
            <Pencil className="mr-2 h-4 w-4" />
            编辑
          </Button>
          <Button variant="outline" size="sm" onClick={onToggleProvider}>
            {provider.status === 'disabled' ? <Power className="mr-2 h-4 w-4" /> : <PowerOff className="mr-2 h-4 w-4" />}
            {provider.status === 'disabled' ? '恢复' : '禁用'}
          </Button>
          <Button variant="outline" size="sm" className="text-destructive hover:text-destructive" onClick={onDelete}>
            <Trash2 className="mr-2 h-4 w-4" />
            删除
          </Button>
        </div>

        {!credentialReady && (
          <div className="flex items-start gap-2 rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-200">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>需要配置 {provider.apiKeyEnv || '供应商 API Key'} 后才能发现上游模型。</span>
          </div>
        )}

        {lastTest && (
          <div
            className={cn(
              'rounded-md border p-3 text-sm',
              lastTest.success
                ? 'border-green-200 bg-green-50 text-green-800 dark:border-green-900 dark:bg-green-950 dark:text-green-200'
                : 'border-red-200 bg-red-50 text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200',
            )}
          >
            <div className="flex items-center gap-2 font-medium">
              {lastTest.success ? <CheckCircle2 className="h-4 w-4" /> : <AlertTriangle className="h-4 w-4" />}
              <span>
                {lastTest.success ? `最近发现 ${discoveredCount ?? provider.models.length} 个模型，已合并到可路由列表` : '上次发现失败'}
              </span>
              <span className="ml-auto text-xs font-normal opacity-75">{formatRelativeTime(lastTest.testedAt)}</span>
            </div>
            <p className="mt-1 line-clamp-2 text-xs opacity-85">{lastTest.message}</p>
          </div>
        )}

        {expanded && (
          <div id={modelListId} className="rounded-md border">
            <div className="flex flex-wrap items-center justify-between gap-2 border-b bg-muted/30 px-3 py-2">
              <div>
                <p className="text-sm font-medium">可路由模型列表</p>
                <p className="text-xs text-muted-foreground">复制路由名或创建别名</p>
              </div>
              <Badge variant="secondary">{provider.models.length}</Badge>
            </div>

            {provider.models.length === 0 ? (
              <div className="px-3 py-6 text-center text-sm text-muted-foreground">
                暂无可路由模型，可先发现上游模型或在配置文件中补充 models。
              </div>
            ) : (
              <div className={cn('grid gap-3 p-2', inventoryGroups.length > 1 && 'xl:grid-cols-2')}>
                {inventoryGroups.map((group) => (
                  <ProviderModelGroupPanel
                    key={group.title}
                    group={group}
                    provider={provider}
                    defaultModel={provider.defaultModel}
                    compact={inventoryGroups.length > 1}
                    onAlias={onAlias}
                    onCopy={onCopy}
                    onToggleModel={onToggleModel}
                    onSetDefaultModel={onSetDefaultModel}
                    modelMutationKey={modelMutationKey}
                    defaultModelMutationKey={defaultModelMutationKey}
                  />
                ))}
              </div>
            )}

            <div className="border-t bg-muted/20 px-3 py-2">
              <Button variant="ghost" size="sm" className="w-full justify-start" onClick={() => void onCopy(defaultRoute)}>
                <Copy className="mr-2 h-4 w-4" />
                复制默认路由：<span className="ml-1 truncate font-mono">{defaultRoute}</span>
              </Button>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function ProviderModelGroupPanel({
  group,
  provider,
  defaultModel,
  compact,
  onCopy,
  onAlias,
  onToggleModel,
  onSetDefaultModel,
  modelMutationKey,
  defaultModelMutationKey,
}: {
  group: ProviderInventoryGroup
  provider: Provider
  defaultModel: string
  compact: boolean
  onCopy: (value: string) => Promise<void>
  onAlias: (alias?: string, target?: string) => void
  onToggleModel: (model: string, enabled: boolean) => void
  onSetDefaultModel: (model: string) => void
  modelMutationKey: string | null
  defaultModelMutationKey: string | null
}) {
  return (
    <div className="min-w-0 rounded-md border bg-background">
      <div className="flex items-center justify-between gap-2 border-b bg-muted/40 px-3 py-2">
        <span className="min-w-0 truncate text-sm font-medium">{group.title}</span>
        <Badge variant="outline" className={cn('shrink-0 font-medium', group.originClassName)}>{group.items.length} 个</Badge>
      </div>
      <ScrollArea className={cn(compact ? 'h-72' : 'max-h-80')}>
        <div className="space-y-1 p-2">
          {group.items.map((item) => {
            const routeName = `${provider.id}:${item.model}`
            const enabled = item.status !== 'disabled'
            const modelBusy = modelMutationKey === routeName
            const defaultBusy = defaultModelMutationKey === routeName
            return (
              <div key={item.model} className={cn('flex items-center gap-2 rounded-md px-2 py-2 hover:bg-muted/60', !enabled && 'opacity-65')}>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="min-w-0 truncate font-mono text-sm font-medium">{item.model}</span>
                    {item.model === defaultModel && <Badge variant="outline">默认</Badge>}
                    {!enabled && <Badge variant="secondary">已禁用</Badge>}
                  </div>
                  <p className="mt-1 truncate font-mono text-xs text-muted-foreground">{routeName}</p>
                </div>
                <Switch
                  checked={enabled}
                  disabled={modelBusy}
                  onCheckedChange={(checked) => onToggleModel(item.model, checked)}
                  aria-label={`${enabled ? '禁用' : '启用'} ${item.model}`}
                />
                <Button variant="ghost" size="icon" className="h-8 w-8 shrink-0" onClick={() => void onCopy(routeName)} aria-label={`复制 ${routeName}`}>
                  <Copy className="h-3.5 w-3.5" />
                </Button>
                {enabled && item.model !== defaultModel && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="shrink-0"
                    disabled={defaultBusy}
                    onClick={() => onSetDefaultModel(item.model)}
                  >
                    默认
                  </Button>
                )}
                <Button variant="outline" size="sm" className="shrink-0" disabled={!enabled} onClick={() => onAlias(item.model, routeName)}>
                  <Plus className="mr-1 h-3.5 w-3.5" />
                  别名
                </Button>
              </div>
            )
          })}
        </div>
      </ScrollArea>
    </div>
  )
}

function InfoRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="grid grid-cols-[72px_minmax(0,1fr)] gap-3">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className={cn('min-w-0 truncate text-xs', mono && 'font-mono')}>{value}</span>
    </div>
  )
}

function Field({ label, className, children }: { label: string; className?: string; children: React.ReactNode }) {
  return (
    <div className={cn('space-y-2', className)}>
      <Label>{label}</Label>
      {children}
    </div>
  )
}

function SwitchRow({
  label,
  checked,
  onCheckedChange,
}: {
  label: string
  checked: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <Label className="text-sm font-normal">{label}</Label>
      <Switch checked={checked} onCheckedChange={onCheckedChange} aria-label={label} />
    </div>
  )
}

function fidelityModeLabel(value: NonNullable<Provider['fidelityMode']>) {
  if (value === 'strict') return '严格无损'
  if (value === 'stability') return '稳定优先'
  return '尽量无损'
}
