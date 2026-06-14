import { useEffect, useMemo, useState } from 'react'
import { useAuditEvents, useExportBackup, useProviders, useReloadConfig, useSettings, useUpdateSettings, useTestProviderConnection } from '@/hooks'
import { useAuthStore } from '@/stores'
import { PageHeader } from '@/components/shared/PageHeader'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Switch } from '@/components/ui/switch'
import { Separator } from '@/components/ui/separator'
import { formatBytes, formatRelativeTime } from '@/lib/utils'
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
  Save,
  ShieldCheck,
} from 'lucide-react'
import type { AuditEvent, BackupExport, SetupCheck, SystemSettings } from '@/types'

type ProviderTestResult = {
  success: boolean
  message: string
  testedAt: string
  models?: string[]
  modelCount?: number
}

export function SettingsPage() {
  const { data: settings, isLoading } = useSettings()

  if (isLoading || !settings) {
    return <LoadingPage />
  }

  return <SettingsForm initialSettings={settings} />
}

function SettingsForm({ initialSettings }: { initialSettings: SystemSettings }) {
  const updateSettings = useUpdateSettings()
  const testConnection = useTestProviderConnection()
  const reloadConfig = useReloadConfig()
  const exportBackup = useExportBackup()
  const { data: providers = [] } = useProviders()
  const { data: auditEvents } = useAuditEvents()
  const currentUser = useAuthStore((state) => state.currentUser)

  const [form, setForm] = useState<SystemSettings>(initialSettings)
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null)
  const [testResults, setTestResults] = useState<Record<string, ProviderTestResult>>({})
  const [notice, setNotice] = useState<{ type: 'success' | 'error'; message: string } | null>(null)

  // Auto-dismiss notices after 5 seconds
  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(null), 5000)
    return () => clearTimeout(timer)
  }, [notice])

  const providerById = useMemo(() => new Map(providers.map((provider) => [provider.id, provider])), [providers])

  const handleSave = () => {
    setNotice(null)
    updateSettings.mutate(form, {
      onSuccess: (settings) => {
        setForm(settings)
        setNotice({ type: 'success', message: '系统设置已更新' })
      },
      onError: (error) => {
        setNotice({ type: 'error', message: error instanceof Error ? error.message : '保存失败' })
      },
    })
  }

  const handleTestProvider = (providerId: string) => {
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
    setNotice(null)
    exportBackup.mutate(undefined, {
      onSuccess: (backup) => {
        downloadBackup(backup)
        setNotice({ type: 'success', message: '控制面备份已生成' })
      },
      onError: (error) => {
        setNotice({ type: 'error', message: error instanceof Error ? error.message : '导出失败' })
      },
    })
  }

  const handleReloadConfig = () => {
    setNotice(null)
    reloadConfig.mutate(undefined, {
      onSuccess: (result) => {
        setForm(result.settings)
        const warningCount = result.issues.filter((issue) => issue.severity === 'warning').length
        setNotice({
          type: 'success',
          message: warningCount > 0
            ? `配置已热加载，${result.providerCount} 个供应商，${warningCount} 条告警`
            : `配置已热加载，${result.providerCount} 个供应商可路由`,
        })
      },
      onError: (error) => {
        setNotice({ type: 'error', message: error instanceof Error ? error.message : '热加载失败' })
      },
    })
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="系统设置"
        description="配置网关运行参数"
        action={{ label: '保存更改', onClick: handleSave, icon: Save }}
      />

      {notice && <InlineNotice type={notice.type} message={notice.message} />}

      <SetupChecklist
        setup={form.setup}
        activeProviderCount={providers.filter((provider) => provider.status === 'active').length}
        defaultProvider={form.gateway.defaultProvider}
        defaultProviderReady={providerById.get(form.gateway.defaultProvider)?.status === 'active'}
        authEnabled={form.auth.enabled}
      />

      <Tabs defaultValue="general">
        <TabsList>
          <TabsTrigger value="general">通用</TabsTrigger>
          <TabsTrigger value="auth">认证</TabsTrigger>
          <TabsTrigger value="ratelimits">限流</TabsTrigger>
          <TabsTrigger value="providers">提供商凭证</TabsTrigger>
          <TabsTrigger value="operations">运维</TabsTrigger>
        </TabsList>

        {/* General Tab */}
        <TabsContent value="general">
          <Card>
            <CardHeader>
              <CardTitle>通用设置</CardTitle>
              <CardDescription>服务器基础配置</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <Label>绑定地址</Label>
                <Input
                  value={form.server.bindAddress}
                  onChange={(e) => setForm({ ...form, server: { ...form.server, bindAddress: e.target.value } })}
                />
                <p className="text-xs text-muted-foreground">格式: host:port，例如 127.0.0.1:17878</p>
              </div>
              <Separator />
              <div className="space-y-2">
                <Label>最大请求体大小</Label>
                <Input
                  type="number"
                  value={form.server.maxRequestBodyBytes}
                  onChange={(e) => setForm({ ...form, server: { ...form.server, maxRequestBodyBytes: Number(e.target.value) } })}
                />
                <p className="text-xs text-muted-foreground">当前值: {formatBytes(form.server.maxRequestBodyBytes)}</p>
              </div>
              <Separator />
              <div className="space-y-2">
                <Label>最大并发请求数</Label>
                <Input
                  type="number"
                  value={form.server.maxConcurrentRequests}
                  onChange={(e) => setForm({ ...form, server: { ...form.server, maxConcurrentRequests: Number(e.target.value) } })}
                />
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Auth Tab */}
        <TabsContent value="auth">
          <Card>
            <CardHeader>
              <CardTitle>认证设置</CardTitle>
              <CardDescription>配置 API 认证方式</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6">
              <div className="flex items-center justify-between">
                <div className="space-y-0.5">
                  <Label>启用认证</Label>
                  <p className="text-xs text-muted-foreground">要求所有 API 请求携带认证令牌</p>
                </div>
                <Switch
                  checked={form.auth.enabled}
                  onCheckedChange={(checked) => setForm({ ...form, auth: { ...form.auth, enabled: checked } })}
                />
              </div>
              <Separator />
              <div className="flex items-center justify-between">
                <div className="space-y-0.5">
                  <Label>允许无认证访问</Label>
                  <p className="text-xs text-muted-foreground">允许未携带令牌的请求通过（不推荐）</p>
                </div>
                <Switch
                  checked={form.auth.allowNoAuth}
                  onCheckedChange={(checked) => setForm({ ...form, auth: { ...form.auth, allowNoAuth: checked } })}
                />
              </div>
              <Separator />
              <div className="space-y-2">
                <Label>Token 环境变量</Label>
                <Input value={form.auth.tokenEnvVar} readOnly className="bg-muted" />
                <p className="text-xs text-muted-foreground">从此环境变量读取认证令牌</p>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Rate Limits Tab */}
        <TabsContent value="ratelimits">
          <Card>
            <CardHeader>
              <CardTitle>限流设置</CardTitle>
              <CardDescription>配置请求限流和超时参数</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-4 md:grid-cols-2">
                <div className="space-y-2">
                  <Label>最大并发请求数</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.maxConcurrentRequests}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, maxConcurrentRequests: Number(e.target.value) } })}
                  />
                </div>
                <div className="space-y-2">
                  <Label>最大请求体大小（字节）</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.maxRequestBodyBytes}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, maxRequestBodyBytes: Number(e.target.value) } })}
                  />
                  <p className="text-xs text-muted-foreground">{formatBytes(form.rateLimits.maxRequestBodyBytes)}</p>
                </div>
                <div className="space-y-2">
                  <Label>请求超时（秒）</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.requestTimeoutSecs}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, requestTimeoutSecs: Number(e.target.value) } })}
                  />
                </div>
                <div className="space-y-2">
                  <Label>流空闲超时（秒）</Label>
                  <Input
                    type="number"
                    value={form.rateLimits.streamIdleTimeoutSecs}
                    onChange={(e) => setForm({ ...form, rateLimits: { ...form.rateLimits, streamIdleTimeoutSecs: Number(e.target.value) } })}
                  />
                </div>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        {/* Providers Tab */}
        <TabsContent value="providers">
          <Card>
            <CardHeader>
              <CardTitle>提供商凭证</CardTitle>
              <CardDescription>管理各提供商的 API Key 配置状态</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-3">
                {form.gateway.providerOrder.map((providerId) => (
                  <ProviderCredentialRow
                    key={providerId}
                    providerId={providerId}
                    displayName={providerById.get(providerId)?.displayName || providerId}
                    apiKeyEnv={providerById.get(providerId)?.apiKeyEnv || `${providerId.toUpperCase()}_API_KEY`}
                    isDefault={providerId === form.gateway.defaultProvider}
                    status={providerById.get(providerId)?.status || 'inactive'}
                    lastTest={testResults[providerId] || providerById.get(providerId)?.lastTest || null}
                    isTesting={testingProviderId === providerId}
                    isDisabled={testConnection.isPending}
                    baseUrl={providerById.get(providerId)?.baseUrl || ''}
                    hasApiKey={providerById.get(providerId)?.hasApiKey || false}
                    apiKeyRequired={providerById.get(providerId)?.apiKeyRequired ?? true}
                    modelCount={providerById.get(providerId)?.models.length || 0}
                    onTest={() => handleTestProvider(providerId)}
                  />
                ))}
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="operations">
          <div className="grid gap-4 lg:grid-cols-3">
            <RuntimeCard runtime={form.runtime} />
            <ConfigReloadCard
              isPending={reloadConfig.isPending}
              isDisabled={currentUser?.role !== 'admin'}
              warningCount={form.setup?.issues.filter((issue) => issue.severity === 'warning').length ?? 0}
              providerCount={form.gateway.providerOrder.length}
              defaultProvider={form.gateway.defaultProvider}
              onReload={handleReloadConfig}
            />
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Database className="h-4 w-4" />
                  数据备份
                </CardTitle>
                <CardDescription>导出控制面快照</CardDescription>
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
                  onClick={handleExportBackup}
                  disabled={exportBackup.isPending || currentUser?.role !== 'admin'}
                  className="w-full"
                >
                  {exportBackup.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <Download className="mr-2 h-4 w-4" />}
                  导出备份
                </Button>
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
              <AuditList events={auditEvents?.events ?? []} />
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  )
}

