import { useEffect, useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { useAuditEvents, useExportBackup, useProviders, useReloadConfig, useSettings, useTestProviderConnection } from '@/hooks'
import { useAuthStore } from '@/stores'
import { PageHeader } from '@/components/shared/PageHeader'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { ErrorState } from '@/components/shared/ErrorState'
import { EmptyState } from '@/components/shared/EmptyState'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { formatBytes, formatRelativeTime } from '@/lib/utils'
import { providerReadiness, settingsTabForCheck, type SettingsOperatorTab } from '@/features/models/operator-state'
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  CircleAlert,
  Copy,
  Database,
  Download,
  Loader2,
  Plug,
  RefreshCw,
  Route,
  Server,
  ShieldCheck,
} from 'lucide-react'
import type { AuditEvent, BackupExport, Provider, SetupCheck, SystemSettings } from '@/types'

type ProviderTestResult = {
  success: boolean
  message: string
  testedAt: string
  models?: string[]
  modelCount?: number
}

export function SettingsPage() {
  const { data: settings, isLoading, error, refetch } = useSettings()

  if (isLoading) {
    return <LoadingPage />
  }

  if (error && !settings) {
    return (
      <ErrorState
        title="运行设置加载失败"
        message={error instanceof Error ? error.message : '无法读取当前运行配置，请检查后端与登录状态。'}
        onRetry={() => void refetch()}
      />
    )
  }

  if (!settings) return <LoadingPage />

  return <SettingsForm initialSettings={settings} />
}

