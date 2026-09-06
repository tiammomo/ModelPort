import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import type { Provider } from '@/types'
import { selectPolicyProvider, type ProjectPolicyDraft } from './project-policy'

export function ProjectPolicyEditor({ value, onChange, providers }: {
  value: ProjectPolicyDraft
  onChange: (value: ProjectPolicyDraft) => void
  providers: readonly Provider[]
}) {
  const selectClass = 'h-9 w-full rounded-lg border border-input bg-background px-3 text-sm'
  const input = (field: keyof ProjectPolicyDraft, label: string) => (
    <div className="space-y-2" key={field}>
      <Label htmlFor={`policy-${field}`}>{label}</Label>
      <Input id={`policy-${field}`} value={String(value[field])} onChange={(event) => onChange({ ...value, [field]: event.target.value })} />
    </div>
  )
  return (
    <fieldset className="min-w-0 space-y-4">
      <legend className="mb-3 font-medium">项目模型接入策略</legend>
      <p className="text-sm text-muted-foreground">选择允许使用的模型和数据边界，记录后应用。默认仅允许本地执行。</p>
      <div className="space-y-2">
        <Label htmlFor="policy-provider">选择 Provider</Label>
        <select id="policy-provider" className={selectClass} value={providers.some((provider) => provider.id === value.allowedProviders) ? value.allowedProviders : ''} onChange={(event) => {
          const provider = providers.find((item) => item.id === event.target.value)
          if (provider) onChange(selectPolicyProvider(value, provider))
        }}>
          <option value="" disabled>选择已配置的 Provider</option>
          {providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.displayName}</option>)}
        </select>
      </div>
      {input('allowedModels', '允许的模型（逗号分隔）')}
      <div className="space-y-2">
        <Label htmlFor="policy-egress">模型执行范围</Label>
        <select id="policy-egress" className={selectClass} value={value.cloudEnabled ? 'cloud' : 'local'} onChange={(event) => onChange({
          ...value, cloudEnabled: event.target.value === 'cloud', maximumMode: event.target.value === 'cloud' ? 'cloud_first' : 'local_strict',
        })}>
          <option value="local">仅本地：禁止云外发</option>
          <option value="cloud">允许获批云：仅限上述 Provider 和模型</option>
        </select>
      </div>
      {value.cloudEnabled && <p className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-950">云调用会将请求数据发送给获批 Provider，并可能产生费用。只有明确分类为内部或公开的数据才可按此策略外发。</p>}
      <div className="space-y-2">
        <Label htmlFor="policy-classification">未携带分类信息时的数据类型</Label>
        <select id="policy-classification" className={selectClass} value={value.defaultClassification} onChange={(event) => onChange({ ...value, defaultClassification: event.target.value as ProjectPolicyDraft['defaultClassification'] })}>
          <option value="unknown">未知：保留本地限制（默认）</option>
          <option value="sensitive">敏感：保留本地限制</option>
          <option value="internal">内部：仅适用于已批准外发的项目数据</option>
          <option value="public">公开：仅适用于不包含私有数据的项目</option>
        </select>
        <p className="text-xs text-muted-foreground">未知和敏感请求始终仅在本地执行。需要直接复制云端客户端配置时，应为获批项目明确分类；不要把含未知或敏感数据的项目设为内部或公开。</p>
      </div>
      <details className="rounded-md border p-3">
        <summary className="cursor-pointer text-sm font-medium">项目范围与高级边界</summary>
        <div className="mt-4 grid gap-4 sm:grid-cols-2">
          {input('organizationId', '组织标识')}{input('projectId', '项目标识')}{input('environmentId', '环境标识')}
          {input('allowedProviders', '允许的 Provider（逗号分隔）')}
          {input('allowedRegions', '允许的区域（逗号分隔）')}{input('allowedApiVersions', '允许的 API 版本（逗号分隔）')}
          {value.cloudEnabled && <div className="space-y-2">
            <Label htmlFor="policy-mode">路由模式上限</Label>
            <select id="policy-mode" className={selectClass} value={value.maximumMode} onChange={(event) => onChange({ ...value, maximumMode: event.target.value as ProjectPolicyDraft['maximumMode'] })}>
              <option value="local_strict">仅本地</option><option value="local_first">本地优先</option><option value="balanced">均衡</option><option value="cloud_first">云优先</option>
            </select>
          </div>}
        </div>
      </details>
    </fieldset>
  )
}
