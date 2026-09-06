import { useMemo, useState } from 'react'
import { AlertTriangle, CheckCircle2, Clock3, Loader2, RefreshCw, Send, ShieldCheck } from 'lucide-react'
import { PageHeader } from '@/components/shared/PageHeader'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { ErrorState } from '@/components/shared/ErrorState'
import { StatusBadge } from '@/components/shared/StatusBadge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  useApplyGovernanceChange,
  useApproveGovernanceChange,
  useCreateGovernanceChange,
  useGovernance,
} from '@/hooks/use-governance'
import type { GovernanceChangeRequest } from '@/types'
import { useProviders } from '@/hooks/use-models'
import { ProjectPolicyEditor } from '@/features/governance/ProjectPolicyEditor'
import { buildProjectPolicy, EMPTY_PROJECT_POLICY } from '@/features/governance/project-policy'

const ACTIONS = [
  ['project_policy.upsert', '项目路由策略'],
  ['provider.allowlist_change', 'Provider 白名单'],
  ['routing.cloud_first', '启用 cloud_first'],
  ['budget.hard_limit', '预算硬上限'],
  ['identity.permission', '身份与权限'],
  ['model.production_promotion', '生产模型晋级'],
  ['data_egress.change', '数据外发范围'],
  ['database.major_migration', '数据库大版本迁移'],
  ['secret.rotation', '生产密钥轮换'],
] as const

const DIRECT_APPLY_ACTIONS = new Set(['project_policy.upsert', 'budget.hard_limit'])

const PAYLOAD_TEMPLATES: Record<string, unknown> = {
  'provider.allowlist_change': { providerId: '', operation: 'add', region: '', apiVersion: '', models: [] },
  'routing.cloud_first': { organizationId: 'local', projectId: 'default', environmentId: 'production', enabled: true },
  'budget.hard_limit': { organizationId: 'local', projectId: 'default', environmentId: 'production', hardLimitMicrounits: 0 },
  'identity.permission': { subjectId: '', operation: 'role_change', role: 'admin' },
  'model.production_promotion': { providerId: '', model: '', environmentId: 'production' },
  'data_egress.change': { projectId: 'default', classification: 'internal', operation: 'allow' },
  'database.major_migration': { fromMajor: 16, toMajor: 18, backupArchive: '', rollbackPlan: '' },
  'secret.rotation': { secretName: '', providerId: '', overlapMinutes: 30 },
}