function SettingsForm({ initialSettings }: { initialSettings: SystemSettings }) {
  const navigate = useNavigate()
  const testConnection = useTestProviderConnection()
  const reloadConfig = useReloadConfig()
  const exportBackup = useExportBackup()
  const {
    data: providers = [],
    isLoading: providersLoading,
    error: providersError,
    refetch: refetchProviders,
  } = useProviders()
  const {
    data: auditEvents,
    isLoading: auditLoading,
    error: auditError,
    refetch: refetchAudit,
  } = useAuditEvents()
  const currentUser = useAuthStore((state) => state.currentUser)
  const canOperate = currentUser?.role === 'admin'

  const [runtimeOverride, setRuntimeOverride] = useState<{
    source: SystemSettings
    settings: SystemSettings
  } | null>(null)
  const form = runtimeOverride?.source === initialSettings ? runtimeOverride.settings : initialSettings
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null)
  const [testResults, setTestResults] = useState<Record<string, ProviderTestResult>>({})
  const [notice, setNotice] = useState<{ type: 'success' | 'error'; message: string } | null>(null)
  const [activeTab, setActiveTab] = useState<SettingsOperatorTab>('service')
  const [showReloadConfirm, setShowReloadConfirm] = useState(false)
  const [showExportConfirm, setShowExportConfirm] = useState(false)

  // Auto-dismiss notices after 5 seconds
  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(null), 5000)
    return () => clearTimeout(timer)
  }, [notice])

  const providerById = useMemo(() => new Map(providers.map((provider) => [provider.id, provider])), [providers])
  const orderedProviderIds = useMemo(() => {
    const ids = new Set(form.gateway.providerOrder)
    providers.forEach((provider) => ids.add(provider.id))
    return [...ids]
  }, [form.gateway.providerOrder, providers])

  const handleTestProvider = (providerId: string) => {
    if (!canOperate) return
    setNotice(null)
    setTestingProviderId(providerId)
    testConnection.mutate(providerId, {
      onSuccess: (result) => {
        setTestResults((current) => ({
          ...current,
          [providerId]: { ...result, testedAt: result.testedAt || new Date().toISOString() },
        }))
        setNotice({
          type: result.success ? 'success' : 'error',
          message: `${providerId}: ${result.message}`,
        })
      },
      onError: (error) => {
        setTestResults((current) => ({
          ...current,
          [providerId]: {
            success: false,
            message: error instanceof Error ? error.message : '测试失败',
            testedAt: new Date().toISOString(),
          },
        }))
        setNotice({ type: 'error', message: error instanceof Error ? error.message : '测试失败' })
      },
      onSettled: () => setTestingProviderId(null),
    })
  }

  const handleExportBackup = () => {
    if (!canOperate) return
    setNotice(null)
    exportBackup.mutate(undefined, {
      onSuccess: (backup) => {
        downloadBackup(backup)
        setShowExportConfirm(false)
        setNotice({ type: 'success', message: '控制面诊断快照已生成' })
      },
      onError: (error) => {
        setShowExportConfirm(false)
        setNotice({ type: 'error', message: error instanceof Error ? error.message : '导出失败' })
      },
    })
  }

  const handleReloadConfig = () => {
    if (!canOperate) return
    setNotice(null)
    reloadConfig.mutate(undefined, {
      onSuccess: (result) => {
        setShowReloadConfirm(false)
        setRuntimeOverride({ source: initialSettings, settings: result.settings })
        const warningCount = result.issues.filter((issue) => issue.severity === 'warning').length
        setNotice({
          type: 'success',
          message: warningCount > 0
            ? `配置已热加载，${result.providerCount} 个供应商，${warningCount} 条告警`
            : `配置已热加载，已读取 ${result.providerCount} 个 Provider 配置`,
        })
      },
      onError: (error) => {
        setShowReloadConfirm(false)
        setNotice({ type: 'error', message: error instanceof Error ? error.message : '热加载失败' })
      },
    })
  }

  const handleCopy = async (label: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value)
      setNotice({ type: 'success', message: `${label} 已复制` })
    } catch {
      setNotice({ type: 'error', message: '复制失败，请手动复制' })
    }
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="运行设置与运维"
        description="查看当前生效的部署事实、连接检查与受控运维操作"
      />

      {!canOperate && (
        <div className="flex items-start gap-3 rounded-lg border border-blue-200 bg-blue-50 p-4 text-sm text-blue-900 dark:border-blue-900 dark:bg-blue-950 dark:text-blue-100" role="status">
          <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0" />
          <div>
            <p className="font-medium">当前账号仅可查看运行事实</p>
            <p className="mt-1 text-xs opacity-80">Provider 测试、配置热加载和诊断快照会产生外部调用或审计记录，仅管理员可以执行。</p>
          </div>
        </div>
      )}

      <SettingsOverview
        settings={form}
        providers={providers}
        providersLoading={providersLoading}
        onOpenProviders={() => setActiveTab('providers')}
        onOpenOperations={() => setActiveTab('operations')}
      />

      {notice && <InlineNotice type={notice.type} message={notice.message} />}

      <SetupChecklist
        setup={form.setup}
        activeProviderCount={providers.filter((provider) => provider.status === 'active').length}
        defaultProvider={form.gateway.defaultProvider}
        defaultProviderReady={providerById.get(form.gateway.defaultProvider)?.status === 'active'}
        authEnabled={form.auth.enabled}
        onNavigate={(checkId) => {
          if (checkId === 'admin') {
            navigate('/users')
            return
          }
          setActiveTab(settingsTabForCheck(checkId))
        }}
      />

      <Tabs value={activeTab} onValueChange={(value) => setActiveTab(value as SettingsOperatorTab)}>
        <div className="overflow-x-auto pb-1">
          <TabsList className="h-auto min-w-max justify-start">
            <TabsTrigger value="service">服务运行</TabsTrigger>
            <TabsTrigger value="security">认证安全</TabsTrigger>
            <TabsTrigger value="limits">请求边界</TabsTrigger>
            <TabsTrigger value="providers">Provider 检查</TabsTrigger>
            <TabsTrigger value="operations">运维审计</TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="service">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2"><Server className="h-4 w-4" />服务运行事实</CardTitle>
              <CardDescription>这些值来自当前进程，不是可编辑表单；部署配置修改后需要重启。</CardDescription>
            </CardHeader>
            <CardContent className="grid gap-3 md:grid-cols-3">
              <RuntimeFact label="绑定地址" value={form.server.bindAddress} hint="MODELPORT_BIND" mono />
              <RuntimeFact label="最大请求体" value={formatBytes(form.server.maxRequestBodyBytes)} hint="MODELPORT_MAX_REQUEST_BODY_BYTES" />
              <RuntimeFact label="并发请求" value={String(form.server.maxConcurrentRequests)} hint="MODELPORT_MAX_CONCURRENT_REQUESTS" />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="security">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2"><ShieldCheck className="h-4 w-4" />认证安全边界</CardTitle>
              <CardDescription>展示数据面认证状态；Dashboard session、CSRF 与 Origin 保护由后端独立执行。</CardDescription>
            </CardHeader>
            <CardContent className="grid gap-3 md:grid-cols-3">
              <RuntimeFact
                label="数据面认证"
                value={form.auth.enabled ? '已启用' : '未启用'}
                hint="/v1 与受保护诊断端点"
                status={form.auth.enabled ? 'ok' : 'error'}
              />
              <RuntimeFact
                label="匿名访问"
                value={form.auth.allowNoAuth ? '允许' : '拒绝'}
                hint={form.auth.allowNoAuth ? '仅适合隔离的本机开发环境' : '未携带有效凭证的请求会被拒绝'}
                status={form.auth.allowNoAuth ? 'error' : 'ok'}
              />
              <RuntimeFact label="Legacy Token 变量" value={form.auth.tokenEnvVar} hint="只显示变量名，不暴露值" mono />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="limits">
          <Card>
            <CardHeader>
              <CardTitle>请求与流边界</CardTitle>
              <CardDescription>并发、请求体和超时是启动参数；此处不表示 API Key 的 USD 周期预算。</CardDescription>
            </CardHeader>
            <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
              <RuntimeFact label="并发请求上限" value={String(form.rateLimits.maxConcurrentRequests)} hint="服务级并发层" />
              <RuntimeFact label="请求体上限" value={formatBytes(form.rateLimits.maxRequestBodyBytes)} hint="超过时返回 413" />
              <RuntimeFact label="非流式 / 握手超时" value={`${form.rateLimits.requestTimeoutSecs} 秒`} hint="SSE 建立后不作为总时限" />
              <RuntimeFact label="SSE 空闲超时" value={`${form.rateLimits.streamIdleTimeoutSecs} 秒`} hint="每个上游数据块会重置" />
            </CardContent>
          </Card>
        </TabsContent>

        {/* Providers Tab */}
        <TabsContent value="providers">
          <Card>
            <CardHeader className="flex-col items-start justify-between gap-3 space-y-0 sm:flex-row">
              <div>
                <CardTitle>Provider 连接检查</CardTitle>
                <CardDescription>这里验证当前运行凭证与模型发现；凭证变量、账号池和模型目录在 Provider 页面管理。</CardDescription>
              </div>
              <Button asChild variant="outline" size="sm" className="w-full shrink-0 sm:w-auto">
                <Link to="/models"><Route className="mr-2 h-4 w-4" />Provider 管理</Link>
              </Button>
            </CardHeader>
            <CardContent>
              {providersLoading ? (
                <div className="flex items-center justify-center py-12 text-sm text-muted-foreground"><Loader2 className="mr-2 h-4 w-4 animate-spin" />加载 Provider…</div>
              ) : providersError && providers.length === 0 ? (
                <ErrorState
                  title="Provider 状态加载失败"
                  message={providersError instanceof Error ? providersError.message : '无法读取 Provider 目录。'}
                  onRetry={() => void refetchProviders()}
                />
              ) : orderedProviderIds.length === 0 ? (
                <EmptyState
                  icon={Plug}
                  title="暂无 Provider"
                  description="请先在 Provider 与模型页面添加上游接入。"
                  action={<Button asChild size="sm"><Link to="/models">前往 Provider 管理</Link></Button>}
                />
              ) : (
                <div className="space-y-3">
                  {orderedProviderIds.map((providerId) => {
                    const provider = providerById.get(providerId)
                    return (
                      <ProviderCredentialRow
                        key={providerId}
                        providerId={providerId}
                        displayName={provider?.displayName || providerId}
                        apiKeyEnv={provider?.apiKeyEnv || `${providerId.toUpperCase()}_API_KEY`}
                        isDefault={providerId === form.gateway.defaultProvider}
                        status={provider?.status || 'inactive'}
                        readiness={provider ? providerReadiness(provider, providerId === form.gateway.defaultProvider) : null}
                        lastTest={testResults[providerId] || provider?.lastTest || null}
                        isTesting={testingProviderId === providerId}
                        isDisabled={!canOperate || !provider || testConnection.isPending}
                        baseUrl={provider?.baseUrl || ''}
                        hasApiKey={provider?.hasApiKey || false}
                        apiKeyRequired={provider?.apiKeyRequired ?? true}
                        modelCount={provider?.models.length || 0}
                        onTest={() => handleTestProvider(providerId)}
                      />
                    )
                  })}
                </div>
              )}
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="operations">
          <div className="grid gap-4 lg:grid-cols-3">
            <RuntimeCard runtime={form.runtime} onCopy={handleCopy} />
            <ConfigReloadCard
              isPending={reloadConfig.isPending}
              isDisabled={!canOperate}
              warningCount={form.setup?.issues.filter((issue) => issue.severity === 'warning').length ?? 0}
              providerCount={orderedProviderIds.length}
              defaultProvider={form.gateway.defaultProvider}
              onReload={() => setShowReloadConfirm(true)}
            />
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Database className="h-4 w-4" />
                  诊断快照
                </CardTitle>
                <CardDescription>导出脱敏控制面数据；不可用于恢复</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="rounded-md border bg-muted/40 p-3 text-sm">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">敏感密钥</span>
                    <Badge variant="outline">不包含明文</Badge>
                  </div>
                  <div className="mt-2 flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">用户/用量数据</span>
                    <Badge variant="secondary">包含</Badge>
                  </div>
                </div>
                <Button
                  onClick={() => setShowExportConfirm(true)}
                  disabled={exportBackup.isPending || !canOperate}
                  className="w-full"
                >
                  {exportBackup.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <Download className="mr-2 h-4 w-4" />}
                  导出诊断快照
                </Button>
                {!canOperate && <p className="text-xs text-muted-foreground">需要管理员权限。</p>}
              </CardContent>
            </Card>
          </div>

          <Card className="mt-4">
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Activity className="h-4 w-4" />
                审计事件
              </CardTitle>
              <CardDescription>最近 {auditEvents?.events.length ?? 0} / {auditEvents?.total ?? 0} 条</CardDescription>
            </CardHeader>
            <CardContent>
              {auditLoading ? (
                <div className="flex items-center justify-center py-10 text-sm text-muted-foreground"><Loader2 className="mr-2 h-4 w-4 animate-spin" />加载审计事件…</div>
              ) : auditError ? (
                <ErrorState
                  title="审计事件加载失败"
                  message={auditError instanceof Error ? auditError.message : '无法读取审计记录。'}
                  onRetry={() => void refetchAudit()}
                />
              ) : (
                <AuditList events={auditEvents?.events ?? []} />
              )}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      <OperationConfirmDialog
        open={showReloadConfirm}
        onOpenChange={setShowReloadConfirm}
        title="确认热加载运行配置"
        description="此操作会立即重新读取 Provider、基础凭证引用、Base URL、模型目录和别名，并写入审计记录；需要重启的服务参数不会改变。"
        confirmLabel="确认热加载"
        isPending={reloadConfig.isPending}
        onConfirm={handleReloadConfig}
      />

      <OperationConfirmDialog
        open={showExportConfirm}
        onOpenChange={setShowExportConfirm}
        title="导出诊断快照"
        description="快照不包含明文密钥，但会包含用户、用量和控制面配置。请仅保存到受控位置，并按敏感运维数据处理。"
        confirmLabel="确认导出"
        isPending={exportBackup.isPending}
        onConfirm={handleExportBackup}
      />
    </div>
  )
}