function SetupChecklist({
  setup,
  activeProviderCount,
  defaultProvider,
  defaultProviderReady,
  authEnabled,
}: {
  setup: SystemSettings['setup']
  activeProviderCount: number
  defaultProvider: string
  defaultProviderReady: boolean
  authEnabled: boolean
}) {
  const checks = setup?.checks ?? fallbackSetupChecks({ activeProviderCount, defaultProvider, defaultProviderReady, authEnabled })
  const errorCount = checks.filter((check) => check.status === 'error').length
  const warningCount = checks.filter((check) => check.status === 'warning').length
  const ready = setup?.ready ?? errorCount === 0

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <div>
          <CardTitle className="flex items-center gap-2">
            <ShieldCheck className="h-4 w-4" />
            上线检查
          </CardTitle>
          <CardDescription>{ready ? '核心链路已具备可用条件' : `${errorCount} 项需要处理`}</CardDescription>
        </div>
        <Badge variant={ready ? 'success' : 'destructive'}>
          {ready ? '可用' : '需处理'}
        </Badge>
      </CardHeader>
      <CardContent>
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {checks.map((check) => (
            <div key={check.id} className="flex min-h-20 items-start gap-3 rounded-lg border p-3">
              <div className="mt-0.5">{statusIcon(check.status)}</div>
              <div className="min-w-0">
                <p className="text-sm font-medium">{check.label}</p>
                <p className="mt-1 text-xs text-muted-foreground">{check.detail}</p>
              </div>
            </div>
          ))}
        </div>
        {warningCount > 0 && (
          <p className="mt-3 text-xs text-yellow-700 dark:text-yellow-300">{warningCount} 项配置存在告警</p>
        )}
      </CardContent>
    </Card>
  )
}