export function GovernancePage() {
  const { data, isLoading, error, refetch } = useGovernance()
  const { data: providers = [] } = useProviders()
  const createChange = useCreateGovernanceChange()
  const approveChange = useApproveGovernanceChange()
  const applyChange = useApplyGovernanceChange()
  const [action, setAction] = useState(ACTIONS[0][0])
  const [target, setTarget] = useState('org_local/prj_default/env_default')
  const [reason, setReason] = useState('')
  const [payloadText, setPayloadText] = useState(() => formatTemplate(ACTIONS[0][0]))
  const [policyDraft, setPolicyDraft] = useState(EMPTY_PROJECT_POLICY)
  const [notice, setNotice] = useState<{ kind: 'success' | 'error'; text: string } | null>(null)
  const [selectedChangeId, setSelectedChangeId] = useState(() => window.sessionStorage.getItem('modelport_change_request_id') || '')

  const actionLabels = useMemo(() => new Map(ACTIONS), [])

  if (isLoading) return <LoadingPage />
  if (error || !data) {
    return (
      <ErrorState
        title="治理控制台加载失败"
        message={error instanceof Error ? error.message : '无法读取治理状态'}
        onRetry={() => void refetch()}
      />
    )
  }

  const submit = () => {
    setNotice(null)
    let payload: unknown
    let changeTarget = target
    try {
      if (action === 'project_policy.upsert') {
        const policy = buildProjectPolicy(policyDraft)
        payload = policy
        changeTarget = `${policy.organizationId}/${policy.projectId}/${policy.environmentId}`
      } else {
        payload = JSON.parse(payloadText)
      }
    } catch (error) {
      setNotice({ kind: 'error', text: error instanceof Error ? error.message : '请检查变更内容' })
      return
    }
    createChange.mutate({ action, target: changeTarget, reason, payload }, {
      onSuccess: (change) => {
        setReason('')
        setNotice({
          kind: 'success',
          text: data.dualApprovalRequired
            ? `已创建 ${change.id}，等待另一名管理员审批`
            : `已记录 ${change.id}；可直接应用支持的变更，也可等待另一名管理员复核`,
        })
      },
      onError: (mutationError) => setNotice({
        kind: 'error',
        text: mutationError instanceof Error ? mutationError.message : '创建失败',
      }),
    })
  }

  const mutate = (kind: 'approve' | 'apply', change: GovernanceChangeRequest) => {
    setNotice(null)
    const mutation = kind === 'approve' ? approveChange : applyChange
    mutation.mutate(change.id, {
      onSuccess: () => setNotice({
        kind: 'success',
        text: kind === 'approve' ? '第二人审批完成' : '变更已应用并写入审计记录',
      }),
      onError: (mutationError) => setNotice({
        kind: 'error',
        text: mutationError instanceof Error ? mutationError.message : '操作失败',
      }),
    })
  }

  const scheduler = data.scheduler
  const approvalMode = data.dualApprovalRequired ? '强制双人审批' : '可选双人复核'
  return (
    <div className="space-y-6">
      <PageHeader
        title="治理与变更审批"
        description={data.dualApprovalRequired
          ? '高风险变更必须先形成载荷摘要，再由另一名管理员审批；Dashboard 与 API 共用同一门禁。'
          : '免费小团队模式允许管理员直接执行并保留审计；需要复核时仍可使用完整的双人审批流程。'}
        action={{ label: '刷新状态', onClick: () => void refetch(), icon: RefreshCw }}
      />

      {notice && (
        <div className={`rounded-lg border p-3 text-sm ${notice.kind === 'success' ? 'border-emerald-300 bg-emerald-50 text-emerald-800' : 'border-red-300 bg-red-50 text-red-800'}`} role="status">
          {notice.text}
        </div>
      )}

      {selectedChangeId && (
        <div className="flex flex-col gap-2 rounded-lg border border-blue-200 bg-blue-50 p-3 text-sm text-blue-900 sm:flex-row sm:items-center sm:justify-between" role="status">
          <span>专用操作已选择审批单 <code>{selectedChangeId}</code>；后续 Dashboard 写请求会自动携带该 ID，服务端仍会校验动作、目标与载荷摘要。</span>
          <Button size="sm" variant="outline" onClick={() => {
            window.sessionStorage.removeItem('modelport_change_request_id')
            setSelectedChangeId('')
          }}>清除选择</Button>
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle>提交高风险变更</CardTitle>
          <CardDescription>
            {data.dualApprovalRequired
              ? '提交人自动成为第一审批人；同一账号不能完成第二次审批。'
              : '当前为可选复核模式；可直接应用支持的变更，也可等待另一名管理员完成复核。'}
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4">
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="governance-action">变更类型</Label>
              <select
                id="governance-action"
                className="h-9 w-full rounded-lg border border-input bg-background px-3 text-sm"
                value={action}
                onChange={(event) => {
                  const next = event.target.value
                  setAction(next as typeof action)
                  setPayloadText(formatTemplate(next))
                }}
              >
                {ACTIONS.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
              </select>
            </div>
            {action !== 'project_policy.upsert' && <div className="space-y-2">
              <Label htmlFor="governance-target">目标标识</Label>
              <Input id="governance-target" value={target} onChange={(event) => setTarget(event.target.value)} placeholder="org_local/prj_default/env_default" />
            </div>}
          </div>
          {action === 'project_policy.upsert' && <ProjectPolicyEditor value={policyDraft} onChange={setPolicyDraft} providers={providers} />}
          <div className="space-y-2">
            <Label htmlFor="governance-reason">业务原因与回滚依据</Label>
            <Input id="governance-reason" value={reason} onChange={(event) => setReason(event.target.value)} placeholder="说明必要性、影响面、验证和回滚条件" />
          </div>
          {action !== 'project_policy.upsert' && <div className="space-y-2">
            <Label htmlFor="governance-payload">精确变更载荷（JSON）</Label>
            <textarea
              id="governance-payload"
              className="min-h-52 w-full rounded-lg border border-input bg-background p-3 font-mono text-xs leading-5"
              value={payloadText}
              onChange={(event) => setPayloadText(event.target.value)}
              spellCheck={false}
            />
          </div>}
          <div className="flex justify-end">
            <Button onClick={submit} disabled={createChange.isPending || reason.trim().length < 8 || !target.trim()}>
              {createChange.isPending ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <Send className="mr-2 h-4 w-4" />}
              {data.dualApprovalRequired ? '提交并记录第一人审批' : '记录变更意图'}
            </Button>
          </div>
        </CardContent>
      </Card>

      <div className="grid gap-4 md:grid-cols-4">
        <Metric title="治理存储" value={data.ready ? '就绪' : '降级'} detail="审批状态持久化" icon={data.ready ? CheckCircle2 : AlertTriangle} />
        <Metric title="审批门禁" value={approvalMode} detail={data.dualApprovalRequired ? '高风险写入必须匹配审批单' : '直接写入仍受 CSRF 与审计保护'} icon={ShieldCheck} />
        <Metric title="交互队列" value={`${scheduler.interactiveQueued} / ${scheduler.limits.globalInteractiveQueue}`} detail="全局本地队列" icon={Clock3} />
        <Metric title="后台队列" value={`${scheduler.batchQueued} / ${scheduler.limits.globalBatchQueue}`} detail="独立低优先级" icon={Clock3} />
      </div>

      <Card>
        <CardHeader>
          <CardTitle>审批队列</CardTitle>
          <CardDescription>
            {data.dualApprovalRequired
              ? 'Provider、身份、数据库与密钥类变更审批后仍由专用生产 Runbook 执行。'
              : '可选审批不会阻断普通管理员操作；选用审批单时仍会校验动作、目标与载荷摘要。'}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {data.changeRequests.length === 0 && <p className="py-8 text-center text-sm text-muted-foreground">暂无高风险变更</p>}
          {data.changeRequests.map((change) => (
            <div key={change.id} data-testid={`governance-change-${change.id}`} className="rounded-xl border border-border/80 p-4">
              <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                <div className="min-w-0 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-medium">{actionLabels.get(change.action as typeof ACTIONS[number][0]) || change.action}</span>
                    <StatusBadge status={change.status} />
                    <code className="text-xs text-muted-foreground">{change.id}</code>
                  </div>
                  <p className="text-sm text-muted-foreground">目标：{change.target} · 提交人：{change.requestedByName}</p>
                  <p className="text-sm">{change.reason}</p>
                  <p className="break-all font-mono text-[11px] text-muted-foreground">SHA-256 {change.payloadSha256}</p>
                  <p className="text-xs text-muted-foreground">审批：{change.approvals.map((approval) => approval.actorName).join(' → ')}</p>
                </div>
                <div className="flex shrink-0 gap-2">
                  {change.status === 'pending_second_approval' && (
                    <Button size="sm" variant="outline" disabled={approveChange.isPending} onClick={() => mutate('approve', change)}>第二人审批</Button>
                  )}
                  {!data.dualApprovalRequired && change.status === 'pending_second_approval' && DIRECT_APPLY_ACTIONS.has(change.action) && (
                    <Button size="sm" disabled={applyChange.isPending} onClick={() => mutate('apply', change)}>直接应用</Button>
                  )}
                  {change.status === 'approved' && DIRECT_APPLY_ACTIONS.has(change.action) && (
                    <Button size="sm" disabled={applyChange.isPending} onClick={() => mutate('apply', change)}>应用变更</Button>
                  )}
                  {change.status === 'approved' && !DIRECT_APPLY_ACTIONS.has(change.action) && (
                    <Button size="sm" variant="outline" onClick={() => {
                      window.sessionStorage.setItem('modelport_change_request_id', change.id)
                      setSelectedChangeId(change.id)
                      setNotice({ kind: 'success', text: '已选择审批单；请到对应管理员页面执行载荷完全一致的操作' })
                    }}>用于下一次专用操作</Button>
                  )}
                </div>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  )
}

function Metric({ title, value, detail, icon: Icon }: { title: string; value: string; detail: string; icon: typeof ShieldCheck }) {
  return (
    <Card>
      <CardContent className="flex items-start justify-between p-5">
        <div><p className="text-xs text-muted-foreground">{title}</p><p className="mt-1 text-xl font-semibold">{value}</p><p className="mt-1 text-xs text-muted-foreground">{detail}</p></div>
        <Icon className="h-5 w-5 text-primary" />
      </CardContent>
    </Card>
  )
}

function formatTemplate(action: string) {
  return JSON.stringify(PAYLOAD_TEMPLATES[action] || {}, null, 2)
}