function SettingsOverview({
  settings,
  providers,
  providersLoading,
  onOpenProviders,
  onOpenOperations,
}: {
  settings: SystemSettings
  providers: Provider[]
  providersLoading: boolean
  onOpenProviders: () => void
  onOpenOperations: () => void
}) {
  const defaultProvider = providers.find((provider) => provider.id === settings.gateway.defaultProvider)
  const readiness = defaultProvider ? providerReadiness(defaultProvider, true) : null
  const ready = settings.setup?.ready ?? Boolean(defaultProvider && settings.auth.enabled)
  const activeProviders = providers.filter((provider) => provider.status === 'active').length

  return (
    <Card className="border-primary/20">
      <CardContent className="grid gap-4 p-5 md:grid-cols-2 xl:grid-cols-[1.2fr_repeat(3,minmax(0,1fr))_auto] xl:items-center">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">运行状态</p>
            <Badge variant={ready ? 'success' : 'destructive'}>{ready ? '基础检查通过' : '需要处理'}</Badge>
          </div>
          <p className="mt-2 truncate text-lg font-semibold">{settings.runtime?.apiEndpoint || settings.server.bindAddress}</p>
          <p className="mt-1 text-xs text-muted-foreground">此页反映当前进程，不会直接改写 .env 或 config.toml。</p>
        </div>
        <OverviewFact label="默认 Provider" value={settings.gateway.defaultProvider || '未配置'} detail={readiness?.label || '未找到'} />
        <OverviewFact
          label="Provider"
          value={providersLoading ? '加载中…' : `${activeProviders} / ${providers.length}`}
          detail={providersLoading ? '正在读取运行目录' : '启用 / 总数'}
        />
        <OverviewFact label="API 认证" value={settings.auth.enabled ? '已启用' : '未启用'} detail={settings.auth.allowNoAuth ? '允许匿名访问' : '拒绝匿名访问'} />
        <div className="flex flex-wrap gap-2 xl:max-w-[180px] xl:justify-end">
          <Button variant="outline" size="sm" onClick={onOpenProviders}>检查 Provider</Button>
          <Button variant="outline" size="sm" onClick={onOpenOperations}>查看运维</Button>
        </div>
      </CardContent>
    </Card>
  )
}

