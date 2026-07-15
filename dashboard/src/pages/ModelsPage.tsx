import { Fragment, useMemo, useState } from 'react'
import {
  useProviders,
  useAliases,
  useBulkToggleModels,
  useCreateAlias,
  useCreateProvider,
  useCreateProviderCredential,
  useDeleteProvider,
  useDeleteProviderCredential,
  useDeleteAlias,
  useDiscoverProviderModels,
  useSelectProviderCredential,
  useSetProviderDisabled,
  useSettings,
  useToggleModel,
  useUpdateDefaultModel,
  useUpdateDefaultProvider,
  useUpdateProviderOrder,
  useUpdateProvider,
  useUpdateProviderCredential,
  useUpdateProviderCredentialPoolMode,
} from '@/hooks'
import { useAuthStore } from '@/stores'
import { PageHeader } from '@/components/shared/PageHeader'
import { TableToolbar } from '@/components/shared/TableToolbar'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { ErrorState } from '@/components/shared/ErrorState'
import { EmptyState } from '@/components/shared/EmptyState'
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
import {
  MODEL_FAMILIES,
  PROVIDER_TEMPLATES,
  guessModelFamily,
  providerEnv,
  providerToml,
  type ProviderTemplate,
} from '@/lib/model-catalog'
import {
  CREDENTIAL_POOL_MODE_LABELS,
  DEFAULT_CREDENTIAL_FORM,
  DEFAULT_PROVIDER_FORM,
  PROVIDER_OPERATIONAL_FILTERS,
  credentialPayloadFromForm,
  credentialToForm,
  defaultToolStreamingArguments,
  defaultToolUseForProviderForm,
  dependencyLabel,
  modelRouteTitle,
  providerDeleteBlockedFromError,
  providerDisplayTitle,
  providerFilterCount,
  providerIdentity,
  providerInventoryGroups,
  providerInventoryItems,
  providerIsDegraded,
  providerIsHealthy,
  providerModelGroups,
  providerNeedsRecharge,
  providerPayloadFromForm,
  providerRuntimeState,
  providerToForm,
  type ProviderCredentialFormState,
  type ProviderFormState,
  type ProviderInventoryGroup,
  type ProviderOperationalFilter,
} from '@/features/models/model-data'
import {
  providerReadiness,
  validateAliasForm,
  validateCredentialForm,
  validateProviderForm,
  type ProviderReadinessLevel,
} from '@/features/models/operator-state'
import { moveProviderInOrder, normalizeProviderOrder, type ProviderOrderDirection } from '@/features/models/provider-order'
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleAlert,
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
  ProviderCredential,
  ProviderCredentialPoolMode,
  ProviderDeleteBlocked,
  ProviderProtocol,
  ToolStreamingArguments,
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
  enabledChannels: number
  preferredChannel: ModelChannel
}

const ALL = '__all__'

