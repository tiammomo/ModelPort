import { Link } from 'react-router-dom'
import {
  ArrowRight,
  Boxes,
  KeyRound,
  ScrollText,
  Terminal,
  Users,
} from 'lucide-react'
import { useAuthStore } from '@/stores'
import { PageHeader } from '@/components/shared/PageHeader'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'

const EXAMPLE_MODEL = 'local_qwen:qwen3.5-9b-q5km'

function CodeBlock({ children }: { children: string }) {
  return (
    <pre className="overflow-x-auto rounded-md bg-slate-950 p-4 font-mono text-xs leading-6 text-slate-100">
      <code>{children}</code>
    </pre>
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
  const isAdmin = useAuthStore((state) => state.currentUser?.role === 'admin')
  const gatewayOrigin = window.location.origin

  const anthropicEnv = `ANTHROPIC_BASE_URL=${gatewayOrigin}\nANTHROPIC_AUTH_TOKEN=<你的 ModelPort API Key>\nANTHROPIC_MODEL=${EXAMPLE_MODEL}`
  const openAiEnv = `OPENAI_BASE_URL=${gatewayOrigin}/v1\nOPENAI_API_KEY=<你的 ModelPort API Key>\nOPENAI_MODEL=${EXAMPLE_MODEL}`

  return (
    <div className="mx-auto max-w-7xl">
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
          <GuideStep index={2} title="确认模型 ID" description={`使用密钥允许访问的模型，例如 ${EXAMPLE_MODEL}。`} />
          <GuideStep index={3} title="选择客户端协议" description="Claude Code 使用 Anthropic Messages；OpenAI SDK 使用 Chat Completions。" />
          <GuideStep index={4} title="核对请求日志" description="确认实际 Provider、模型、Token、计费和终止状态均符合预期。" to="/logs" />
        </ol>
      </section>

      <section className="border-b py-8">
        <SectionHeading
          eyebrow="Client setup"
          title="配置客户端"
          description="两种客户端协议使用同一套 ModelPort API Key，但 Base URL 规则不同。"
        />
        <div className="grid gap-8 xl:grid-cols-2 xl:gap-0 xl:divide-x">
          <article className="xl:pr-8">
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
              <h3 className="flex items-center gap-2 font-semibold"><Terminal className="h-4 w-4 text-primary" />Anthropic-compatible</h3>
              <span className="text-xs text-muted-foreground">Claude Code / Anthropic SDK</span>
            </div>
            <p className="mb-3 text-sm leading-6 text-muted-foreground">客户端连接 ModelPort，而不是直接连接实际模型上游。</p>
            <CodeBlock>{anthropicEnv}</CodeBlock>
            <p className="mt-3 text-xs text-muted-foreground">接口：<code className="font-mono">POST /v1/messages</code>，认证头：<code className="font-mono">x-api-key</code>。</p>
          </article>

          <article className="xl:pl-8">
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
              <h3 className="flex items-center gap-2 font-semibold"><Terminal className="h-4 w-4 text-primary" />OpenAI-compatible</h3>
              <span className="text-xs text-muted-foreground">OpenAI SDK / Quant</span>
            </div>
            <p className="mb-3 text-sm leading-6 text-muted-foreground">Base URL 需要包含 <code className="font-mono">/v1</code>，模型 ID 与密钥权限保持一致。</p>
            <CodeBlock>{openAiEnv}</CodeBlock>
            <p className="mt-3 text-xs text-muted-foreground">接口：<code className="font-mono">POST /v1/chat/completions</code>，认证头：<code className="font-mono">Authorization: Bearer …</code>。</p>
          </article>
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
            <Link to="/users" className="group flex items-center gap-4 py-4">
              <Users className="h-5 w-5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1"><span className="block font-medium">2. 创建或确认用户</span><span className="mt-1 block text-sm text-muted-foreground">确认角色、状态和归属</span></span>
              <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
            </Link>
            <Link to="/api-keys" className="group flex items-center gap-4 py-4">
              <KeyRound className="h-5 w-5 shrink-0 text-primary" />
              <span className="min-w-0 flex-1"><span className="block font-medium">3. 签发最小权限密钥</span><span className="mt-1 block text-sm text-muted-foreground">限制模型、Provider 和预算</span></span>
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