function OverviewFact({ label, value, detail }: { label: string; value: string; detail: string }) {
  return (
    <div className="rounded-md border bg-muted/20 p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="mt-1 truncate text-sm font-semibold">{value}</p>
      <p className="mt-1 truncate text-xs text-muted-foreground">{detail}</p>
    </div>
  )
}

function RuntimeFact({
  label,
  value,
  hint,
  mono,
  status,
}: {
  label: string
  value: string
  hint: string
  mono?: boolean
  status?: 'ok' | 'error'
}) {
  return (
    <div className="rounded-lg border bg-muted/20 p-4">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-muted-foreground">{label}</p>
        {status && <Badge variant={status === 'ok' ? 'success' : 'destructive'}>{status === 'ok' ? '安全' : '注意'}</Badge>}
      </div>
      <p className={mono ? 'mt-2 break-all font-mono text-sm font-semibold' : 'mt-2 text-lg font-semibold'}>{value}</p>
      <p className="mt-2 break-words text-xs text-muted-foreground">{hint}</p>
    </div>
  )
}

function SetupChecklist({
  setup,
  activeProviderCount,
  defaultProvider,
  defaultProviderReady,
  authEnabled,
  onNavigate,
}: {
  setup: SystemSettings['setup']
  activeProviderCount: number
  defaultProvider: string
  defaultProviderReady: boolean
  authEnabled: boolean
  onNavigate: (checkId: string) => void
}) {
  const checks = setup?.checks ?? fallbackSetupChecks({ activeProviderCount, defaultProvider, defaultProviderReady, authEnabled })
  const errorCount = checks.filter((check) => check.status === 'error').length
  const issues = setup?.issues ?? []
  const warningCount = checks.filter((check) => check.status === 'warning').length
    + issues.filter((issue) => issue.severity === 'warning').length
  const ready = setup?.ready ?? errorCount === 0

  return (
    <Card>
      <CardHeader className="flex-row flex-wrap items-center justify-between gap-3 space-y-0">
        <div>
          <CardTitle className="flex items-center gap-2">
            <ShieldCheck className="h-4 w-4" />
            上线检查
          </CardTitle>
          <CardDescription>{ready ? '基础配置检查已通过；实际调用仍以连接测试和请求日志为准' : `${errorCount} 项需要处理`}</CardDescription>
        </div>
        <Badge variant={ready ? 'success' : 'destructive'}>
          {ready ? '可用' : '需处理'}
        </Badge>
      </CardHeader>
      <CardContent>
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {checks.map((check) => (
            <button
              key={check.id}
              type="button"
              className="flex min-h-20 items-start gap-3 rounded-lg border p-3 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => onNavigate(check.id)}
              aria-label={`查看 ${check.label} 详情：${check.detail}`}
            >
              <div className="mt-0.5">{statusIcon(check.status)}</div>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">{check.label}</p>
                <p className="mt-1 text-xs text-muted-foreground">{check.detail}</p>
              </div>
              <span className="text-xs text-primary">查看</span>
            </button>
          ))}
        </div>
        {issues.length > 0 && (
          <div className="mt-4 space-y-2" aria-label="配置校验问题">
            {issues.map((issue, index) => (
              <div
                key={`${issue.severity}:${issue.message}:${index}`}
                className={issue.severity === 'error'
                  ? 'flex items-start gap-2 rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200'
                  : 'flex items-start gap-2 rounded-md border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200'}
              >
                {issue.severity === 'error' ? <CircleAlert className="h-4 w-4 shrink-0" /> : <AlertTriangle className="h-4 w-4 shrink-0" />}
                <span>{issue.message}</span>
              </div>
            ))}
          </div>
        )}
        {warningCount > 0 && <p className="mt-3 text-xs text-amber-700 dark:text-amber-300">共 {warningCount} 条警告；运行可用不代表每个 Provider 都已验收。</p>}
      </CardContent>
    </Card>
  )
}

