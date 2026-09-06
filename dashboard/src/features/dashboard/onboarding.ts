import type { DashboardStats, Provider, SystemSettings } from '@/types'

export interface OnboardingStep {
  id: string
  title: string
  detail: string
  to: string
  complete: boolean
}

export interface OnboardingState {
  steps: OnboardingStep[]
  completed: number
  total: number
  percent: number
  complete: boolean
}

export function buildOnboardingState({
  providers,
  settings,
  stats,
}: {
  providers: readonly Provider[]
  settings?: SystemSettings
  stats: DashboardStats
}): OnboardingState {
  const credentialReadyProviders = providers.filter((provider) => (
    provider.status === 'active'
    && (
      provider.hasApiKey
      || !provider.apiKeyRequired
      || Boolean(provider.credentials?.some((credential) => credential.status === 'active' && credential.hasApiKey))
    )
  ))
  const testedProviders = credentialReadyProviders.filter((provider) => provider.lastTest?.success)
  const hasModel = testedProviders.some((provider) => (
    provider.models.some((model) => (
      !provider.modelInventory?.some((item) => item.model === model && item.status === 'disabled')
    ))
  ))
  const defaultProvider = providers.find((provider) => provider.id === settings?.gateway.defaultProvider)
  const routeReady = Boolean(
    settings?.setup?.defaultProviderReady
    ?? (defaultProvider && testedProviders.some((provider) => provider.id === defaultProvider.id)),
  )
  const policyReady = stats.onboardingMilestones?.hasDefaultProjectPolicy ?? false
  const hasRequest = stats.onboardingMilestones?.hasRequestEver ?? stats.totalRequests > 0
  const hasEvidence = stats.onboardingMilestones?.hasSuccessfulRequestEver ?? false

  const steps: OnboardingStep[] = [
    {
      id: 'provider',
      title: 'Provider 凭证与连接',
      detail: credentialReadyProviders.length === 0
        ? '保存 Secret 环境变量引用，注入后重启进程。'
        : testedProviders.length === 0
          ? '凭证已解析；继续运行连接测试并发现模型。'
          : `${testedProviders.length} 个 Provider 已通过连接测试。`,
      to: '/models',
      complete: testedProviders.length > 0,
    },
    {
      id: 'model',
      title: '至少一个模型可用',
      detail: hasModel ? '已形成可路由模型目录。' : '发现或启用一个已验证 Provider 的模型。',
      to: '/models',
      complete: hasModel,
    },
    {
      id: 'route',
      title: '默认路由与外发策略',
      detail: !routeReady
        ? '先确认默认 Provider，再明确项目的本地与云外发边界。'
        : policyReady
          ? `默认 Provider：${settings?.gateway.defaultProvider || defaultProvider?.id || '已配置'}；项目策略已应用。`
          : '默认 Provider 已就绪；还需在治理页应用 org_local/prj_default/env_default 项目策略。',
      to: routeReady ? '/governance' : '/models',
      complete: routeReady && policyReady,
    },
    {
      id: 'identity',
      title: '用户与受限密钥',
      detail: (stats.apiKeysActive ?? 0) > 0
        ? `${stats.apiKeysActive} 把启用密钥；${stats.activeUsers} 个活跃用户。`
        : '创建开发者账号并签发最小权限密钥。',
      to: '/api-keys',
      complete: (stats.apiKeysActive ?? 0) > 0,
    },
    {
      id: 'request',
      title: '首次受治理请求',
      detail: hasRequest ? '网关已接收模型请求。' : '复制客户端配置并发送首次请求。',
      to: '/guide',
      complete: hasRequest,
    },
    {
      id: 'evidence',
      title: '路由证据已记录',
      detail: hasEvidence ? '请求 ID、实际 Provider 与模型均已记录。' : '在请求日志核对路由、身份、用量与结果。',
      to: '/logs',
      complete: hasEvidence,
    },
  ]
  const completed = steps.filter((step) => step.complete).length

  return {
    steps,
    completed,
    total: steps.length,
    percent: Math.round((completed / steps.length) * 100),
    complete: completed === steps.length,
  }
}

export function setupJourneySteps(state: OnboardingState): OnboardingStep[] {
  const step = (id: string) => state.steps.find((item) => item.id === id)!
  const provider = step('provider')
  const model = step('model')
  const identity = step('identity')
  const request = step('request')
  return [
    { ...provider, title: '接入模型', complete: provider.complete && model.complete, detail: provider.complete ? model.detail : provider.detail },
    { ...step('route'), title: '设置外发边界', to: '/governance' },
    { ...(identity.complete ? request : identity), id: 'client', title: '密钥与客户端', complete: identity.complete && request.complete },
    { ...step('evidence'), title: '核对请求结果' },
  ]
}

export function nextSetupPath(state: OnboardingState) {
  const steps = setupJourneySteps(state)
  return `${(steps.find((step) => !step.complete) ?? steps[steps.length - 1]).to}?setup=1`
}
