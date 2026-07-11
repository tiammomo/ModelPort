import type { Provider } from '@/types'
import { parseList, type ProviderCredentialFormState, type ProviderFormState } from './model-data'

export type ProviderReadinessLevel = 'ready' | 'attention' | 'blocked' | 'disabled'

export interface ProviderReadiness {
  level: ProviderReadinessLevel
  label: string
  detail: string
  nextStep: string
}

export interface FormValidation<T extends string> {
  errors: Partial<Record<T, string>>
  warnings: string[]
  valid: boolean
}

export type SettingsOperatorTab = 'service' | 'security' | 'limits' | 'providers' | 'operations'

const PROVIDER_ID_PATTERN = /^[a-z0-9_-]+$/
const ENV_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/
const CREDENTIAL_ID_PATTERN = /^[a-z0-9_-]+$/

export function providerReadiness(provider: Provider, isDefault = false): ProviderReadiness {
  if (provider.status === 'disabled') {
    return {
      level: 'disabled',
      label: '已停用',
      detail: '该 Provider 不参与模型解析或 fallback。',
      nextStep: '确认依赖后恢复，或迁移别名和策略再删除。',
    }
  }

  const credentialReady = provider.hasApiKey || !provider.apiKeyRequired
  if (!credentialReady) {
    return {
      level: 'blocked',
      label: '缺少凭证',
      detail: `运行进程未读取到 ${provider.apiKeyEnv || '所需 API Key'}。`,
      nextStep: '先配置环境变量并重启服务，再测试连接或发现模型。',
    }
  }

  if (!provider.defaultModel.trim() || provider.models.length === 0) {
    return {
      level: 'blocked',
      label: '没有可路由模型',
      detail: 'Provider 已配置，但模型目录尚未形成可用路由。',
      nextStep: '发现上游模型，或编辑 Provider 补充默认模型和模型列表。',
    }
  }

  if (provider.status !== 'active') {
    return {
      level: 'blocked',
      label: provider.status === 'error' ? '配置异常' : '尚未激活',
      detail: 'Provider 当前不参与实际模型解析。',
      nextStep: '检查 Provider 配置和运行日志，修复后重新测试连接。',
    }
  }

  const runtimeStatus = provider.runtimeStatus ?? provider.health?.status ?? 'healthy'
  if (runtimeStatus === 'cooldown') {
    return {
      level: 'attention',
      label: '冷却中',
      detail: provider.health?.lastError || '近期上游失败触发了临时冷却。',
      nextStep: provider.health?.recommendedAction || '检查上游状态与凭证，等待冷却结束后重新测试。',
    }
  }

  const rechargeRequired = Boolean(
    provider.health?.rechargeRequired
    || provider.credentials?.some((credential) => credential.health?.rechargeRequired),
  )
  if (
    runtimeStatus === 'degraded'
    || rechargeRequired
  ) {
    return {
      level: 'attention',
      label: rechargeRequired ? '需要处理账号' : '运行降级',
      detail: provider.health?.lastError || '近期请求成功率或账号状态需要关注。',
      nextStep: provider.health?.recommendedAction || '检查账号池、余额和最近错误后重新测试。',
    }
  }

  return {
    level: 'ready',
    label: isDefault ? '默认路由已配置' : '配置就绪',
    detail: `${provider.models.length} 个模型已启用，可通过 ${provider.id}:model 显式选择。`,
    nextStep: isDefault ? '未指定 Provider 的请求将按该默认值和 Provider 顺序解析；实际可用性仍以连接测试与请求日志为准。' : '可设为默认 Provider，或创建稳定别名供客户端使用。',
  }
}