function RuntimeCard({
  runtime,
  onCopy,
}: {
  runtime?: SystemSettings['runtime']
  onCopy: (label: string, value: string) => void
}) {
  const rows = [
    ['Anthropic', runtime?.anthropicEndpoint || runtime?.apiEndpoint, '后端未上报'],
    ['OpenAI', runtime?.openaiEndpoint, '后端未上报'],
    ['Models', runtime?.modelsEndpoint, '后端未上报'],
    ['Admin', runtime?.adminEndpoint, '后端未上报'],
    ['Control', runtime?.controlDataPath, runtime ? '未配置' : '后端未上报'],
    ['Auth', runtime?.authDataPath, runtime ? '未配置' : '后端未上报'],
  ] as const

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Database className="h-4 w-4" />
          运行信息
        </CardTitle>
        <CardDescription>端点和本地数据文件</CardDescription>
      </CardHeader>
      <CardContent className="space-y-2">
        {rows.map(([label, value, fallback]) => (
          <div key={label} className="flex items-center gap-2 rounded-md border px-3 py-2">
            <span className="w-16 shrink-0 text-xs font-medium text-muted-foreground">{label}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-xs">{value || fallback}</span>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => value && onCopy(label, value)}
              disabled={!value}
              aria-label={`复制 ${label}`}
              title={value ? `复制 ${label}` : `${label} ${fallback}`}
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
          </div>
        ))}
      </CardContent>
    </Card>
  )
}