export function ModelsPage() {
  const {
    data: providers = [],
    isLoading,
    error: providersError,
    refetch: refetchProviders,
  } = useProviders()
  const {
    data: settings,
    isLoading: settingsLoading,
    error: settingsError,
    refetch: refetchSettings,
  } = useSettings()
  const { data: aliases = [], error: aliasesError, refetch: refetchAliases } = useAliases()
  const currentUser = useAuthStore((state) => state.currentUser)
  const canManage = currentUser?.role === 'admin'
  const createAlias = useCreateAlias()
  const deleteAlias = useDeleteAlias()
  const discoverModels = useDiscoverProviderModels()
  const createProvider = useCreateProvider()
  const updateProvider = useUpdateProvider()
  const setProviderDisabled = useSetProviderDisabled()
  const createProviderCredential = useCreateProviderCredential()
  const updateProviderCredential = useUpdateProviderCredential()
  const selectProviderCredential = useSelectProviderCredential()
  const updateProviderCredentialPoolMode = useUpdateProviderCredentialPoolMode()
  const deleteProviderCredential = useDeleteProviderCredential()
  const deleteProvider = useDeleteProvider()
  const toggleModel = useToggleModel()
  const bulkToggleModels = useBulkToggleModels()
  const updateDefaultModel = useUpdateDefaultModel()
  const updateDefault = useUpdateDefaultProvider()
  const updateProviderOrder = useUpdateProviderOrder()

  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)
  const [expandedModel, setExpandedModel] = useState<string | null>(null)
  const [discoveringProvider, setDiscoveringProvider] = useState<string | null>(null)
  const [showAliasDialog, setShowAliasDialog] = useState(false)
  const [showProviderDialog, setShowProviderDialog] = useState(false)
  const [aliasSubmitAttempted, setAliasSubmitAttempted] = useState(false)
  const [providerSubmitAttempted, setProviderSubmitAttempted] = useState(false)
  const [credentialSubmitAttempted, setCredentialSubmitAttempted] = useState(false)
  const [credentialDialogProvider, setCredentialDialogProvider] = useState<Provider | null>(null)
  const [selectedTemplate, setSelectedTemplate] = useState<ProviderTemplate | null>(null)
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null)
  const [editingCredential, setEditingCredential] = useState<ProviderCredential | null>(null)
  const [providerForm, setProviderForm] = useState<ProviderFormState>(DEFAULT_PROVIDER_FORM)
  const [credentialForm, setCredentialForm] = useState<ProviderCredentialFormState>(DEFAULT_CREDENTIAL_FORM)
  const [deleteTarget, setDeleteTarget] = useState<Provider | null>(null)
  const [deleteBlock, setDeleteBlock] = useState<ProviderDeleteBlocked | null>(null)
  const [deleteConfirmation, setDeleteConfirmation] = useState('')
  const [aliasForm, setAliasForm] = useState({ alias: '', target: '' })
  const [search, setSearch] = useState('')
  const [family, setFamily] = useState(ALL)
  const [providerFilter, setProviderFilter] = useState<ProviderOperationalFilter>('all')
  const [modelPage, setModelPage] = useState(1)
  const [modelPageSize, setModelPageSize] = useState(20)
  const [aliasPage, setAliasPage] = useState(1)
  const [aliasPageSize, setAliasPageSize] = useState(20)
  const [activeTab, setActiveTab] = useState('library')
  const [aliasDeleteTarget, setAliasDeleteTarget] = useState<string | null>(null)
  const [credentialDeleteTarget, setCredentialDeleteTarget] = useState<{
    provider: Provider
    credential: ProviderCredential
  } | null>(null)

  const configuredProviderIds = useMemo(() => new Set(providers.map((provider) => provider.id)), [providers])
  const defaultProvider = settings?.gateway.defaultProvider.trim() ?? ''
  const providerOrder = useMemo(
    () => normalizeProviderOrder(settings?.gateway.providerOrder, providers.map((provider) => provider.id)),
    [providers, settings?.gateway.providerOrder],
  )
  const orderedProviders = useMemo(() => {
    const providersById = new Map(providers.map((provider) => [provider.id, provider]))
    return providerOrder.flatMap((providerId) => {
      const provider = providersById.get(providerId)
      return provider ? [provider] : []
    })
  }, [providerOrder, providers])
  const activeProviders = orderedProviders.filter((provider) => provider.status === 'active')
  const rechargeProviders = useMemo(() => providers.filter(providerNeedsRecharge), [providers])
  const degradedProviders = useMemo(() => providers.filter(providerIsDegraded), [providers])
  const filteredProviders = useMemo(() => providers.filter((provider) => {
    if (providerFilter === 'recharge') return providerNeedsRecharge(provider)
    if (providerFilter === 'healthy') return providerIsHealthy(provider)
    if (providerFilter === 'degraded') return providerIsDegraded(provider)
    return true
  }), [providers, providerFilter])
  const totalConfiguredModels = providers.reduce((sum, provider) => sum + provider.models.length, 0)
  const capabilityRows = useMemo(() => providers.map((provider) => ({
    provider,
    toolUse: provider.toolUse ?? defaultToolUseForProviderForm(
      provider.id,
      provider.protocol,
      provider.deduplicateStreamText,
    ),
  })), [providers])
  const toolUseProviderCount = capabilityRows.filter((row) => row.toolUse.supported).length
  const defaultProviderRecord = providers.find((provider) => provider.id === defaultProvider)
  const providerStates = useMemo(
    () => providers.map((provider) => ({
      provider,
      readiness: providerReadiness(provider, provider.id === defaultProvider),
    })),
    [defaultProvider, providers],
  )
  const attentionProviderCount = providerStates.filter(({ readiness }) => readiness.level !== 'ready').length

  const modelRows = useMemo<ModelRow[]>(() => {
    const rows = new Map<string, ModelChannel[]>()

    orderedProviders.forEach((provider, priority) => {
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
          enabledChannels: sortedChannels.filter((channel) => channel.provider.status === 'active').length,
          preferredChannel: sortedChannels.find((channel) => channel.provider.status === 'active') ?? sortedChannels[0],
        }
      })
      .sort((a, b) => a.family.localeCompare(b.family) || a.model.localeCompare(b.model))
  }, [orderedProviders])

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
  const bulkModelMutation = bulkToggleModels.isPending && bulkToggleModels.variables
    ? {
        providerId: bulkToggleModels.variables.providerId,
        enabled: bulkToggleModels.variables.enabled,
      }
    : null
  const providerValidation = useMemo(() => validateProviderForm(providerForm), [providerForm])
  const credentialValidation = useMemo(
    () => validateCredentialForm(credentialForm, !editingCredential),
    [credentialForm, editingCredential],
  )
  const aliasValidation = useMemo(
    () => validateAliasForm(aliasForm.alias, aliasForm.target),
    [aliasForm.alias, aliasForm.target],
  )

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      toast.success('已复制到剪贴板')
    } catch {
      toast.error('复制失败，请手动复制')
    }
  }

  const openAliasDialog = (alias = '', target = '') => {
    setAliasForm({ alias, target })
    setAliasSubmitAttempted(false)
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
    setProviderSubmitAttempted(false)
    setShowProviderDialog(true)
  }

  const openEditProviderDialog = (provider: Provider) => {
    setEditingProvider(provider)
    setProviderForm(providerToForm(provider))
    setProviderSubmitAttempted(false)
    setShowProviderDialog(true)
  }

  const closeProviderDialog = () => {
    setShowProviderDialog(false)
    setEditingProvider(null)
    setProviderForm(DEFAULT_PROVIDER_FORM)
    setProviderSubmitAttempted(false)
  }

  const openCredentialDialog = (provider: Provider, credential?: ProviderCredential) => {
    setCredentialDialogProvider(provider)
    setEditingCredential(credential ?? null)
    setCredentialForm(credentialToForm(provider, credential))
    setCredentialSubmitAttempted(false)
  }

  const closeCredentialDialog = () => {
    setCredentialDialogProvider(null)
    setEditingCredential(null)
    setCredentialForm(DEFAULT_CREDENTIAL_FORM)
    setCredentialSubmitAttempted(false)
  }

  const handleSubmitProvider = () => {
    setProviderSubmitAttempted(true)
    if (!providerValidation.valid) {
      toast.error('请先修正表单中的错误')
      focusFirstInvalidDialogField()
      return
    }
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

  const handleSubmitCredential = () => {
    if (!credentialDialogProvider) return
    setCredentialSubmitAttempted(true)
    if (!credentialValidation.valid) {
      toast.error('请先修正账号表单中的错误')
      focusFirstInvalidDialogField()
      return
    }
    const data = credentialPayloadFromForm(credentialForm, !editingCredential)
    const options = {
      onSuccess: () => {
        toast.success(editingCredential ? '账号已更新' : '账号已新增')
        closeCredentialDialog()
      },
      onError: (error: unknown) => toast.error(error instanceof Error ? error.message : '保存账号失败'),
    }

    if (editingCredential) {
      updateProviderCredential.mutate({
        providerId: credentialDialogProvider.id,
        credentialId: editingCredential.id,
        data,
      }, options)
    } else {
      createProviderCredential.mutate({
        providerId: credentialDialogProvider.id,
        data,
      }, options)
    }
  }

  const handleSetProviderDisabled = (provider: Provider) => {
    const disabled = provider.status !== 'disabled'
    setProviderDisabled.mutate({ providerId: provider.id, disabled }, {
      onSuccess: () => toast.success(disabled ? `已禁用 ${provider.displayName}` : `已恢复 ${provider.displayName}`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新供应商状态失败'),
    })
  }

  const handleSelectProviderCredential = (provider: Provider, credentialId: string) => {
    selectProviderCredential.mutate({ providerId: provider.id, credentialId }, {
      onSuccess: () => toast.success(`已切换 ${provider.displayName} 账号`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '切换账号失败'),
    })
  }

  const handleUpdateProviderCredentialPoolMode = (provider: Provider, mode: ProviderCredentialPoolMode) => {
    updateProviderCredentialPoolMode.mutate({ providerId: provider.id, mode }, {
      onSuccess: () => toast.success(`已更新 ${provider.displayName} 号池策略`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新号池策略失败'),
    })
  }

  const handleDeleteProviderCredential = () => {
    if (!credentialDeleteTarget) return
    const { provider, credential } = credentialDeleteTarget
    deleteProviderCredential.mutate({ providerId: provider.id, credentialId: credential.id }, {
      onSuccess: () => {
        toast.success(`已删除账号 ${credential.name}`)
        setCredentialDeleteTarget(null)
      },
      onError: (error) => toast.error(error instanceof Error ? error.message : '删除账号失败'),
    })
  }

  const handleDeleteProvider = (force = false) => {
    if (!deleteTarget) return
    deleteProvider.mutate({ providerId: deleteTarget.id, force }, {
      onSuccess: () => {
        toast.success(`已删除供应商 ${deleteTarget.displayName}`)
        setDeleteTarget(null)
        setDeleteBlock(null)
        setDeleteConfirmation('')
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

  const handleBulkToggleProviderModels = (provider: Provider, enabled: boolean) => {
    const inventory = providerInventoryItems(provider)
    const models = inventory
      .filter((item) => {
        const itemEnabled = item.status !== 'disabled'
        if (enabled) return !itemEnabled
        return itemEnabled && item.model !== provider.defaultModel
      })
      .map((item) => item.model)

    if (models.length === 0) {
      toast.info(enabled ? '没有需要启用的模型' : '没有可禁用的非默认模型')
      return
    }

    bulkToggleModels.mutate({ providerId: provider.id, models, enabled }, {
      onSuccess: ({ updated }) => toast.success(enabled ? `已启用 ${updated} 个模型` : `已禁用 ${updated} 个非默认模型`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '批量更新模型状态失败'),
    })
  }

  const handleSetDefaultModel = (provider: Provider, model: string) => {
    updateDefaultModel.mutate({ providerId: provider.id, model }, {
      onSuccess: () => toast.success(`默认模型已设为 ${model}`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新默认模型失败'),
    })
  }

  const handleSetDefaultProvider = (providerId: string) => {
    updateDefault.mutate(providerId, {
      onSuccess: () => toast.success(`默认供应商已设为 ${providerId}`),
      onError: (error) => toast.error(
        error instanceof Error ? error.message : '更新默认供应商失败',
      ),
    })
  }

  const handleMoveProvider = (provider: Provider, direction: ProviderOrderDirection) => {
    const nextOrder = moveProviderInOrder(providerOrder, provider.id, direction)
    updateProviderOrder.mutate(nextOrder, {
      onSuccess: () => toast.success(`${providerDisplayTitle(provider)} 已${direction === 'up' ? '上移' : '下移'}，路由顺序已生效`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新 Provider 路由顺序失败'),
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

  if (providersError && providers.length === 0) {
    return (
      <ErrorState
        title="Provider 数据加载失败"
        message={errorMessage(providersError, '无法读取 Provider 与模型目录，请检查会话和后端状态。')}
        onRetry={() => void refetchProviders()}
      />
    )
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Provider 与模型"
        description="管理上游接入、凭证账号、模型目录、别名和默认路由"
      />

      {!canManage && (
        <div className="flex items-start gap-3 rounded-lg border border-blue-200 bg-blue-50 p-4 text-sm text-blue-900 dark:border-blue-900 dark:bg-blue-950 dark:text-blue-100" role="status">
          <KeyRound className="mt-0.5 h-4 w-4 shrink-0" />
          <div>
            <p className="font-medium">当前为只读视图</p>
            <p className="mt-1 text-xs opacity-80">只有管理员可以修改 Provider、凭证、模型状态、别名和默认路由；复制路由与查看诊断不受影响。</p>
          </div>
        </div>
      )}

      <ProviderRoutingOverview
        defaultProvider={defaultProviderRecord}
        defaultProviderId={defaultProvider}
        readiness={defaultProviderRecord ? providerReadiness(defaultProviderRecord, true) : null}
        routeState={settingsLoading ? 'loading' : settingsError && !settings ? 'error' : 'loaded'}
        providerCount={providers.length}
        attentionCount={attentionProviderCount}
        canManage={canManage}
        onOpenProviders={() => setActiveTab('providers')}
        onOpenRouting={() => setActiveTab('routing')}
      />

      <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
        <Card>
          <CardContent className="flex items-center gap-3 p-4">
            <div className="flex h-10 w-10 items-center justify-center rounded-md bg-primary/10 text-primary">
              <Layers3 className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm text-muted-foreground">唯一模型</p>
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
              <p className="text-sm text-muted-foreground">启用 Provider</p>
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
              <p className="text-sm text-muted-foreground">模型渠道</p>
              <p className="text-2xl font-semibold">{formatNumber(totalConfiguredModels)}</p>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="flex items-center gap-3 p-4">
            <div className="flex h-10 w-10 items-center justify-center rounded-md bg-amber-500/10 text-amber-600">
              <AlertTriangle className="h-5 w-5" />
            </div>
            <div>
              <p className="text-sm text-muted-foreground">需要处理</p>
              <p className="text-2xl font-semibold">{attentionProviderCount}</p>
            </div>
          </CardContent>
        </Card>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <div className="overflow-x-auto pb-1">
          <TabsList className="h-auto min-w-max justify-start">
            <TabsTrigger value="library">模型与路由</TabsTrigger>
            <TabsTrigger value="providers">Provider 与凭证</TabsTrigger>
            <TabsTrigger value="capabilities">协议能力</TabsTrigger>
            <TabsTrigger value="aliases">别名</TabsTrigger>
            <TabsTrigger value="routing">默认路由</TabsTrigger>
            <TabsTrigger value="templates">配置模板</TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="library" className="space-y-4">
          <TableToolbar>
            <div className="relative min-w-[240px] flex-1">
              <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                className="pl-8"
                aria-label="搜索模型、Provider 或渠道"
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
            <SelectTrigger className="w-[180px]" aria-label="筛选模型系列"><SelectValue placeholder="全部模型系列" /></SelectTrigger>
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
                    <TableHead>首选渠道（配置）</TableHead>
                    <TableHead className="text-center">供应商</TableHead>
                    <TableHead className="text-right">路由</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {filteredModelRows.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={5} className="p-0">
                        <EmptyState
                          icon={Layers3}
                          title={providers.length === 0 ? '尚未配置 Provider' : '没有匹配的模型'}
                          description={providers.length === 0
                            ? '先添加 Provider，再发现或填写上游模型。'
                            : '清除搜索词或切换模型系列后重试。'}
                          action={canManage && providers.length === 0 ? (
                            <Button size="sm" onClick={() => { setActiveTab('providers'); openCreateProviderDialog() }}>
                              <Plus className="mr-2 h-4 w-4" />
                              添加 Provider
                            </Button>
                          ) : undefined}
                        />
                      </TableCell>
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
                              aria-expanded={expandedModel === row.model}
                              aria-label={`${expandedModel === row.model ? '收起' : '展开'} ${row.model} 的渠道`}
                            >
                              {expandedModel === row.model ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                            </Button>
                            <span className="font-mono text-sm font-medium">{row.model}</span>
                          </div>
                        </TableCell>
                        <TableCell><Badge variant="outline">{row.family}</Badge></TableCell>
                        <TableCell>
                          <div className="space-y-1">
                            <p className="text-sm font-medium">{modelRouteTitle(row.preferredChannel.provider, row.model)}</p>
                            <p className="text-xs text-muted-foreground">{row.preferredChannel.provider.id}</p>
                          </div>
                        </TableCell>
                        <TableCell className="text-center">
                          <Badge variant={row.enabledChannels > 0 ? 'success' : 'secondary'}>
                            {row.enabledChannels} / {row.channels.length} 已启用
                          </Badge>
                        </TableCell>
                        <TableCell className="text-right">
                          <Button
                            variant="outline"
                            size="sm"
                            className="max-w-[220px]"
                            onClick={() => void copyText(row.preferredChannel.routeName)}
                            aria-label={`复制路由 ${row.preferredChannel.routeName}`}
                          >
                            <Copy className="mr-2 h-4 w-4" />
                            <span className="truncate">{row.preferredChannel.routeName}</span>
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
                                    {canManage && <Button variant="ghost" size="sm" onClick={() => openAliasDialog(row.model, channel.routeName)}>
                                      <Plus className="mr-2 h-4 w-4" />
                                      设为别名
                                    </Button>}
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
              模板只生成 TOML 与环境变量片段，不会修改运行中配置；保存文件并重启后才生效。
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
            actions={canManage ? (
              <Button onClick={openCreateProviderDialog}>
                <Plus className="mr-2 h-4 w-4" />
                新增 Provider
              </Button>
            ) : undefined}
          >
            <div className="flex flex-wrap items-center gap-2">
              {PROVIDER_OPERATIONAL_FILTERS.map((filter) => (
                <Button
                  key={filter.value}
                  type="button"
                  size="sm"
                  variant={providerFilter === filter.value ? 'default' : 'outline'}
                  onClick={() => {
                    setProviderFilter(filter.value)
                    setExpandedProvider(null)
                  }}
                >
                  {filter.label}
                  <span className="ml-2 rounded bg-background/20 px-1.5 py-0.5 text-[11px]">
                    {providerFilterCount(filter.value, providers, rechargeProviders, degradedProviders)}
                  </span>
                </Button>
              ))}
            </div>
          </TableToolbar>
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {filteredProviders.length === 0 ? (
              <Card className="md:col-span-2 xl:col-span-3">
                <CardContent className="p-0">
                  <EmptyState
                    icon={KeyRound}
                    title={providers.length === 0 ? '尚未配置 Provider' : '当前筛选没有结果'}
                    description={providers.length === 0 ? '添加首个上游接入后，才能配置凭证和模型目录。' : '切换状态筛选以查看其他 Provider。'}
                    action={canManage && providers.length === 0 ? (
                      <Button size="sm" onClick={openCreateProviderDialog}><Plus className="mr-2 h-4 w-4" />新增 Provider</Button>
                    ) : undefined}
                  />
                </CardContent>
              </Card>
            ) : filteredProviders.map((provider) => (
              <ProviderCard
                key={provider.id}
                provider={provider}
                isDefault={provider.id === defaultProvider}
                canManage={canManage}
                expanded={expandedProvider === provider.id}
                className={expandedProvider === provider.id ? 'md:col-span-2 xl:col-span-3' : undefined}
                discovering={discoveringProvider === provider.id && discoverModels.isPending}
                onDiscover={() => handleDiscoverModels(provider.id)}
                onToggleList={() => setExpandedProvider(expandedProvider === provider.id ? null : provider.id)}
                onEdit={() => openEditProviderDialog(provider)}
                onToggleProvider={() => handleSetProviderDisabled(provider)}
                onDelete={() => {
                  setDeleteTarget(provider)
                  setDeleteBlock(null)
                  setDeleteConfirmation('')
                }}
                onCopy={copyText}
                onAlias={openAliasDialog}
                onCreateCredential={() => openCredentialDialog(provider)}
                onEditCredential={(credential) => openCredentialDialog(provider, credential)}
                onSelectCredential={(credentialId) => handleSelectProviderCredential(provider, credentialId)}
                onUpdateCredentialPoolMode={(mode) => handleUpdateProviderCredentialPoolMode(provider, mode)}
                onDeleteCredential={(credential) => setCredentialDeleteTarget({ provider, credential })}
                onToggleModel={(model, enabled) => handleToggleProviderModel(provider, model, enabled)}
                onBulkToggleModels={(enabled) => handleBulkToggleProviderModels(provider, enabled)}
                onSetDefaultModel={(model) => handleSetDefaultModel(provider, model)}
                modelMutationKey={modelMutationKey}
                bulkModelMutation={bulkModelMutation}
                credentialBusy={selectProviderCredential.isPending || updateProviderCredentialPoolMode.isPending || deleteProviderCredential.isPending}
                defaultModelMutationKey={defaultModelMutationKey}
              />
            ))}
          </div>
        </TabsContent>

        <TabsContent value="capabilities" className="space-y-4">
          <div className="grid gap-4 md:grid-cols-3">
            <Card>
              <CardContent className="flex items-center gap-3 p-4">
                <div className="flex h-10 w-10 items-center justify-center rounded-md bg-blue-500/10 text-blue-600">
                  <ListChecks className="h-5 w-5" />
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">Tool Use Provider</p>
                  <p className="text-2xl font-semibold">{toolUseProviderCount} / {providers.length}</p>
                </div>
              </CardContent>
            </Card>
            <Card>
              <CardContent className="flex items-center gap-3 p-4">
                <div className="flex h-10 w-10 items-center justify-center rounded-md bg-green-500/10 text-green-600">
                  <CheckCircle2 className="h-5 w-5" />
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">Anthropic-compatible</p>
                  <p className="text-2xl font-semibold">{providers.filter((provider) => provider.protocol === 'anthropic').length}</p>
                </div>
              </CardContent>
            </Card>
            <Card>
              <CardContent className="flex items-center gap-3 p-4">
                <div className="flex h-10 w-10 items-center justify-center rounded-md bg-amber-500/10 text-amber-600">
                  <AlertTriangle className="h-5 w-5" />
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">需要关注</p>
                  <p className="text-2xl font-semibold">{degradedProviders.length}</p>
                </div>
              </CardContent>
            </Card>
          </div>

          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Provider</TableHead>
                    <TableHead>协议</TableHead>
                    <TableHead>Tool Use</TableHead>
                    <TableHead>tool_choice</TableHead>
                    <TableHead>并行工具</TableHead>
                    <TableHead>Arguments</TableHead>
                    <TableHead>保真模式</TableHead>
                    <TableHead>状态</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {capabilityRows.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={8} className="h-24 text-center text-muted-foreground">暂无 Provider</TableCell>
                    </TableRow>
                  ) : capabilityRows.map(({ provider, toolUse }) => (
                    <TableRow key={provider.id}>
                      <TableCell>
                        <div className="min-w-0 space-y-1">
                          <p className="truncate font-medium">{providerDisplayTitle(provider)}</p>
                          <p className="truncate font-mono text-xs text-muted-foreground">{provider.id}</p>
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline">{PROVIDER_PROTOCOL_LABELS[provider.protocol]}</Badge>
                      </TableCell>
                      <TableCell>
                        <Badge variant={toolUse.supported ? 'success' : 'secondary'}>
                          {toolUse.supported ? '支持' : '关闭'}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <Badge variant={toolUse.toolChoice ? 'outline' : 'secondary'}>
                          {toolUse.toolChoice ? '支持' : '不支持'}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <Badge variant={toolUse.parallelToolCalls ? 'outline' : 'secondary'}>
                          {toolUse.parallelToolCalls ? '允许' : '单工具'}
                        </Badge>
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline">{toolStreamingArgumentsLabel(toolUse.streamingArguments)}</Badge>
                      </TableCell>
                      <TableCell>
                        {provider.fidelityMode ? <Badge variant="outline">{fidelityModeLabel(provider.fidelityMode)}</Badge> : <span className="text-sm text-muted-foreground">默认</span>}
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap items-center gap-2">
                          <StatusBadge status={providerRuntimeState(provider)} />
                          {providerNeedsRecharge(provider) && <Badge variant="warning">等待充值</Badge>}
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="aliases" className="space-y-4">
          <TableToolbar
            actions={canManage ? (
              <Button onClick={() => openAliasDialog()}>
                <Plus className="mr-2 h-4 w-4" />
                新建别名
              </Button>
            ) : undefined}
          >
            <div className="text-sm text-muted-foreground">
              共 {aliases.length} 个模型别名；别名目标可以写成 provider:model。
            </div>
          </TableToolbar>

          {aliasesError && aliases.length === 0 ? (
            <Card>
              <CardContent>
                <ErrorState
                  title="别名加载失败"
                  message={errorMessage(aliasesError, '无法读取模型别名。')}
                  onRetry={() => void refetchAliases()}
                />
              </CardContent>
            </Card>
          ) : <Card>
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
                  {aliasWindow.items.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={5} className="p-0">
                        <EmptyState icon={Route} title="暂无模型别名" description="别名可以为稳定的客户端模型名绑定明确的 provider:model 路由。" />
                      </TableCell>
                    </TableRow>
                  ) : aliasWindow.items.map((alias) => (
                    <TableRow key={alias.alias}>
                      <TableCell className="font-mono font-medium">{alias.alias}</TableCell>
                      <TableCell className="text-muted-foreground">{alias.target}</TableCell>
                      <TableCell>{alias.resolvedProvider}</TableCell>
                      <TableCell className="font-mono text-sm">{alias.resolvedModel}</TableCell>
                      <TableCell>
                        {canManage && <Button
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 text-destructive"
                          onClick={() => setAliasDeleteTarget(alias.alias)}
                          aria-label={`删除别名 ${alias.alias}`}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>}
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
          </Card>}
        </TabsContent>

        <TabsContent value="routing" className="space-y-4">
          {settingsError && !settings ? (
            <Card>
              <CardContent>
                <ErrorState
                  title="默认路由加载失败"
                  message={errorMessage(settingsError, '无法读取当前默认 Provider 与路由顺序。')}
                  onRetry={() => void refetchSettings()}
                />
              </CardContent>
            </Card>
          ) : (
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2 text-base" role="heading" aria-level={2}>
                <Settings className="h-4 w-4" />
                默认路由策略
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <p className="text-sm text-muted-foreground">
                同名模型会按供应商优先级解析；需要固定渠道时使用 provider:model，例如 openai:gpt-5.5。
              </p>
              <div className="space-y-2">
                <Label>默认提供商</Label>
                <Select
                  value={defaultProvider || undefined}
                  disabled={!canManage || !settings || updateDefault.isPending || activeProviders.length === 0}
                  onValueChange={handleSetDefaultProvider}
                >
                  <SelectTrigger className="w-full" aria-label="默认 Provider">
                    <SelectValue placeholder="加载默认供应商…" />
                  </SelectTrigger>
                  <SelectContent>
                    {activeProviders.map((provider) => (
                      <SelectItem key={provider.id} value={provider.id}>{providerDisplayTitle(provider)}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <div className="flex items-start justify-between gap-4">
                  <div>
                    <Label>Provider 解析顺序</Label>
                    <p className="mt-1 text-xs text-muted-foreground">同名模型从上到下匹配可用 Provider；调整后立即保存并参与新请求路由。</p>
                  </div>
                  <span className="shrink-0 text-xs text-muted-foreground" aria-live="polite">
                    {updateProviderOrder.isPending ? '正在保存…' : `${orderedProviders.length} 个 Provider`}
                  </span>
                </div>
                {orderedProviders.length > 0 ? <div className="divide-y border-y">
                  {orderedProviders.map((provider, index) => (
                    <div key={provider.id} className="flex min-h-14 items-center gap-3 py-2">
                      <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-semibold text-muted-foreground" aria-label={`优先级 ${index + 1}`}>
                        {index + 1}
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="truncate text-sm font-medium">{providerDisplayTitle(provider)}</span>
                          {index === 0 && <Badge variant="secondary" className="text-[10px]">最高优先级</Badge>}
                          {provider.id === defaultProvider && <Badge variant="outline" className="text-[10px]">默认</Badge>}
                        </div>
                        <p className="truncate font-mono text-xs text-muted-foreground">{provider.id}</p>
                      </div>
                      <StatusBadge status={provider.status} />
                      <div className="flex shrink-0 items-center gap-1" aria-label={`${providerDisplayTitle(provider)} 排序操作`}>
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          className="h-8 w-8"
                          disabled={!canManage || updateProviderOrder.isPending || index === 0}
                          onClick={() => handleMoveProvider(provider, 'up')}
                          aria-label={`上移 ${providerDisplayTitle(provider)}`}
                          title="提高路由优先级"
                        >
                          <ArrowUp className="h-4 w-4" />
                        </Button>
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          className="h-8 w-8"
                          disabled={!canManage || updateProviderOrder.isPending || index === orderedProviders.length - 1}
                          onClick={() => handleMoveProvider(provider, 'down')}
                          aria-label={`下移 ${providerDisplayTitle(provider)}`}
                          title="降低路由优先级"
                        >
                          <ArrowDown className="h-4 w-4" />
                        </Button>
                      </div>
                    </div>
                  ))}
                </div> : (
                  <p className="border-y py-5 text-center text-sm text-muted-foreground">暂无可排序的 Provider</p>
                )}
                {!canManage && <p className="text-xs text-muted-foreground">当前账号为只读角色，只有管理员可以调整路由优先级。</p>}
              </div>
            </CardContent>
          </Card>
          )}
        </TabsContent>
      </Tabs>

      <Dialog
        open={showAliasDialog}
        onOpenChange={(open) => {
          setShowAliasDialog(open)
          if (!open) setAliasSubmitAttempted(false)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建别名</DialogTitle>
            <DialogDescription>创建模型别名以简化路由配置</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <Field label="别名" htmlFor="model-alias" error={aliasSubmitAttempted ? aliasValidation.errors.alias : undefined} description="客户端使用的稳定模型名；不能包含冒号。">
              <Input id="model-alias" value={aliasForm.alias} onChange={(event) => setAliasForm({ ...aliasForm, alias: event.target.value })} placeholder="例如: sonnet" aria-invalid={aliasSubmitAttempted && Boolean(aliasValidation.errors.alias)} />
            </Field>
            <Field label="目标路由" htmlFor="model-alias-target" error={aliasSubmitAttempted ? aliasValidation.errors.target : undefined} description="使用 provider:model 可固定上游渠道。">
              <Input id="model-alias-target" value={aliasForm.target} onChange={(event) => setAliasForm({ ...aliasForm, target: event.target.value })} placeholder="例如: openrouter:anthropic/claude-sonnet-4.6" aria-invalid={aliasSubmitAttempted && Boolean(aliasValidation.errors.target)} />
            </Field>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAliasDialog(false)}>取消</Button>
            <Button onClick={() => {
              setAliasSubmitAttempted(true)
              if (!aliasValidation.valid) {
                toast.error('请先修正别名和目标路由')
                focusFirstInvalidDialogField()
                return
              }
              createAlias.mutate({ alias: aliasForm.alias.trim(), target: aliasForm.target.trim() }, {
                onSuccess: () => {
                  toast.success(`已保存别名 ${aliasForm.alias.trim()}`)
                  setShowAliasDialog(false)
                  setAliasForm({ alias: '', target: '' })
                  setAliasSubmitAttempted(false)
                },
                onError: (error) => toast.error(error instanceof Error ? error.message : '创建别名失败'),
              })
            }} disabled={createAlias.isPending}>
              {createAlias.isPending ? <><Loader2 className="mr-2 h-4 w-4 animate-spin" />保存中</> : '保存别名'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={showProviderDialog} onOpenChange={(open) => { if (open) setShowProviderDialog(true); else closeProviderDialog() }}>
        <DialogContent className="max-h-[94vh] w-[calc(100vw-2rem)] max-w-3xl overflow-hidden">
          <DialogHeader>
            <DialogTitle>{editingProvider ? '编辑供应商' : '新增供应商'}</DialogTitle>
            <DialogDescription>
              供应商配置会写入控制面存储并立即参与运行时路由，无需重启后端。
            </DialogDescription>
          </DialogHeader>
          <ScrollArea className="max-h-[70vh] pr-3">
            <div className="grid gap-4 md:grid-cols-2">
              <FormSectionHeader
                title="1. Provider 身份与端点"
                description="定义稳定 ID、协议、上游根地址和默认凭证引用。真实密钥不会写入控制面。"
              />
              <Field label="Provider ID" htmlFor="provider-id" error={providerSubmitAttempted ? providerValidation.errors.id : undefined} description={editingProvider ? '稳定标识，创建后不可修改。' : '用于 provider:model 路由，只支持小写字母、数字、- 和 _。'} required>
                <Input
                  id="provider-id"
                  value={providerForm.id}
                  disabled={!!editingProvider}
                  onChange={(event) => setProviderForm({ ...providerForm, id: event.target.value.toLowerCase() })}
                  placeholder="例如: siliconflow"
                  aria-invalid={providerSubmitAttempted && Boolean(providerValidation.errors.id)}
                  aria-required="true"
                />
              </Field>
              <Field label="显示名称" htmlFor="provider-display-name" description="留空时使用 Provider ID。">
                <Input
                  id="provider-display-name"
                  value={providerForm.displayName}
                  onChange={(event) => setProviderForm({ ...providerForm, displayName: event.target.value })}
                  placeholder="例如: 第三方 · OpenAI"
                />
              </Field>
              <Field label="上游协议" description="选择上游实际实现的协议；网关会从 Anthropic Messages 或 OpenAI Chat 入口归一化后适配。" required>
                <Select
                  value={providerForm.protocol}
                  onValueChange={(value) => {
                    const protocol = value as ProviderProtocol
                    setProviderForm({
                      ...providerForm,
                      protocol,
                      toolStreamingArguments: defaultToolStreamingArguments(
                        protocol,
                        providerForm.deduplicateStreamText,
                        providerForm.id,
                      ),
                    })
                  }}
                >
                  <SelectTrigger aria-label="上游协议"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="openai-compat">OpenAI 兼容</SelectItem>
                    <SelectItem value="anthropic">Anthropic</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <Field label="默认 API Key 环境变量" htmlFor="provider-api-key-env" error={providerSubmitAttempted ? providerValidation.errors.apiKeyEnv : undefined} description="这里只保存变量名；清空会显式移除旧引用。">
                <Input
                  id="provider-api-key-env"
                  value={providerForm.apiKeyEnv}
                  onChange={(event) => setProviderForm({ ...providerForm, apiKeyEnv: event.target.value })}
                  placeholder="例如: SILICONFLOW_API_KEY"
                  aria-invalid={providerSubmitAttempted && Boolean(providerValidation.errors.apiKeyEnv)}
                />
              </Field>
              <Field label="API Base URL" htmlFor="provider-base-url" className="md:col-span-2" error={providerSubmitAttempted ? providerValidation.errors.baseUrl : undefined} description="填写 API 根路径，不要包含 /chat/completions、/messages、查询参数或凭证。" required>
                <Input
                  id="provider-base-url"
                  value={providerForm.baseUrl}
                  onChange={(event) => setProviderForm({ ...providerForm, baseUrl: event.target.value })}
                  placeholder="https://example.com/v1"
                  aria-invalid={providerSubmitAttempted && Boolean(providerValidation.errors.baseUrl)}
                  aria-required="true"
                />
              </Field>
              <FormSectionHeader
                title="2. 模型目录与请求字段"
                description="默认模型决定显式 provider 路由的回退；模型列表控制目录与可见性。"
              />
              <Field label="默认模型" htmlFor="provider-default-model" error={providerSubmitAttempted ? providerValidation.errors.defaultModel : undefined} description="保存时会自动加入模型列表。" required>
                <Input
                  id="provider-default-model"
                  value={providerForm.defaultModel}
                  onChange={(event) => setProviderForm({ ...providerForm, defaultModel: event.target.value })}
                  placeholder="例如: gpt-4o-mini"
                  aria-invalid={providerSubmitAttempted && Boolean(providerValidation.errors.defaultModel)}
                  aria-required="true"
                />
              </Field>
              <Field label="Max Tokens 字段" description="按上游兼容性选择请求字段名。">
                <Select value={providerForm.maxTokensField} onValueChange={(value) => setProviderForm({ ...providerForm, maxTokensField: value as MaxTokensField })}>
                  <SelectTrigger aria-label="Max Tokens 字段"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="max_completion_tokens">max_completion_tokens</SelectItem>
                    <SelectItem value="max_tokens">max_tokens</SelectItem>
                    <SelectItem value="both">both</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <Field label="模型列表" htmlFor="provider-models" className="md:col-span-2" description="每行或逗号分隔；发现模型后会与目录合并。">
                <textarea
                  id="provider-models"
                  className="min-h-24 w-full rounded-md border bg-background px-3 py-2 text-sm outline-none ring-offset-background placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                  value={providerForm.models}
                  onChange={(event) => setProviderForm({ ...providerForm, models: event.target.value })}
                  placeholder={'每行一个模型，或用逗号分隔\ndeepseek-v4-flash\ngpt-4o-mini'}
                />
              </Field>
              <Field label="模型前缀" htmlFor="provider-model-prefixes" className="md:col-span-2" description="可选；用于接受匹配前缀的模型名，不等同于已发现模型。">
                <Input
                  id="provider-model-prefixes"
                  value={providerForm.modelPrefixes}
                  onChange={(event) => setProviderForm({ ...providerForm, modelPrefixes: event.target.value })}
                  placeholder="可选，例如 openai/, anthropic/"
                />
              </Field>
              <FormSectionHeader
                title="3. 协议兼容与能力声明"
                description="这些开关描述适配器行为，不代表上游已通过真实 Tool Use 或流式验收。"
              />
              <Field label="保真模式" error={providerSubmitAttempted ? providerValidation.errors.fidelityMode : undefined} description="严格无损会拒绝无法无损映射的请求。">
                <Select value={providerForm.fidelityMode} onValueChange={(value) => setProviderForm({ ...providerForm, fidelityMode: value as FidelityMode })}>
                  <SelectTrigger
                    aria-label="保真模式"
                    aria-invalid={providerSubmitAttempted && Boolean(providerValidation.errors.fidelityMode)}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="best_effort">尽量无损</SelectItem>
                    <SelectItem value="strict">严格无损</SelectItem>
                    <SelectItem value="stability">稳定优先</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <Field label="Tool Use 参数流" description="native 直通；delta/cumulative/best_effort 用于 OpenAI-compatible 参数片段。">
                <Select
                  value={providerForm.toolStreamingArguments}
                  onValueChange={(value) => setProviderForm({ ...providerForm, toolStreamingArguments: value as ToolStreamingArguments })}
                >
                  <SelectTrigger aria-label="Tool Use 参数流"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="native">native</SelectItem>
                    <SelectItem value="delta">delta</SelectItem>
                    <SelectItem value="cumulative">cumulative</SelectItem>
                    <SelectItem value="best_effort">best_effort</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <div className="space-y-3 rounded-md border bg-muted/20 p-3 md:col-span-2" aria-label="Provider 能力开关">
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
                  onCheckedChange={(deduplicateStreamText) => setProviderForm({
                    ...providerForm,
                    deduplicateStreamText,
                    toolStreamingArguments: defaultToolStreamingArguments(
                      providerForm.protocol,
                      deduplicateStreamText,
                      providerForm.id,
                    ),
                  })}
                />
                <SwitchRow
                  label="缓冲非流式文本"
                  checked={providerForm.bufferStreamText}
                  onCheckedChange={(bufferStreamText) => setProviderForm({ ...providerForm, bufferStreamText })}
                />
                <SwitchRow
                  label="支持 Tool Use"
                  checked={providerForm.toolUseSupported}
                  onCheckedChange={(toolUseSupported) => setProviderForm({
                    ...providerForm,
                    toolUseSupported,
                    toolChoice: toolUseSupported ? providerForm.toolChoice : false,
                    parallelToolCalls: toolUseSupported ? providerForm.parallelToolCalls : false,
                  })}
                />
                <SwitchRow
                  label="支持 tool_choice"
                  checked={providerForm.toolChoice}
                  disabled={!providerForm.toolUseSupported}
                  onCheckedChange={(toolChoice) => setProviderForm({ ...providerForm, toolChoice })}
                />
                <SwitchRow
                  label="允许并行工具调用"
                  checked={providerForm.parallelToolCalls}
                  disabled={!providerForm.toolUseSupported}
                  onCheckedChange={(parallelToolCalls) => setProviderForm({ ...providerForm, parallelToolCalls })}
                />
                <SwitchRow
                  label="保存后禁用"
                  checked={providerForm.disabled}
                  onCheckedChange={(disabled) => setProviderForm({ ...providerForm, disabled })}
                />
              </div>
              {providerValidation.warnings.length > 0 && (
                <div className="space-y-2 rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-100 md:col-span-2" role="status">
                  <div className="flex items-center gap-2 font-medium"><AlertTriangle className="h-4 w-4" />保存前请确认</div>
                  <ul className="list-disc space-y-1 pl-5 text-xs">
                    {providerValidation.warnings.map((warning) => <li key={warning}>{warning}</li>)}
                  </ul>
                </div>
              )}
            </div>
          </ScrollArea>
          <DialogFooter>
            <Button variant="outline" onClick={closeProviderDialog}>取消</Button>
            <Button
              onClick={handleSubmitProvider}
              disabled={createProvider.isPending || updateProvider.isPending}
            >
              {createProvider.isPending || updateProvider.isPending
                ? <><Loader2 className="mr-2 h-4 w-4 animate-spin" />保存中</>
                : editingProvider ? '保存 Provider' : '创建 Provider'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!credentialDialogProvider} onOpenChange={(open) => { if (!open) closeCredentialDialog() }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editingCredential ? '编辑上游账号' : '新增上游账号'}</DialogTitle>
            <DialogDescription>
              账号只保存环境变量名；真实 API Key 仍放在 .env 或系统环境变量中。
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            {!editingCredential && (
              <Field label="账号 ID" htmlFor="credential-id" error={credentialSubmitAttempted ? credentialValidation.errors.id : undefined} description="用于账号池选择，创建后不可修改。" required>
                <Input
                  id="credential-id"
                  value={credentialForm.id}
                  onChange={(event) => setCredentialForm({ ...credentialForm, id: event.target.value.toLowerCase() })}
                  placeholder="例如: account-a"
                  aria-invalid={credentialSubmitAttempted && Boolean(credentialValidation.errors.id)}
                  aria-required="true"
                />
              </Field>
            )}
            <Field label="显示名称" htmlFor="credential-name" error={credentialSubmitAttempted ? credentialValidation.errors.name : undefined} required>
              <Input
                id="credential-name"
                value={credentialForm.name}
                onChange={(event) => setCredentialForm({ ...credentialForm, name: event.target.value })}
                placeholder="例如: Mimo 主账号"
                aria-invalid={credentialSubmitAttempted && Boolean(credentialValidation.errors.name)}
                aria-required="true"
              />
            </Field>
            <Field label="API Key 环境变量" htmlFor="credential-api-key-env" error={credentialSubmitAttempted ? credentialValidation.errors.apiKeyEnv : undefined} description="只保存变量名；新增变量后必须重启进程才能读取。" required>
              <Input
                id="credential-api-key-env"
                value={credentialForm.apiKeyEnv}
                onChange={(event) => setCredentialForm({ ...credentialForm, apiKeyEnv: event.target.value })}
                placeholder="例如: MIMO_OPENAI_API_KEY_ALT"
                aria-invalid={credentialSubmitAttempted && Boolean(credentialValidation.errors.apiKeyEnv)}
                aria-required="true"
              />
            </Field>
            <Field label="账号专用 Base URL" htmlFor="credential-base-url" error={credentialSubmitAttempted ? credentialValidation.errors.baseUrl : undefined} description="可选；用于同一 Provider 下的不同上游入口，留空沿用 Provider。">
              <Input
                id="credential-base-url"
                value={credentialForm.baseUrl}
                onChange={(event) => setCredentialForm({ ...credentialForm, baseUrl: event.target.value })}
                placeholder="可选，不填则沿用供应商 Base URL"
                aria-invalid={credentialSubmitAttempted && Boolean(credentialValidation.errors.baseUrl)}
              />
            </Field>
            <div className="rounded-md border bg-muted/20 p-3">
              <SwitchRow
                label="启用账号"
                checked={credentialForm.status === 'active'}
                onCheckedChange={(checked) => setCredentialForm({ ...credentialForm, status: checked ? 'active' : 'disabled' })}
              />
            </div>
            {credentialValidation.warnings.length > 0 && (
              <div className="flex items-start gap-2 rounded-md border border-amber-200 bg-amber-50 p-3 text-xs text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-100" role="status">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <span>{credentialValidation.warnings.join(' ')}</span>
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={closeCredentialDialog}>取消</Button>
            <Button
              onClick={handleSubmitCredential}
              disabled={
                createProviderCredential.isPending
                || updateProviderCredential.isPending
              }
            >
              {createProviderCredential.isPending || updateProviderCredential.isPending
                ? <><Loader2 className="mr-2 h-4 w-4 animate-spin" />保存中</>
                : editingCredential ? '保存账号' : '新增账号'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!deleteTarget} onOpenChange={(open) => {
        if (!open) {
          setDeleteTarget(null)
          setDeleteBlock(null)
          setDeleteConfirmation('')
        }
      }}>
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
            <div className="rounded-md border bg-muted/30 p-3 text-xs text-muted-foreground">
              Provider、账号池、模型覆盖和健康记录会被移除；基础配置中的 Provider 会留下禁用墓碑。首次删除会先检查默认路由、别名和访问策略依赖。
              强制删除会清理别名与路由控制项，但不会自动改写 API Key 或团队中的 allowedProviders 策略。
            </div>
            {deleteBlock && (
              <div className="space-y-3">
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
                <div className="space-y-2">
                  <Label htmlFor="provider-delete-confirm">
                    输入 <code className="rounded bg-muted px-1.5 py-0.5">{deleteTarget?.id}</code> 确认强制删除
                  </Label>
                  <Input
                    id="provider-delete-confirm"
                    value={deleteConfirmation}
                    onChange={(event) => setDeleteConfirmation(event.target.value)}
                    placeholder={deleteTarget?.id}
                    autoComplete="off"
                    spellCheck={false}
                  />
                </div>
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => { setDeleteTarget(null); setDeleteBlock(null) }}>取消</Button>
            {deleteBlock ? (
              <Button
                variant="destructive"
                onClick={() => handleDeleteProvider(true)}
                disabled={deleteProvider.isPending || deleteConfirmation !== deleteTarget?.id}
              >
                {deleteProvider.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                强制删除 Provider
              </Button>
            ) : (
              <Button variant="destructive" onClick={() => handleDeleteProvider(false)} disabled={deleteProvider.isPending}>
                {deleteProvider.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
                检查依赖并删除
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!credentialDeleteTarget} onOpenChange={(open) => { if (!open) setCredentialDeleteTarget(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>删除上游账号</DialogTitle>
            <DialogDescription>账号配置和健康记录会删除；真实环境变量不会被修改。</DialogDescription>
          </DialogHeader>
          <div className="rounded-md border bg-muted/30 p-3 text-sm">
            <p className="font-medium">{credentialDeleteTarget?.credential.name}</p>
            <p className="mt-1 font-mono text-xs text-muted-foreground">{credentialDeleteTarget?.credential.apiKeyEnv}</p>
            {credentialDeleteTarget?.credential.active && (
              <p className="mt-3 text-xs text-amber-700 dark:text-amber-300">这是当前账号；删除后系统会选择其他可用账号，若没有候选则 Provider 可能不可路由。</p>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCredentialDeleteTarget(null)}>取消</Button>
            <Button variant="destructive" onClick={handleDeleteProviderCredential} disabled={deleteProviderCredential.isPending}>
              {deleteProviderCredential.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              删除账号
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!aliasDeleteTarget} onOpenChange={(open) => { if (!open) setAliasDeleteTarget(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>删除模型别名</DialogTitle>
            <DialogDescription>使用该别名的客户端将不再解析到原目标。</DialogDescription>
          </DialogHeader>
          <p className="text-sm">确认删除别名 <code className="rounded bg-muted px-2 py-1">{aliasDeleteTarget}</code>？</p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAliasDeleteTarget(null)}>取消</Button>
            <Button
              variant="destructive"
              disabled={deleteAlias.isPending}
              onClick={() => {
                if (!aliasDeleteTarget) return
                deleteAlias.mutate(aliasDeleteTarget, {
                  onSuccess: () => {
                    toast.success(`已删除别名 ${aliasDeleteTarget}`)
                    setAliasDeleteTarget(null)
                  },
                  onError: (error) => toast.error(error instanceof Error ? error.message : '删除别名失败'),
                })
              }}
            >
              {deleteAlias.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              删除别名
            </Button>
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

function ProviderRoutingOverview({
  defaultProvider,
  defaultProviderId,
  readiness,
  routeState,
  providerCount,
  attentionCount,
  canManage,
  onOpenProviders,
  onOpenRouting,
}: {
  defaultProvider?: Provider
  defaultProviderId: string
  readiness: ReturnType<typeof providerReadiness> | null
  routeState: 'loading' | 'error' | 'loaded'
  providerCount: number
  attentionCount: number
  canManage: boolean
  onOpenProviders: () => void
  onOpenRouting: () => void
}) {
  const credentialReady = defaultProvider
    ? defaultProvider.hasApiKey || !defaultProvider.apiKeyRequired
    : false

  return (
    <Card className="overflow-hidden border-primary/20">
      <CardContent className="grid gap-5 p-5 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)_auto] lg:items-center">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">当前默认路由</p>
            {readiness && <ReadinessBadge level={readiness.level} label={readiness.label} />}
          </div>
          {routeState === 'loading' ? (
            <>
              <p className="mt-2 text-lg font-semibold">正在读取默认路由</p>
              <p className="mt-1 text-sm text-muted-foreground">等待当前运行设置返回。</p>
            </>
          ) : routeState === 'error' ? (
            <>
              <p className="mt-2 text-lg font-semibold">默认路由状态不可用</p>
              <p className="mt-1 text-sm text-muted-foreground">打开“默认路由”查看错误并重试。</p>
            </>
          ) : defaultProvider ? (
            <>
              <p className="mt-2 truncate text-lg font-semibold">{providerDisplayTitle(defaultProvider)}</p>
              <p className="mt-1 truncate font-mono text-sm text-muted-foreground">
                {defaultProvider.id}:{defaultProvider.defaultModel}
              </p>
            </>
          ) : defaultProviderId ? (
            <>
              <p className="mt-2 text-lg font-semibold">默认 Provider 不在当前目录</p>
              <p className="mt-1 truncate font-mono text-sm text-muted-foreground">{defaultProviderId}</p>
            </>
          ) : (
            <>
              <p className="mt-2 text-lg font-semibold">尚未形成默认路由</p>
              <p className="mt-1 text-sm text-muted-foreground">添加并启用 Provider 后，再选择默认入口。</p>
            </>
          )}
        </div>

        {routeState === 'loaded' ? (
          <div className="grid grid-cols-3 gap-2 text-center">
            <RouteStage label="Provider" ready={Boolean(defaultProvider && defaultProvider.status === 'active')} />
            <RouteStage label="凭证" ready={credentialReady} />
            <RouteStage label="模型" ready={Boolean(defaultProvider?.models.length)} />
          </div>
        ) : (
          <div className="rounded-md border bg-muted/30 p-4 text-center text-sm text-muted-foreground">
            路由检查暂不可用
          </div>
        )}

        <div className="flex flex-wrap gap-2 lg:max-w-[220px] lg:justify-end">
          <Button variant="outline" size="sm" onClick={onOpenProviders}>
            查看 Provider
          </Button>
          {canManage && (
            <Button size="sm" onClick={onOpenRouting}>
              管理默认路由
            </Button>
          )}
          <p className="w-full text-xs text-muted-foreground lg:text-right">
            {providerCount} 个 Provider · {attentionCount} 个需处理
          </p>
        </div>
      </CardContent>
    </Card>
  )
}

function RouteStage({ label, ready }: { label: string; ready: boolean }) {
  return (
    <div className={cn(
      'rounded-md border px-2 py-2 text-xs',
      ready
        ? 'border-green-200 bg-green-50 text-green-800 dark:border-green-900 dark:bg-green-950 dark:text-green-200'
        : 'border-red-200 bg-red-50 text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-200',
    )}>
      {ready ? <CheckCircle2 className="mx-auto mb-1 h-4 w-4" /> : <CircleAlert className="mx-auto mb-1 h-4 w-4" />}
      {label}
    </div>
  )
}

function ReadinessBadge({ level, label }: { level: ProviderReadinessLevel; label: string }) {
  const variant = level === 'ready'
    ? 'success'
    : level === 'blocked'
      ? 'destructive'
      : level === 'attention'
        ? 'warning'
        : 'secondary'
  return <Badge variant={variant}>{label}</Badge>
}

function ProviderReadinessNotice({ readiness }: { readiness: ReturnType<typeof providerReadiness> }) {
  const Icon = readiness.level === 'ready' ? CheckCircle2 : readiness.level === 'disabled' ? PowerOff : AlertTriangle
  return (
    <div className={cn(
      'flex items-start gap-3 rounded-md border p-3 text-sm',
      readiness.level === 'ready' && 'border-green-200 bg-green-50 text-green-900 dark:border-green-900 dark:bg-green-950 dark:text-green-100',
      readiness.level === 'attention' && 'border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-100',
      readiness.level === 'blocked' && 'border-red-200 bg-red-50 text-red-900 dark:border-red-900 dark:bg-red-950 dark:text-red-100',
      readiness.level === 'disabled' && 'bg-muted/40 text-muted-foreground',
    )} role="status">
      <Icon className="mt-0.5 h-4 w-4 shrink-0" />
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <p className="font-medium">{readiness.label}</p>
          <span className="text-xs opacity-75">{readiness.detail}</span>
        </div>
        <p className="mt-1 text-xs opacity-80">下一步：{readiness.nextStep}</p>
      </div>
    </div>
  )
}

function ProviderCard({
  provider,
  isDefault,
  canManage,
  expanded,
  className,
  discovering,
  onDiscover,
  onToggleList,
  onEdit,
  onToggleProvider,
  onDelete,
  onCopy,
  onAlias,
  onCreateCredential,
  onEditCredential,
  onSelectCredential,
  onUpdateCredentialPoolMode,
  onDeleteCredential,
  onToggleModel,
  onBulkToggleModels,
  onSetDefaultModel,
  modelMutationKey,
  bulkModelMutation,
  credentialBusy,
  defaultModelMutationKey,
}: {
  provider: Provider
  isDefault: boolean
  canManage: boolean
  expanded: boolean
  className?: string
  discovering: boolean
  onDiscover: () => void
  onToggleList: () => void
  onEdit: () => void
  onToggleProvider: () => void
  onDelete: () => void
  onCopy: (value: string) => Promise<void>
  onAlias: (alias?: string, target?: string) => void
  onCreateCredential: () => void
  onEditCredential: (credential: ProviderCredential) => void
  onSelectCredential: (credentialId: string) => void
  onUpdateCredentialPoolMode: (mode: ProviderCredentialPoolMode) => void
  onDeleteCredential: (credential: ProviderCredential) => void
  onToggleModel: (model: string, enabled: boolean) => void
  onBulkToggleModels: (enabled: boolean) => void
  onSetDefaultModel: (model: string) => void
  modelMutationKey: string | null
  bulkModelMutation: { providerId: string; enabled: boolean } | null
  credentialBusy: boolean
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
  const credentials = provider.credentials ?? []
  const activeCredential = credentials.find((credential) => credential.active)
    ?? credentials.find((credential) => credential.id === provider.activeCredentialId)
    ?? null
  const credentialPoolMode = provider.credentialPoolMode ?? 'failover'
  const modelGroups = providerModelGroups(provider)
  const inventoryGroups = providerInventoryGroups(provider)
  const inventoryItems = providerInventoryItems(provider)
  const enabledModelCount = inventoryItems.filter((item) => item.status !== 'disabled').length
  const disabledModelCount = inventoryItems.length - enabledModelCount
  const disableCandidateCount = inventoryItems.filter((item) => item.status !== 'disabled' && item.model !== provider.defaultModel).length
  const isBulkUpdating = bulkModelMutation?.providerId === provider.id
  const rechargeBadge = provider.health?.rechargeRequired ? '等待充值' : null
  const readiness = providerReadiness(provider, isDefault)

  return (
    <Card className={cn('overflow-hidden transition-all', className)} data-testid={`provider-card-${provider.id}`}>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="truncate text-base">{displayTitle}</CardTitle>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Badge variant="outline" className={identity.originClassName}>{identity.origin}</Badge>
              <Badge variant="outline">{PROVIDER_PROTOCOL_LABELS[provider.protocol]}</Badge>
              <code className="rounded bg-muted px-2 py-1 text-xs">{provider.id}</code>
              {isDefault && <Badge variant="outline">默认 Provider</Badge>}
              {runtimeStatus && <StatusBadge status={runtimeStatus} />}
              {rechargeBadge && <Badge variant="warning">{rechargeBadge}</Badge>}
            </div>
          </div>
          <StatusBadge status={provider.status} />
        </div>
      </CardHeader>
      <CardContent className="space-y-4 pt-0">
        <div className="space-y-2 rounded-md border bg-muted/30 p-3 text-sm">
          <InfoRow label="Base URL" value={provider.baseUrl} mono />
          <InfoRow label="默认模型" value={provider.defaultModel} mono />
          <InfoRow label="启用模型目录" value={`${provider.models.length} 个模型`} />
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

        <ProviderReadinessNotice readiness={readiness} />

        <div className="flex flex-wrap gap-2">
          <Badge variant={routeReady ? 'success' : credentialReady ? 'secondary' : 'destructive'}>
            {routeReady ? '配置已启用' : credentialReady ? '未激活' : '缺少密钥'}
          </Badge>
          {provider.fidelityMode && <Badge variant="outline">{fidelityModeLabel(provider.fidelityMode)}</Badge>}
          {provider.toolUse?.supported && <Badge variant="outline">Tool Use</Badge>}
          {provider.toolUse?.supported && (
            <Badge variant="outline">工具流 {toolStreamingArgumentsLabel(provider.toolUse.streamingArguments)}</Badge>
          )}
          {provider.toolUse && !provider.toolUse.parallelToolCalls && <Badge variant="secondary">单工具调用</Badge>}
          {provider.passthroughUnknownModels && <Badge variant="warning">透传未知模型</Badge>}
        </div>

        {provider.health?.recommendedAction && provider.health.failureKind !== 'none' && (
          <div className="flex items-start gap-2 rounded-md border border-amber-200 bg-amber-50 p-3 text-xs text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-100">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="min-w-0 space-y-1">
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                {rechargeBadge && <Badge variant="warning">{rechargeBadge}</Badge>}
                <p className="font-medium">{provider.health.recommendedAction}</p>
              </div>
              {provider.health.lastError && (
                <p className="line-clamp-2 opacity-80">{provider.health.lastError}</p>
              )}
            </div>
          </div>
        )}

        <div className="rounded-md border bg-muted/20 p-3">
          <div className="mb-3 grid gap-3 md:grid-cols-[minmax(0,1fr)_220px]">
            <div>
              <p className="text-sm font-medium">上游账号</p>
              <p className="text-xs text-muted-foreground">
                {credentials.length > 0 ? `${credentials.length} 个账号 · ${CREDENTIAL_POOL_MODE_LABELS[credentialPoolMode]}` : '默认凭证'}
              </p>
            </div>
            <div className="flex min-w-0 items-center gap-2">
              <Select
                value={credentialPoolMode}
                onValueChange={(value) => onUpdateCredentialPoolMode(value as ProviderCredentialPoolMode)}
                disabled={!canManage || credentialBusy || credentials.length === 0}
              >
                <SelectTrigger className="h-9 min-w-0" aria-label={`${displayTitle} 账号池策略`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="manual">手动</SelectItem>
                  <SelectItem value="failover">故障切换</SelectItem>
                  <SelectItem value="round_robin">轮询</SelectItem>
                </SelectContent>
              </Select>
              {canManage && <Button variant="outline" size="sm" onClick={onCreateCredential}>
                <Plus className="h-3.5 w-3.5" />
                新增
              </Button>}
            </div>
          </div>
          {credentials.length === 0 ? (
            <div className="flex flex-wrap items-center gap-2 text-sm">
              <Badge variant={credentialReady ? 'success' : 'destructive'}>
                {credentialReady ? '默认环境变量可用' : '缺少默认密钥'}
              </Badge>
              <code className="rounded bg-background px-2 py-1 text-xs">{provider.apiKeyEnv || '无需 API Key'}</code>
            </div>
          ) : (
            <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto]">
              <Select
                value={activeCredential?.id || provider.activeCredentialId || credentials[0]?.id}
                onValueChange={onSelectCredential}
                disabled={!canManage || credentialBusy}
              >
                <SelectTrigger aria-label={`${displayTitle} 当前账号`}>
                  <SelectValue placeholder="选择账号" />
                </SelectTrigger>
                <SelectContent>
                  {credentials.map((credential) => (
                    <SelectItem key={credential.id} value={credential.id} disabled={credential.status === 'disabled'}>
                      {credential.name} · {credential.apiKeyEnv}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <div className="flex flex-wrap items-center gap-2">
                {activeCredential && (
                  <>
                    <Badge variant={activeCredential.hasApiKey ? 'success' : 'destructive'}>
                      {activeCredential.hasApiKey ? 'Key 可用' : 'Key 缺失'}
                    </Badge>
                    {canManage && <Button variant="outline" size="sm" onClick={() => onEditCredential(activeCredential)} disabled={credentialBusy}>
                      <Pencil className="h-3.5 w-3.5" />
                      编辑
                    </Button>}
                    {canManage && <Button
                      variant="outline"
                      size="sm"
                      className="text-destructive hover:text-destructive"
                      onClick={() => onDeleteCredential(activeCredential)}
                      disabled={credentialBusy}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                      删除
                    </Button>}
                  </>
                )}
              </div>
              {activeCredential && (
                <div className="min-w-0 space-y-1 text-xs text-muted-foreground lg:col-span-2">
                  <p className="truncate">
                    环境变量：<code className="text-foreground">{activeCredential.apiKeyEnv}</code>
                  </p>
                  {activeCredential.baseUrl && (
                    <p className="truncate">
                      Base URL：<code className="text-foreground">{activeCredential.baseUrl}</code>
                    </p>
                  )}
                </div>
              )}
              <div className="space-y-2 lg:col-span-2">
                {credentials.map((credential) => {
                  const health = credential.health
                  const healthStatus = health?.status ?? (credential.hasApiKey ? 'healthy' : 'degraded')
                  const credentialRechargeBadge = health?.rechargeRequired ? '等待充值' : null
                  return (
                    <div key={credential.id} className="grid gap-2 rounded-md border bg-background/70 px-3 py-2 md:grid-cols-[minmax(0,1fr)_auto]">
                      <div className="min-w-0">
                        <div className="flex min-w-0 flex-wrap items-center gap-2">
                          <span className="truncate text-sm font-medium">{credential.name}</span>
                          {credential.active && <Badge variant="outline">当前</Badge>}
                          {credential.status === 'disabled' && <Badge variant="secondary">禁用</Badge>}
                        </div>
                        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                          <code className="max-w-full truncate text-foreground">{credential.apiKeyEnv}</code>
                          {health?.lastUsedAt && <span>最近 {formatRelativeTime(health.lastUsedAt)}</span>}
                        </div>
                      </div>
                      <div className="flex flex-wrap items-center gap-1.5 md:justify-end">
                        <Badge variant={credential.hasApiKey ? 'success' : 'destructive'}>
                          {credential.hasApiKey ? 'Key 可用' : 'Key 缺失'}
                        </Badge>
                        <Badge variant={credentialHealthVariant(healthStatus)}>
                          {credentialHealthLabel(healthStatus)}
                        </Badge>
                        {credentialRechargeBadge && <Badge variant="warning">{credentialRechargeBadge}</Badge>}
                        <span className="rounded bg-muted px-2 py-1 text-xs text-muted-foreground">
                          {health?.requestsTotal ? `${formatNumber(health.requestsTotal)} 次 · ${Math.round(health.successRate)}%` : '暂无请求'}
                        </span>
                      </div>
                      {health?.lastError && (
                        <p className="line-clamp-2 text-xs text-muted-foreground md:col-span-2">{health.lastError}</p>
                      )}
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </div>

        <div className="grid gap-2 sm:grid-cols-2">
          {canManage && <Button
            size="sm"
            onClick={onDiscover}
            disabled={discovering || !credentialReady}
            aria-label={`发现 ${displayTitle} 模型`}
          >
            {discovering ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <RefreshCw className="mr-2 h-4 w-4" />}
            {discovering ? '发现中' : '发现模型'}
          </Button>}
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
          {canManage && <Button variant="outline" size="sm" onClick={onEdit}>
            <Pencil className="mr-2 h-4 w-4" />
            编辑
          </Button>}
          {canManage && <Button variant="outline" size="sm" onClick={onToggleProvider}>
            {provider.status === 'disabled' ? <Power className="mr-2 h-4 w-4" /> : <PowerOff className="mr-2 h-4 w-4" />}
            {provider.status === 'disabled' ? '恢复' : '禁用'}
          </Button>}
          {canManage && <Button variant="outline" size="sm" className="text-destructive hover:text-destructive" onClick={onDelete}>
            <Trash2 className="mr-2 h-4 w-4" />
            删除
          </Button>}
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
                {lastTest.success ? `最近发现 ${discoveredCount ?? provider.models.length} 个模型，已合并到模型目录` : '上次发现失败'}
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
                <p className="text-sm font-medium">模型目录</p>
                <p className="text-xs text-muted-foreground">复制路由名或创建别名</p>
              </div>
              <div className="flex flex-wrap items-center justify-end gap-2">
                <Badge variant="success">{enabledModelCount} 启用</Badge>
                <Badge variant={disabledModelCount > 0 ? 'secondary' : 'outline'}>{disabledModelCount} 禁用</Badge>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!canManage || isBulkUpdating || disabledModelCount === 0}
                  onClick={() => onBulkToggleModels(true)}
                >
                  {isBulkUpdating && bulkModelMutation?.enabled ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Power className="h-3.5 w-3.5" />}
                  启用全部
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!canManage || isBulkUpdating || disableCandidateCount === 0}
                  onClick={() => onBulkToggleModels(false)}
                >
                  {isBulkUpdating && !bulkModelMutation?.enabled ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <PowerOff className="h-3.5 w-3.5" />}
                  禁用非默认
                </Button>
              </div>
            </div>

            {inventoryItems.length === 0 ? (
              <div className="px-3 py-6 text-center text-sm text-muted-foreground">
                暂无启用模型，可先发现上游模型或在配置文件中补充 models。
              </div>
            ) : (
              <div className={cn('mx-auto grid w-full max-w-6xl gap-3 p-3', inventoryGroups.length > 1 && 'xl:grid-cols-2')}>
                {inventoryGroups.map((group) => (
                  <ProviderModelGroupPanel
                    key={group.title}
                    group={group}
                    provider={provider}
                    defaultModel={provider.defaultModel}
                    compact={inventoryGroups.length > 1}
                    canManage={canManage}
                    onAlias={onAlias}
                    onCopy={onCopy}
                    onToggleModel={onToggleModel}
                    onSetDefaultModel={onSetDefaultModel}
                    bulkUpdating={isBulkUpdating}
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
  canManage,
  onCopy,
  onAlias,
  onToggleModel,
  onSetDefaultModel,
  bulkUpdating,
  modelMutationKey,
  defaultModelMutationKey,
}: {
  group: ProviderInventoryGroup
  provider: Provider
  defaultModel: string
  compact: boolean
  canManage: boolean
  onCopy: (value: string) => Promise<void>
  onAlias: (alias?: string, target?: string) => void
  onToggleModel: (model: string, enabled: boolean) => void
  onSetDefaultModel: (model: string) => void
  bulkUpdating: boolean
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
                  disabled={!canManage || modelBusy || bulkUpdating}
                  onCheckedChange={(checked) => onToggleModel(item.model, checked)}
                  aria-label={`${enabled ? '禁用' : '启用'} ${item.model}`}
                />
                <Button variant="ghost" size="icon" className="h-8 w-8 shrink-0" onClick={() => void onCopy(routeName)} aria-label={`复制 ${routeName}`}>
                  <Copy className="h-3.5 w-3.5" />
                </Button>
                {canManage && enabled && item.model !== defaultModel && (
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
                {canManage && <Button variant="outline" size="sm" className="shrink-0" disabled={!enabled} onClick={() => onAlias(item.model, routeName)}>
                  <Plus className="mr-1 h-3.5 w-3.5" />
                  别名
                </Button>}
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

function credentialHealthLabel(status: string) {
  if (status === 'cooldown') return '冷却'
  if (status === 'degraded') return '降级'
  return '健康'
}

function credentialHealthVariant(status: string): 'success' | 'warning' {
  if (status === 'cooldown' || status === 'degraded') return 'warning'
  return 'success'
}

function FormSectionHeader({ title, description }: { title: string; description: string }) {
  return (
    <div className="border-b pb-2 md:col-span-2">
      <p className="text-sm font-semibold">{title}</p>
      <p className="mt-1 text-xs text-muted-foreground">{description}</p>
    </div>
  )
}

function Field({
  label,
  htmlFor,
  className,
  description,
  error,
  required,
  children,
}: {
  label: string
  htmlFor?: string
  className?: string
  description?: string
  error?: string
  required?: boolean
  children: React.ReactNode
}) {
  return (
    <div className={cn('space-y-2', className)}>
      <Label htmlFor={htmlFor}>
        {label}
        {required && <span className="ml-1 text-destructive" aria-hidden="true">*</span>}
      </Label>
      {children}
      {error ? (
        <p className="flex items-start gap-1 text-xs text-destructive" role="alert">
          <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          {error}
        </p>
      ) : description ? (
        <p className="text-xs text-muted-foreground">{description}</p>
      ) : null}
    </div>
  )
}

function SwitchRow({
  label,
  checked,
  disabled,
  onCheckedChange,
}: {
  label: string
  checked: boolean
  disabled?: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <Label className={cn('text-sm font-normal', disabled && 'text-muted-foreground')}>{label}</Label>
      <Switch checked={checked} disabled={disabled} onCheckedChange={onCheckedChange} aria-label={label} />
    </div>
  )
}

function fidelityModeLabel(value: NonNullable<Provider['fidelityMode']>) {
  if (value === 'strict') return '严格无损'
  if (value === 'stability') return '稳定优先'
  return '尽量无损'
}

function toolStreamingArgumentsLabel(value: NonNullable<Provider['toolUse']>['streamingArguments']) {
  if (value === 'native') return 'Native'
  if (value === 'cumulative') return '累计恢复'
  if (value === 'best_effort') return 'Best effort'
  return 'Delta'
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback
}

function focusFirstInvalidDialogField() {
  window.requestAnimationFrame(() => {
    document.querySelector<HTMLElement>('[role="dialog"] [aria-invalid="true"]')?.focus()
  })
}
