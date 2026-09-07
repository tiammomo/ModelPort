import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import {
  useCreateProviderCredential,
  useDeleteProviderCredential,
  useSelectProviderCredential,
  useUpdateProviderCredential,
  useUpdateProviderCredentialPoolMode,
} from '@/hooks'
import { focusFirstInvalidDialogField, formatNumber, formatRelativeTime } from '@/lib/utils'
import type { Provider, ProviderCredential, ProviderCredentialPoolMode, ProviderOnlineBalance } from '@/types'
import { AlertTriangle, Loader2, Pencil, Plus, Trash2, WalletCards } from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'
import {
  CREDENTIAL_POOL_MODE_LABELS,
  DEFAULT_CREDENTIAL_FORM,
  credentialPayloadFromForm,
  credentialToForm,
  providerCredentialState,
  providerDisplayTitle,
  type ProviderCredentialFormState,
} from './model-data'
import { Field, SwitchRow } from './ModelFormField'
import { validateCredentialForm } from './operator-state'

export function ProviderCredentials({ provider, canManage, onlineBalance, checkingBalance, onCheckBalance }: {
  provider: Provider
  canManage: boolean
  onlineBalance?: ProviderOnlineBalance
  checkingBalance: boolean
  onCheckBalance: () => void
}) {
  const createProviderCredential = useCreateProviderCredential()
  const updateProviderCredential = useUpdateProviderCredential()
  const selectProviderCredential = useSelectProviderCredential()
  const updateProviderCredentialPoolMode = useUpdateProviderCredentialPoolMode()
  const deleteProviderCredential = useDeleteProviderCredential()
  const [credentialSubmitAttempted, setCredentialSubmitAttempted] = useState(false)
  const [credentialDialogOpen, setCredentialDialogOpen] = useState(false)
  const [editingCredential, setEditingCredential] = useState<ProviderCredential | null>(null)
  const [credentialForm, setCredentialForm] = useState<ProviderCredentialFormState>(DEFAULT_CREDENTIAL_FORM)
  const [credentialDeleteTarget, setCredentialDeleteTarget] = useState<ProviderCredential | null>(null)

  const { credentials, activeCredential, credentialReady, credentialPoolMode } = providerCredentialState(provider)
  const displayTitle = providerDisplayTitle(provider)
  const credentialBusy = selectProviderCredential.isPending || updateProviderCredentialPoolMode.isPending || deleteProviderCredential.isPending
  const credentialValidation = useMemo(
    () => validateCredentialForm(credentialForm, !editingCredential),
    [credentialForm, editingCredential],
  )
  const openCredentialDialog = (credential?: ProviderCredential) => {
    setCredentialDialogOpen(true)
    setEditingCredential(credential ?? null)
    setCredentialForm(credentialToForm(provider, credential))
    setCredentialSubmitAttempted(false)
  }

  const closeCredentialDialog = () => {
    setCredentialDialogOpen(false)
    setEditingCredential(null)
    setCredentialForm(DEFAULT_CREDENTIAL_FORM)
    setCredentialSubmitAttempted(false)
  }

  const handleSubmitCredential = () => {
    if (!credentialDialogOpen) return
    setCredentialSubmitAttempted(true)
    if (!credentialValidation.valid) {
      toast.error('请先修正账号表单中的错误')
      focusFirstInvalidDialogField()
      return
    }
    const data = credentialPayloadFromForm(credentialForm, !editingCredential)
    const options = {
      onSuccess: () => {
        toast.success(editingCredential
          ? '账号引用已更新；如环境变量值有变化，请重启进程并重新测试'
          : '账号引用已新增；注入环境变量、重启进程并重新测试后才会生效')
        closeCredentialDialog()
      },
      onError: (error: unknown) => toast.error(error instanceof Error ? error.message : '保存账号失败'),
    }

    if (editingCredential) {
      updateProviderCredential.mutate({
        providerId: provider.id,
        credentialId: editingCredential.id,
        data,
      }, options)
    } else {
      createProviderCredential.mutate({
        providerId: provider.id,
        data,
      }, options)
    }
  }

  const handleSelectProviderCredential = (credentialId: string) => {
    selectProviderCredential.mutate({ providerId: provider.id, credentialId }, {
      onSuccess: () => toast.success(`已切换 ${provider.displayName} 账号`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '切换账号失败'),
    })
  }

  const handleUpdateProviderCredentialPoolMode = (mode: ProviderCredentialPoolMode) => {
    updateProviderCredentialPoolMode.mutate({ providerId: provider.id, mode }, {
      onSuccess: () => toast.success(`已更新 ${provider.displayName} 号池策略`),
      onError: (error) => toast.error(error instanceof Error ? error.message : '更新号池策略失败'),
    })
  }

  const handleDeleteProviderCredential = () => {
    if (!credentialDeleteTarget) return
    const credential = credentialDeleteTarget
    deleteProviderCredential.mutate({ providerId: provider.id, credentialId: credential.id }, {
      onSuccess: () => {
        toast.success(`已删除账号 ${credential.name}`)
        setCredentialDeleteTarget(null)
      },
      onError: (error) => toast.error(error instanceof Error ? error.message : '删除账号失败'),
    })
  }

  return (
    <>
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
              onValueChange={(value) => handleUpdateProviderCredentialPoolMode(value as ProviderCredentialPoolMode)}
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
            {canManage && <Button variant="outline" size="sm" onClick={() => openCredentialDialog()}>
              <Plus className="h-3.5 w-3.5" />
              新增
            </Button>}
          </div>
        </div>
        {provider.id === 'deepseek' && canManage && (
          <div className="mb-3 rounded-md border bg-background/70 p-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <p className="text-sm font-medium">DeepSeek 线上余额</p>
                  {onlineBalance && (
                    <Badge variant={onlineBalance.isAvailable ? 'success' : 'destructive'}>
                      {onlineBalance.isAvailable ? '可调用' : '余额不足'}
                    </Badge>
                  )}
                </div>
                <p className="mt-1 text-xs text-muted-foreground">
                  实时只读查询；充值、退款与账单以 DeepSeek 控制台为准。
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={onCheckBalance}
                disabled={checkingBalance || !credentialReady}
              >
                {checkingBalance
                  ? <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  : <WalletCards className="mr-2 h-4 w-4" />}
                {checkingBalance ? '查询中' : '查询余额'}
              </Button>
            </div>
            {onlineBalance && (
              <div className="mt-3 grid gap-2 sm:grid-cols-2">
                {onlineBalance.balanceInfos.map((balance) => (
                  <div key={balance.currency} className="rounded-md bg-muted/40 px-3 py-2 text-xs">
                    <p className="text-muted-foreground">{balance.currency} 可用总额</p>
                    <p className="mt-1 font-mono text-base font-semibold text-foreground">
                      {balance.totalBalance} {balance.currency}
                    </p>
                    <p className="mt-1 text-muted-foreground">
                      赠金 {balance.grantedBalance} · 充值 {balance.toppedUpBalance}
                    </p>
                  </div>
                ))}
                <p className="self-end text-xs text-muted-foreground">
                  最近查询：{formatRelativeTime(onlineBalance.checkedAt)}
                </p>
              </div>
            )}
          </div>
        )}
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
              onValueChange={(id) => handleSelectProviderCredential(id)}
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
                  {canManage && <Button variant="outline" size="sm" onClick={() => openCredentialDialog(activeCredential)} aria-label={`编辑上游账号 ${activeCredential.name}`} disabled={credentialBusy}>
                    <Pencil className="h-3.5 w-3.5" />
                    编辑
                  </Button>}
                  {canManage && <Button
                    variant="outline"
                    size="sm"
                    className="text-destructive hover:text-destructive"
                    onClick={() => setCredentialDeleteTarget(activeCredential)}
                    aria-label={`删除上游账号 ${activeCredential.name}`}
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

      <Dialog open={credentialDialogOpen} onOpenChange={(open) => { if (!open) closeCredentialDialog() }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editingCredential ? '编辑上游账号' : '新增上游账号'}</DialogTitle>
            <DialogDescription>
              账号只保存环境变量名；真实 API Key 仍放在 .env、容器 Secret 或系统环境变量中。保存后需重启并重新运行 Provider 连接测试。
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

      <Dialog open={!!credentialDeleteTarget} onOpenChange={(open) => { if (!open) setCredentialDeleteTarget(null) }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>删除上游账号</DialogTitle>
            <DialogDescription>账号配置和健康记录会删除；真实环境变量不会被修改。</DialogDescription>
          </DialogHeader>
          <div className="rounded-md border bg-muted/30 p-3 text-sm">
            <p className="font-medium">{credentialDeleteTarget?.name}</p>
            <p className="mt-1 font-mono text-xs text-muted-foreground">{credentialDeleteTarget?.apiKeyEnv}</p>
            {credentialDeleteTarget?.active && (
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
    </>
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