function ConfigReloadCard({
  isPending,
  isDisabled,
  warningCount,
  providerCount,
  defaultProvider,
  onReload,
}: {
  isPending: boolean
  isDisabled: boolean
  warningCount: number
  providerCount: number
  defaultProvider: string
  onReload: () => void
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <RefreshCw className="h-4 w-4" />
          配置热加载
        </CardTitle>
        <CardDescription>重新读取 provider、基础密钥、Base URL、模型与别名</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2 rounded-md border bg-muted/40 p-3 text-sm">
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">供应商</span>
            <Badge variant="secondary">{providerCount}</Badge>
          </div>
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">默认路由</span>
            <span className="max-w-40 truncate font-mono text-xs">{defaultProvider || '未配置'}</span>
          </div>
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">校验告警</span>
            <Badge variant={warningCount > 0 ? 'outline' : 'success'}>{warningCount}</Badge>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          监听端口、限流/并发层、请求体上限、HTTP 超时、安全/会话、存储、可信代理与新账号环境变量仍需重启。
        </p>
        <Button
          onClick={onReload}
          disabled={isPending || isDisabled}
          className="w-full"
          variant="outline"
        >
          {isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <RefreshCw className="mr-2 h-4 w-4" />}
          热重载配置
        </Button>
        {isDisabled && <p className="text-xs text-muted-foreground">需要管理员权限；热加载会写入审计记录。</p>}
      </CardContent>
    </Card>
  )
}

