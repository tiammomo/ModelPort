import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import {
  ArrowRight,
  AlertTriangle,
  Boxes,
  Copy,
  KeyRound,
  Scale,
  ScrollText,
  Terminal,
  Users,
} from 'lucide-react'
import { toast } from 'sonner'
import { useAliases, useApiKeys, useNow, useProviders, useSettings } from '@/hooks'
import { useAuthStore } from '@/stores'
import { PageHeader } from '@/components/shared/PageHeader'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { availableModelOptions, preferredAvailableModel } from '@/features/models/available-models'
import { apiKeyExpiryState } from '@/features/api-keys/api-key-view'
import { buildClientProfiles } from '@/features/client-profiles/client-profiles'

function CodeBlock({ children, copyLabel, copyDisabled = false }: { children: string; copyLabel: string; copyDisabled?: boolean }) {
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(children)
      toast.success(`${copyLabel}已复制`)
    } catch {
      toast.error('复制失败，请手动选择文本')
    }
  }

  return (
    <div className="relative">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="absolute right-2 top-2 z-10 h-8 border border-white/15 bg-white/10 text-slate-100 hover:bg-white/20 hover:text-white"
        onClick={() => void copy()}
        disabled={copyDisabled}
        aria-label={`复制${copyLabel}`}
      >
        <Copy className="h-3.5 w-3.5" />
        复制
      </Button>
      <pre className="overflow-x-auto rounded-md bg-slate-950 p-4 pr-24 font-mono text-xs leading-6 text-slate-100">
        <code>{children}</code>
      </pre>
    </div>
  )
}

function GuideStep({
  index,
  title,
  description,
  to,
}: {
  index: number
  title: string
  description: string
  to?: string
}) {
  const content = (
    <>
      <div className="flex items-center justify-between gap-3">
        <span className="font-mono text-xs font-semibold text-primary">0{index}</span>
        {to && <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />}
      </div>
      <p className="mt-3 font-semibold">{title}</p>
      <p className="mt-1.5 text-sm leading-6 text-muted-foreground">{description}</p>
    </>
  )

  const className = 'group border-t pt-4 transition-colors hover:border-primary/50'
  if (!to) return <li className={className}>{content}</li>
  return <li className={className}><Link to={to} className="block">{content}</Link></li>
}

function SectionHeading({
  eyebrow,
  title,
  description,
}: {
  eyebrow: string
  title: string
  description?: string
}) {
  return (
    <div className="mb-5">
      <p className="text-xs font-semibold uppercase tracking-[0.16em] text-primary">{eyebrow}</p>
      <h2 className="mt-2 text-lg font-semibold tracking-tight">{title}</h2>
      {description && <p className="mt-1.5 max-w-3xl text-sm leading-6 text-muted-foreground">{description}</p>}
    </div>
  )
}

