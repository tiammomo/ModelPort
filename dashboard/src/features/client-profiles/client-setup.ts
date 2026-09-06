import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import { isMockMode } from '@/lib/mock-mode'
import type { DataClassification, HybridMode } from '@/types'

export interface ClientSetupCheck {
  apiKeyId: string
  model: string
  status: 'ready' | 'blocked'
  code: string
  checkedAtMs: number
  upstreamVerified: false
  defaultClassification?: DataClassification
  effectiveMode?: HybridMode
  ipRestricted?: boolean
  project?: { organizationId: string; projectId: string; environmentId: string }
}

export function useClientSetup(apiKeyId: string, model: string) {
  return useQuery({
    queryKey: ['client-setup', apiKeyId, model],
    queryFn: (): Promise<ClientSetupCheck> => isMockMode
      ? Promise.resolve({ apiKeyId, model, status: 'blocked', code: 'unverified', checkedAtMs: Date.now(), upstreamVerified: false })
      : api.get(`/admin/api-keys/${encodeURIComponent(apiKeyId)}/setup?model=${encodeURIComponent(model)}`),
    enabled: Boolean(apiKeyId && model),
    staleTime: 0,
    refetchOnMount: 'always',
    refetchOnWindowFocus: 'always',
    refetchInterval: 10_000,
  })
}

export function setupAllowsCopy(check: ClientSetupCheck | undefined, apiKeyId: string, model: string) {
  return Boolean(apiKeyId && model && check?.apiKeyId === apiKeyId && check.model === model && check.status === 'ready' && check.code === 'ready')
}

export function setupMessage(code?: string) {
  const messages: Record<string, string> = {
    ready: '所选密钥的模型权限、Provider 凭证和默认外发策略检查通过。',
    key_unavailable: '密钥已失效、被撤销或所属团队不可用。请检查密钥和团队状态。',
    control_only_key: '这是运维 Agent 控制密钥，不能调用模型。请选择独立的推理密钥。',
    owner_unavailable: '密钥所属用户不可用。请联系管理员恢复用户状态。',
    model_not_allowed: '密钥或团队未允许这个模型。请选择其他模型或联系管理员调整范围。',
    project_policy_denied: '项目外发策略未允许这条路由。请联系管理员检查 Provider、模型和外发边界。',
    local_only: '当前默认策略只允许本地执行，这个模型位于云端。请选择本地模型，或为已批准的项目明确数据分类。',
    model_unavailable: '模型已停用、未登记或没有符合策略的路由。请检查模型接入。',
    missing_credential: 'Provider 凭证未就绪。请联系管理员配置凭证后重试。',
    unverified: '演示模式无法验证真实接入，配置复制已禁用。',
  }
  return messages[code ?? ''] ?? '尚未获得有效的接入检查结果，配置复制已禁用。'
}