function AuditList({ events }: { events: AuditEvent[] }) {
  if (events.length === 0) {
    return <div className="rounded-md border py-10 text-center text-sm text-muted-foreground">暂无审计事件</div>
  }

  return (
    <div className="divide-y rounded-md border">
      {events.map((event) => (
        <div key={event.id} className="flex flex-wrap items-start justify-between gap-3 p-3">
          <div className="flex min-w-0 flex-1 items-start gap-3">
            <div className="mt-0.5">{statusIcon(event.severity)}</div>
            <div className="min-w-0">
              <p className="text-sm">{event.message}</p>
              <p className="mt-1 truncate text-xs text-muted-foreground">
                {[event.actor, event.target].filter(Boolean).join(' · ') || event.type}
              </p>
            </div>
          </div>
          <span className="text-xs text-muted-foreground">{formatRelativeTime(event.timestamp)}</span>
        </div>
      ))}
    </div>
  )
}

function OperationConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  isPending,
  onConfirm,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: string
  confirmLabel: string
  isPending: boolean
  onConfirm: () => void
}) {
  return (
    <Dialog open={open} onOpenChange={(nextOpen) => { if (!isPending) onOpenChange(nextOpen) }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>取消</Button>
          <Button onClick={onConfirm} disabled={isPending}>
            {isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function InlineNotice({ type, message }: { type: 'success' | 'error'; message: string }) {
  const Icon = type === 'success' ? CheckCircle2 : CircleAlert
  return (
    <div
      role={type === 'error' ? 'alert' : 'status'}
      aria-live={type === 'error' ? 'assertive' : 'polite'}
      className={type === 'success'
        ? 'flex items-start gap-2 rounded-lg border border-green-200 bg-green-50 p-3 text-sm text-green-800 dark:border-green-900 dark:bg-green-950 dark:text-green-200'
        : 'flex items-start gap-2 rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200'}
    >
      <Icon className="mt-0.5 h-4 w-4 shrink-0" />
      <span>{message}</span>
    </div>
  )
}

function ProviderCredentialRow({
  providerId,
  displayName,
  apiKeyEnv,
  isDefault,
  status,
  lastTest,
  isTesting,
  isDisabled,
  baseUrl,
  hasApiKey,
  apiKeyRequired,
  modelCount,
  readiness,
  onTest,
}: {
  providerId: string
  displayName: string
  apiKeyEnv: string
  isDefault: boolean
  status: string
  lastTest: ProviderTestResult | null
  isTesting: boolean
  isDisabled: boolean
  baseUrl: string
  hasApiKey: boolean
  apiKeyRequired: boolean
  modelCount: number
  readiness: ReturnType<typeof providerReadiness> | null
  onTest: () => void
}) {
  const displayStatus = lastTest ? (lastTest.success ? 'success' : 'error') : status
  const credentialLabel = !apiKeyRequired ? '无需凭证' : hasApiKey ? '凭证已配置' : '缺少凭证'
  const credentialVariant = !apiKeyRequired || hasApiKey ? 'secondary' : 'destructive'

  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border p-4">
      <div className="min-w-0 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm font-medium">{displayName}</p>
          {isDefault && <Badge variant="outline">默认</Badge>}
          <Badge variant={credentialVariant}>{credentialLabel}</Badge>
        </div>
        <p className="font-mono text-xs text-muted-foreground">{apiKeyEnv}</p>
        <p className="max-w-[560px] truncate text-xs text-muted-foreground">{baseUrl}</p>
        {lastTest && (
          <p className={lastTest.success ? 'text-xs text-green-700 dark:text-green-300' : 'text-xs text-red-700 dark:text-red-300'}>
            {lastTest.message} · {formatTestTime(lastTest.testedAt)}
          </p>
        )}
        {!lastTest && readiness && (
          <p className={readiness.level === 'ready' ? 'text-xs text-green-700 dark:text-green-300' : 'text-xs text-amber-700 dark:text-amber-300'}>
            {readiness.label} · {readiness.nextStep}
          </p>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <span className="hidden text-xs text-muted-foreground sm:inline">{modelCount} models</span>
        <StatusBadge status={displayStatus} />
        <Button
          variant="outline"
          size="sm"
          onClick={onTest}
          disabled={isDisabled}
          aria-label={`测试 ${providerId} 连接`}
          title={isDisabled && !isTesting ? '需要管理员权限、有效 Provider，或等待当前测试完成' : undefined}
        >
          {isTesting ? (
            <Loader2 className="mr-1 h-3 w-3 animate-spin" />
          ) : (
            <Plug className="mr-1 h-3 w-3" />
          )}
          测试连接
        </Button>
      </div>
    </div>
  )
}

function formatTestTime(value: string) {
  const timestamp = Number(value)
  const date = Number.isFinite(timestamp) ? new Date(timestamp) : new Date(value)
  if (Number.isNaN(date.getTime())) return '刚刚'
  return date.toLocaleString()
}

function fallbackSetupChecks({
  activeProviderCount,
  defaultProvider,
  defaultProviderReady,
  authEnabled,
}: {
  activeProviderCount: number
  defaultProvider: string
  defaultProviderReady: boolean
  authEnabled: boolean
}): SetupCheck[] {
  return [
    {
      id: 'auth',
      label: 'API 认证',
      status: authEnabled ? 'ok' : 'error',
      detail: authEnabled ? '已启用请求认证' : '未配置认证令牌',
    },
    {
      id: 'providers',
      label: '供应商凭证',
      status: activeProviderCount > 0 ? 'ok' : 'error',
      detail: activeProviderCount > 0 ? `${activeProviderCount} 个供应商可用` : '没有可用供应商',
    },
    {
      id: 'defaultProvider',
      label: '默认供应商',
      status: defaultProviderReady ? 'ok' : 'error',
      detail: defaultProviderReady ? `${defaultProvider} 可用` : `${defaultProvider} 不可用`,
    },
  ]
}

function statusIcon(status: SetupCheck['status'] | AuditEvent['severity']) {
  if (status === 'ok' || status === 'info') return <CheckCircle2 className="h-4 w-4 text-green-600" />
  if (status === 'warning') return <AlertTriangle className="h-4 w-4 text-yellow-600" />
  return <CircleAlert className="h-4 w-4 text-red-600" />
}

function downloadBackup(backup: BackupExport) {
  const generatedAt = Number(backup.generatedAt)
  const date = Number.isFinite(generatedAt) ? new Date(generatedAt) : new Date()
  const stamp = date.toISOString().replace(/[:.]/g, '-')
  const blob = new Blob([JSON.stringify(backup, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `modelport-diagnostic-snapshot-${stamp}.json`
  anchor.click()
  URL.revokeObjectURL(url)
}