export function UsageGuidePage() {
  const currentUser = useAuthStore((state) => state.currentUser)
  const isAdmin = currentUser?.role === 'admin'
  const catalogNow = useNow()
  const {
    data: apiKeys = [],
    isLoading: apiKeysLoading,
    isFetching: apiKeysFetching,
    error: apiKeysError,
  } = useApiKeys(!isAdmin)
  const usableApiKeys = useMemo(() => apiKeys.filter((key) => (
    key.status === 'active'
    && apiKeyExpiryState(key, catalogNow) !== 'expired'
    && (!key.ipRestricted || (key.allowedIps?.length ?? 0) > 0)
  )), [apiKeys, catalogNow])
  const [selectedCatalogKeyId, setSelectedCatalogKeyId] = useState('')
  const activeCatalogKeyId = usableApiKeys.some((key) => key.id === selectedCatalogKeyId)
    ? selectedCatalogKeyId
    : usableApiKeys[0]?.id ?? ''
  const catalogEnabled = isAdmin || Boolean(activeCatalogKeyId)
  const {
    data: providers = [],
    isLoading: providersLoading,
    isFetching: providersFetching,
    error: providersError,
  } = useProviders(
    isAdmin ? undefined : activeCatalogKeyId,
    catalogEnabled,
  )
  const {
    data: aliases = [],
    isLoading: aliasesLoading,
    isFetching: aliasesFetching,
    error: aliasesError,
  } = useAliases(
    isAdmin ? undefined : activeCatalogKeyId,
    catalogEnabled,
  )
  const { data: settings } = useSettings(isAdmin)
  const [selectedModel, setSelectedModel] = useState('')
  const gatewayOrigin = String(import.meta.env.VITE_API_BASE_URL || window.location.origin).replace(/\/+$/, '')

  const modelOptions = useMemo(() => {
    if (providersError || aliasesError || providersFetching || aliasesFetching || (!isAdmin && apiKeysFetching)) return []
    return availableModelOptions(providers, aliases)
  }, [aliases, aliasesError, aliasesFetching, apiKeysFetching, isAdmin, providers, providersError, providersFetching])
  const preferredModel = useMemo(
    () => preferredAvailableModel(modelOptions, settings?.gateway.defaultProvider),
    [modelOptions, settings?.gateway.defaultProvider],
  )

  const catalogLoading = providersLoading
    || providersFetching
    || aliasesLoading
    || aliasesFetching
    || (!isAdmin && (apiKeysLoading || apiKeysFetching))
  const catalogError = providersError || aliasesError || (!isAdmin ? apiKeysError : null)
  const activeModel = modelOptions.some((option) => option.id === selectedModel)
    ? selectedModel
    : preferredModel
  const copyDisabled = catalogLoading || Boolean(catalogError) || !activeModel
  const clientProfiles = useMemo(() => buildClientProfiles({
    gatewayOrigin,
    selectedModel: activeModel || undefined,
  }), [activeModel, gatewayOrigin])

  return (
    <div className="w-full">
      <div className="border-b pb-6">
        <PageHeader
          title="用户使用说明"
          description="从获取 API Key 到完成首次模型调用，并在请求日志中核对路由、Token、延迟和费用。"
        />
      </div>

      <section className="border-b py-8">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <SectionHeading
            eyebrow="Quick start"
            title="最短调用路径"
            description="普通用户只需要完成以下四步，不需要接触 Provider 凭证。"
          />
          <Badge variant="outline">约 5 分钟</Badge>
        </div>
        <ol className="grid gap-x-8 gap-y-6 sm:grid-cols-2 xl:grid-cols-4">
          <GuideStep index={1} title="获取 API Key" description="使用管理员签发的受限密钥；密钥明文只在创建时展示一次。" to="/api-keys" />
          <GuideStep index={2} title="确认模型 ID" description={activeModel ? `当前选择 ${activeModel}；模型来自实时目录，不使用固定示例。` : '选择 API Key，再从它的实时目录选择模型。'} to="/models" />
          <GuideStep index={3} title="选择客户端协议" description="Claude Code 使用 Anthropic Messages；Qwen Code 与 OpenAI SDK 使用 Chat Completions。" />
          <GuideStep index={4} title="核对请求日志" description="确认实际 Provider、模型、Token、计费和终止状态均符合预期。" to="/logs" />
        </ol>
      </section>

      <section className="border-b py-8">
        <SectionHeading
          eyebrow="Client setup"
          title="配置客户端"
          description="Client/Harness 使用 ModelPort 客户端密钥；Provider 凭证始终留在服务端。"
        />
        <div className="mb-6 rounded-lg border bg-muted/20 p-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <p className="text-sm font-semibold">选择当前可用模型</p>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                {isAdmin
                  ? '管理员看到组织当前可路由目录；优先使用稳定逻辑别名。'
                  : '目录由服务端按所选 API Key 的模型、Provider 与项目策略计算；IP 限制请由实际客户端用同一密钥请求 /v1/models 验证。'}
              </p>
            </div>
            <Badge variant="outline">{modelOptions.length} 个可用</Badge>
          </div>
          {!isAdmin && usableApiKeys.length > 0 && (
            <div className="mt-3">
              <Label htmlFor="guide-api-key" className="text-xs">用于查询目录的 API Key</Label>
              <Select
                value={activeCatalogKeyId}
                onValueChange={(value) => {
                  setSelectedCatalogKeyId(value)
                  setSelectedModel('')
                }}
                disabled={apiKeysLoading}
              >
                <SelectTrigger id="guide-api-key" className="mt-1 w-full bg-background" aria-label="选择用于查询模型目录的 API 密钥">
                  <SelectValue placeholder="选择 API Key" />
                </SelectTrigger>
                <SelectContent>
                  {usableApiKeys.map((key) => (
                    <SelectItem key={key.id} value={key.id}>
                      {key.name}（{key.keyPreview || key.keyPrefix}）
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          {catalogError ? (
            <p className="mt-3 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-xs text-rose-700" role="alert">
              无法读取实时模型目录，请检查会话和后端状态后重试。为避免误导，下面不会填入固定模型。
            </p>
          ) : modelOptions.length > 0 ? (
            <Select value={activeModel} onValueChange={setSelectedModel} disabled={catalogLoading}>
              <SelectTrigger className="mt-3 w-full bg-background" aria-label="选择可用模型">
                <SelectValue placeholder={catalogLoading ? '正在加载模型目录…' : '选择模型'} />
              </SelectTrigger>
              <SelectContent>
                {modelOptions.map((option) => (
                  <SelectItem key={option.id} value={option.id}>
                    {option.displayName}{option.kind === 'alias' ? '（逻辑模型）' : ''}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <div className="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900">
              <span>{catalogLoading ? '正在加载实时模型目录…' : isAdmin ? '当前没有通过凭证解析的启用模型。' : usableApiKeys.length === 0 ? '当前没有可用于查询目录的有效 API Key。' : '所选密钥的模型、Provider 或项目策略未允许任何已启用模型。'}</span>
              {!catalogLoading && <Button asChild size="sm" variant="outline"><Link to={isAdmin ? '/models' : '/api-keys'}>{isAdmin ? '检查 Provider' : '创建或检查密钥'}</Link></Button>}
            </div>
          )}
        </div>
        <div className="grid gap-6 xl:grid-cols-2">
          {clientProfiles.map((profile) => (
            <article key={profile.id} className="rounded-lg border p-4">
              <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
                <h3 className="flex items-center gap-2 font-semibold">
                  {profile.status === 'blocked' ? <AlertTriangle className="h-4 w-4 text-amber-600" /> : <Terminal className="h-4 w-4 text-primary" />}
                  {profile.name}
                </h3>
                <Badge variant="outline">{profile.status === 'supported' ? '可配置' : '暂不支持'}</Badge>
              </div>
              <p className="mb-3 text-sm leading-6 text-muted-foreground">{profile.description}</p>
              {profile.status === 'supported' ? (
                <CodeBlock copyLabel={`${profile.name} 配置`} copyDisabled={copyDisabled}>{profile.configuration}</CodeBlock>
              ) : (
                <div className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm leading-6 text-amber-950 dark:border-amber-900 dark:bg-amber-950/30 dark:text-amber-100">
                  <p>{profile.reason}</p>
                  <p className="mt-2 text-xs">{profile.followUp}</p>
                </div>
              )}
            </article>
          ))}
        </div>
      </section>

      <section className="grid gap-8 border-b py-8 lg:grid-cols-[1.35fr_0.65fr] lg:divide-x">
        <div className="lg:pr-8">
          <SectionHeading eyebrow="Verification" title="调用后检查" />
          <dl className="divide-y border-y">
            <div className="grid gap-1 py-4 sm:grid-cols-[160px_1fr] sm:gap-5">
              <dt className="font-medium">路由与模型</dt>
              <dd className="text-sm leading-6 text-muted-foreground">请求日志中的 Provider、请求模型和解析后模型应与预期一致。</dd>
            </div>
            <div className="grid gap-1 py-4 sm:grid-cols-[160px_1fr] sm:gap-5">
              <dt className="font-medium">用量与费用</dt>
              <dd className="text-sm leading-6 text-muted-foreground">优先使用上游返回的 Token，并核对本次请求保存的价格快照。</dd>
            </div>
            <div className="grid gap-1 py-4 sm:grid-cols-[160px_1fr] sm:gap-5">
              <dt className="font-medium">状态与延迟</dt>
              <dd className="text-sm leading-6 text-muted-foreground">区分上游错误、策略拒绝、超时和客户端主动取消，避免误判 Provider 故障。</dd>
            </div>
            <div className="grid gap-1 py-4 sm:grid-cols-[160px_1fr] sm:gap-5">
              <dt className="font-medium">权限边界</dt>
              <dd className="text-sm leading-6 text-muted-foreground">403 通常表示模型、Provider 或 IP 不在密钥策略内；429 表示配额或限流。</dd>
            </div>
          </dl>
        </div>

        <aside className="lg:pl-8">
          <SectionHeading eyebrow="Security" title="密钥安全" />
          <div className="space-y-4 text-sm leading-6 text-muted-foreground">
            <p>不要把 API Key 写入前端代码、聊天记录、截图或 Git 仓库。</p>
            <p>不同应用使用不同密钥，并限制允许的模型与 Provider；密钥泄露后立即吊销并重新签发。</p>
            <p>Dashboard 登录会话不能替代数据面 API Key，客户端调用必须携带独立密钥。</p>
          </div>
          <Button asChild variant="outline" className="mt-5">
            <Link to="/api-keys"><KeyRound className="h-4 w-4" />查看 API 密钥</Link>
          </Button>
        </aside>
      </section>

      {isAdmin && (
        <section className="border-b py-8">
          <SectionHeading
            eyebrow="Administrator"
            title="管理员：首次接入顺序"
            description="这部分只对管理员显示，完成后普通用户即可按上面的调用流程使用。"
          />
          <div className="divide-y border-y">
            <Link to="/models" className="group flex items-center gap-4 py-4">
              <Boxes className="h-5 w-5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1"><span className="block font-medium">1. 接入模型与渠道</span><span className="mt-1 block text-sm text-muted-foreground">配置上游、凭证和默认路由</span></span>
              <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </Link>
            <Link to="/governance" className="group flex items-center gap-4 py-4">
              <Scale className="h-5 w-5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1"><span className="block font-medium">2. 应用项目路由策略</span><span className="mt-1 block text-sm text-muted-foreground">在治理页明确本地与云外发边界；默认不会放行云 Provider</span></span>
              <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </Link>
            <Link to="/users" className="group flex items-center gap-4 py-4">
              <Users className="h-5 w-5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1"><span className="block font-medium">3. 创建或确认用户</span><span className="mt-1 block text-sm text-muted-foreground">确认角色、状态和归属</span></span>
              <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </Link>
            <Link to="/api-keys" className="group flex items-center gap-4 py-4">
              <KeyRound className="h-5 w-5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1"><span className="block font-medium">4. 签发最小权限密钥</span><span className="mt-1 block text-sm text-muted-foreground">限制模型、Provider 和预算</span></span>
              <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </Link>
          </div>
        </section>
      )}

      <footer className="flex flex-wrap items-center justify-between gap-4 py-6 text-sm">
        <div className="flex items-center gap-2 text-muted-foreground">
          <ScrollText className="h-4 w-4 text-primary" />
          <span>完成调用后，请求日志是路由、用量和计费的最终核对入口。</span>
        </div>
        <Button asChild size="sm"><Link to="/logs">打开请求日志<ArrowRight className="h-4 w-4" /></Link></Button>
      </footer>
    </div>
  )
}