function RuntimeCard({ runtime }: { runtime?: SystemSettings['runtime'] }) {
  const rows = [
    ['API', runtime?.apiEndpoint || 'http://127.0.0.1:17878/v1/messages'],
    ['Models', runtime?.modelsEndpoint || 'http://127.0.0.1:17878/v1/models'],
    ['Admin', runtime?.adminEndpoint || 'http://127.0.0.1:17878/admin'],
    ['Control', runtime?.controlDataPath || '未配置'],
    ['Auth', runtime?.authDataPath || '未配置'],
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
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-center gap-2 rounded-md border px-3 py-2">
            <span className="w-16 shrink-0 text-xs font-medium text-muted-foreground">{label}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-xs">{value}</span>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => copyText(value)}
              aria-label={`复制 ${label}`}
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
        <CardDescription>重新读取 provider、密钥、Base URL 与模型列表</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2 rounded-md border bg-muted/40 p-3 text-sm">
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">供应商</span>
            <Badge variant="secondary">{providerCount}</Badge>
          </div>
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">默认路由</span>
            <span className="max-w-40 truncate font-mono text-xs">{defaultProvider}</span>
          </div>
          <div className="flex items-center justify-between gap-3">
            <span className="text-muted-foreground">校验告警</span>
            <Badge variant={warningCount > 0 ? 'outline' : 'success'}>{warningCount}</Badge>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          监听端口、并发层、请求体上限、HTTP 超时和可信代理仍需重启后端。
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

function InlineNotice({ type, message }: { type: 'success' | 'error'; message: string }) {
  const Icon = type === 'success' ? CheckCircle2 : CircleAlert
  return (
    <div className={type === 'success'
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
      </div>
      <div className="flex items-center gap-3">
        <span className="hidden text-xs text-muted-foreground sm:inline">{modelCount} models</span>
        <StatusBadge status={displayStatus} />
        <Button
          variant="outline"
          size="sm"
          onClick={onTest}
          disabled={isDisabled}
          aria-label={`测试 ${providerId} 连接`}
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

async function copyText(text: string) {
  await navigator.clipboard.writeText(text)
}

function downloadBackup(backup: BackupExport) {
  const generatedAt = Number(backup.generatedAt)
  const date = Number.isFinite(generatedAt) ? new Date(generatedAt) : new Date()
  const stamp = date.toISOString().replace(/[:.]/g, '-')
  const blob = new Blob([JSON.stringify(backup, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `modelport-backup-${stamp}.json`
  anchor.click()
  URL.revokeObjectURL(url)
}