export function validateProviderForm(
  form: ProviderFormState,
): FormValidation<keyof ProviderFormState & string> {
  const errors: Partial<Record<keyof ProviderFormState, string>> = {}
  const warnings: string[] = []
  const providerId = form.id.trim().toLowerCase()
  const baseUrl = form.baseUrl.trim()
  const defaultModel = form.defaultModel.trim()
  const models = parseList(form.models)
  const apiKeyEnv = form.apiKeyEnv.trim()

  if (!providerId) {
    errors.id = '请输入 Provider ID。'
  } else if (providerId.length > 80 || !PROVIDER_ID_PATTERN.test(providerId)) {
    errors.id = '仅支持小写字母、数字、短横线和下划线，最多 80 个字符。'
  }

  if (!baseUrl) {
    errors.baseUrl = '请输入 API Base URL。'
  } else {
    try {
      const url = new URL(baseUrl)
      if (!['http:', 'https:'].includes(url.protocol)) {
        errors.baseUrl = 'Base URL 必须使用 http:// 或 https://。'
      } else if (url.username || url.password || url.search || url.hash) {
        errors.baseUrl = 'Base URL 不能包含账号、密码、查询参数或片段。'
      } else {
        if (/\/(chat\/completions|messages)\/?$/i.test(url.pathname)) {
          warnings.push('Base URL 看起来是完整请求端点；应填写 API 根路径，而不是 /chat/completions 或 /messages。')
        }
        const localHost = ['localhost', '127.0.0.1', '::1', '[::1]', '0.0.0.0'].includes(url.hostname)
        if (url.protocol === 'http:' && !localHost) {
          warnings.push('远程 HTTP 会明文传输提示词与凭证；除可信内网外应使用 HTTPS。')
        }
      }
    } catch {
      errors.baseUrl = '请输入完整有效的 URL，例如 https://api.example.com/v1。'
    }
  }

  if (!defaultModel) {
    errors.defaultModel = '请输入默认模型。'
  } else if (defaultModel.length > 240) {
    errors.defaultModel = '默认模型最多 240 个字符。'
  }

  if (apiKeyEnv && (apiKeyEnv.length > 120 || !ENV_NAME_PATTERN.test(apiKeyEnv))) {
    errors.apiKeyEnv = '环境变量名须以字母或下划线开头，只能包含字母、数字和下划线。'
  }
  if (form.apiKeyRequired && !apiKeyEnv) {
    warnings.push('已要求 API Key，但未填写默认凭证变量；保存后 Provider 会保持不可路由，直到配置账号或环境变量。')
  }
  if (models.length === 0) {
    warnings.push('模型列表为空；保存时后端只会加入默认模型。')
  } else if (defaultModel && !models.includes(defaultModel)) {
    warnings.push('默认模型不在模型列表中；保存时后端会自动把它加入列表。')
  }
  if (form.fidelityMode === 'strict' && (form.deduplicateStreamText || form.bufferStreamText)) {
    errors.fidelityMode = '严格无损不能与流式文本去重或缓冲改写同时启用。'
  }
  if (form.protocol === 'anthropic' && form.toolStreamingArguments !== 'native') {
    warnings.push('原生 Anthropic Provider 通常应使用 native 工具参数流；请确认上游协议确实需要转换。')
  }

  return { errors, warnings, valid: Object.keys(errors).length === 0 }
}

export function validateCredentialForm(
  form: ProviderCredentialFormState,
  requireId: boolean,
): FormValidation<keyof ProviderCredentialFormState & string> {
  const errors: Partial<Record<keyof ProviderCredentialFormState, string>> = {}
  const warnings: string[] = []
  const id = form.id.trim().toLowerCase()
  const name = form.name.trim()
  const apiKeyEnv = form.apiKeyEnv.trim()
  const baseUrl = form.baseUrl.trim()

  if (requireId && (!id || id.length > 80 || !CREDENTIAL_ID_PATTERN.test(id))) {
    errors.id = '账号 ID 只能包含小写字母、数字、短横线和下划线，最多 80 个字符。'
  }
  if (!name || name.length > 120) {
    errors.name = '显示名称须为 1–120 个字符。'
  }
  if (!apiKeyEnv || apiKeyEnv.length > 120 || !ENV_NAME_PATTERN.test(apiKeyEnv)) {
    errors.apiKeyEnv = '请输入有效环境变量名；它不会保存真实 API Key。'
  }
  if (baseUrl) {
    try {
      const url = new URL(baseUrl)
      if (!['http:', 'https:'].includes(url.protocol)) {
        errors.baseUrl = '账号 Base URL 必须使用 http:// 或 https://。'
      } else if (url.username || url.password || url.search || url.hash) {
        errors.baseUrl = '账号 Base URL 不能包含账号、密码、查询参数或片段。'
      } else {
        const localHost = ['localhost', '127.0.0.1', '::1', '[::1]', '0.0.0.0'].includes(url.hostname)
        if (url.protocol === 'http:' && !localHost) {
          warnings.push('账号专用 Base URL 使用远程 HTTP；提示词与凭证可能被明文传输。')
        }
      }
    } catch {
      errors.baseUrl = '请输入有效 URL，或留空沿用 Provider。'
    }
  }

  return { errors, warnings, valid: Object.keys(errors).length === 0 }
}

export function validateAliasForm(alias: string, target: string) {
  const normalizedAlias = alias.trim()
  const normalizedTarget = target.trim()
  const errors: { alias?: string; target?: string } = {}
  if (!normalizedAlias || normalizedAlias.length > 120) {
    errors.alias = '别名须为 1–120 个字符。'
  } else if (normalizedAlias.includes(':')) {
    errors.alias = '别名不能包含 Provider 选择符“:”。'
  }
  if (!normalizedTarget || normalizedTarget.length > 240) {
    errors.target = '目标须为 1–240 个字符。'
  }
  return { errors, valid: Object.keys(errors).length === 0 }
}

export function settingsTabForCheck(checkId: string): SettingsOperatorTab {
  if (checkId === 'auth' || checkId === 'admin') return 'security'
  if (checkId === 'providers' || checkId === 'defaultProvider') return 'providers'
  if (checkId === 'persistence' || checkId === 'config') return 'operations'
  return 'service'
}
