import { ApiError } from '@/lib/api-client'
import { guessModelFamily } from '@/lib/model-catalog'
import type {
  FidelityMode,
  MaxTokensField,
  Provider,
  ProviderCredential,
  ProviderCredentialPoolMode,
  ProviderCredentialWritePayload,
  ProviderDeleteBlocked,
  ProviderModelInventory,
  ProviderProtocol,
  ProviderWritePayload,
  ToolStreamingArguments,
} from '@/types'

export interface ProviderFormState {
  id: string
  displayName: string
  protocol: ProviderProtocol
  baseUrl: string
  apiKeyEnv: string
  apiKeyRequired: boolean
  defaultModel: string
  models: string
  modelPrefixes: string
  passthroughUnknownModels: boolean
  maxTokensField: MaxTokensField
  deduplicateStreamText: boolean
  bufferStreamText: boolean
  fidelityMode: FidelityMode
  toolUseSupported: boolean
  toolChoice: boolean
  parallelToolCalls: boolean
  toolStreamingArguments: ToolStreamingArguments
  disabled: boolean
}

export interface ProviderCredentialFormState {
  id: string
  name: string
  apiKeyEnv: string
  baseUrl: string
  status: 'active' | 'disabled'
}

export interface ProviderInventoryGroup {
  title: string
  brand: string
  originClassName: string
  items: ProviderModelInventory[]
}

export type ProviderOperationalFilter = 'all' | 'healthy' | 'degraded' | 'recharge'

export const PROVIDER_OPERATIONAL_FILTERS: Array<{
  value: ProviderOperationalFilter
  label: string
}> = [
  { value: 'all', label: '全部' },
  { value: 'healthy', label: '健康' },
  { value: 'degraded', label: '异常' },
  { value: 'recharge', label: '等待充值' },
]

export const DEFAULT_PROVIDER_FORM: ProviderFormState = {
  id: '',
  displayName: '',
  protocol: 'openai-compat',
  baseUrl: '',
  apiKeyEnv: '',
  apiKeyRequired: true,
  defaultModel: '',
  models: '',
  modelPrefixes: '',
  passthroughUnknownModels: false,
  maxTokensField: 'max_completion_tokens',
  deduplicateStreamText: false,
  bufferStreamText: false,
  fidelityMode: 'best_effort',
  toolUseSupported: true,
  toolChoice: true,
  parallelToolCalls: true,
  toolStreamingArguments: 'delta',
  disabled: false,
}

export const DEFAULT_CREDENTIAL_FORM: ProviderCredentialFormState = {
  id: '',
  name: '',
  apiKeyEnv: '',
  baseUrl: '',
  status: 'active',
}

export const CREDENTIAL_POOL_MODE_LABELS: Record<ProviderCredentialPoolMode, string> = {
  manual: '手动',
  failover: '故障切换',
  round_robin: '轮询',
}

const PROVIDER_BRAND_NAMES: Record<string, string> = {
  deepseek: 'DeepSeek',
  deepseek_openai: 'DeepSeek',
  mimo: '小米 MiMo',
  openai: 'OpenAI',
  anthropic: 'Anthropic Claude',
  openrouter: 'OpenRouter',
  gemini: 'Google Gemini',
  dashscope: '阿里云百炼 Qwen',
  kimi: 'Moonshot Kimi',
  zhipu: '智谱 GLM',
  xai: 'xAI Grok',
  groq: 'Groq',
  mistral: 'Mistral AI',
  ark: '火山方舟 Doubao',
  ollama: 'Ollama',
  sglang: 'SGLang',
  vllm: 'vLLM',
  llamacpp: 'llama.cpp',
}

const OFFICIAL_PROVIDER_HOSTS: Record<string, string[]> = {
  deepseek: ['api.deepseek.com'],
  deepseek_openai: ['api.deepseek.com'],
  mimo: ['api.xiaomimimo.com'],
  openai: ['api.openai.com'],
  anthropic: ['api.anthropic.com'],
  gemini: ['generativelanguage.googleapis.com'],
  dashscope: ['dashscope.aliyuncs.com'],
  kimi: ['api.moonshot.cn'],
  zhipu: ['open.bigmodel.cn'],
  xai: ['api.x.ai'],
  groq: ['api.groq.com'],
  mistral: ['api.mistral.ai'],
  ark: ['ark.cn-beijing.volces.com'],
}

const LOCAL_PROVIDER_IDS = new Set(['ollama', 'local_sglang', 'local_vllm', 'local_llamacpp'])
const AGGREGATOR_PROVIDER_IDS = new Set(['openrouter'])
const MODEL_FAMILY_BRAND_NAMES: Record<string, string> = {
  OpenAI: 'OpenAI',
  Claude: 'Anthropic Claude',
  DeepSeek: 'DeepSeek',
  Gemini: 'Google Gemini',
  Qwen: 'Qwen',
  Kimi: 'Moonshot Kimi',
  GLM: '智谱 GLM',
  Grok: 'xAI Grok',
  Llama: 'Llama',
  Mistral: 'Mistral AI',
  Doubao: 'Doubao',
  Mimo: '小米 MiMo',
  Local: '本地模型',
  Custom: '自定义模型',
}

export function providerDisplayTitle(provider: Provider): string {
  const identity = providerIdentity(provider)
  const groups = providerModelGroups(provider)
  if (groups.length > 1) return `${identity.origin} · 多模型渠道`
  if (groups.length === 1) return groups[0].title
  return `${identity.origin} · ${identity.brand}`
}

export function providerIdentity(provider: Provider) {
  const origin = providerOrigin(provider)
  return {
    origin,
    brand: PROVIDER_BRAND_NAMES[provider.id] ?? compactProviderName(provider.displayName),
    originClassName: providerOriginClassName(origin),
  }
}

export function modelRouteTitle(provider: Provider, model: string): string {
  return `${providerOrigin(provider)} · ${modelOwnerBrand(model)}`
}

export function modelOwnerBrand(model: string): string {
  const family = guessModelFamily(model)
  return MODEL_FAMILY_BRAND_NAMES[family] ?? family
}

export function providerModelGroups(provider: Provider) {
  const groups = new Map<string, { title: string; brand: string; originClassName: string; models: string[] }>()
  const origin = providerOrigin(provider)
  const originClassName = providerOriginClassName(origin)

  for (const model of provider.models) {
    const brand = modelOwnerBrand(model)
    const title = `${origin} · ${brand}`
    const group = groups.get(title) ?? { title, brand, originClassName, models: [] }
    group.models.push(model)
    groups.set(title, group)
  }

  return Array.from(groups.values())
    .sort((left, right) => right.models.length - left.models.length || left.brand.localeCompare(right.brand))
}

export function providerInventoryGroups(provider: Provider): ProviderInventoryGroup[] {
  const groups = new Map<string, ProviderInventoryGroup>()
  const origin = providerOrigin(provider)
  const originClassName = providerOriginClassName(origin)

  for (const item of providerInventoryItems(provider)) {
    const brand = item.family || modelOwnerBrand(item.model)
    const title = `${origin} · ${brand}`
    const group = groups.get(title) ?? { title, brand, originClassName, items: [] }
    group.items.push(item)
    groups.set(title, group)
  }

  return Array.from(groups.values())
    .sort((left, right) => right.items.length - left.items.length || left.brand.localeCompare(right.brand))
}

export function providerInventoryItems(provider: Provider): ProviderModelInventory[] {
  const inventory = provider.modelInventory && provider.modelInventory.length > 0
    ? provider.modelInventory
    : provider.models.map((model): ProviderModelInventory => ({
        model,
        status: 'active',
        default: model === provider.defaultModel,
      }))

  return [...inventory].sort((left, right) => {
    const leftDefault = left.model === provider.defaultModel ? 0 : 1
    const rightDefault = right.model === provider.defaultModel ? 0 : 1
    if (leftDefault !== rightDefault) return leftDefault - rightDefault
    if (left.status !== right.status) return left.status === 'active' ? -1 : 1
    return left.model.localeCompare(right.model)
  })
}

export function providerOrigin(provider: Provider): string {
  const host = providerHost(provider)
  if (LOCAL_PROVIDER_IDS.has(provider.id) || isLocalHost(host)) return '本地'
  if (provider.id === 'custom') return '自定义'
  if (AGGREGATOR_PROVIDER_IDS.has(provider.id)) return '聚合平台'
  if ((OFFICIAL_PROVIDER_HOSTS[provider.id] || []).some((officialHost) => hostMatches(host, officialHost))) {
    return '官方'
  }
  return '第三方'
}

export function providerOriginClassName(origin: string): string {
  if (origin === '官方') return 'border-emerald-200 bg-emerald-50 text-emerald-700'
  if (origin === '第三方') return 'border-amber-200 bg-amber-50 text-amber-700'
  if (origin === '本地') return 'border-sky-200 bg-sky-50 text-sky-700'
  if (origin === '聚合平台') return 'border-violet-200 bg-violet-50 text-violet-700'
  return 'border-slate-200 bg-slate-50 text-slate-700'
}

function providerHost(provider: Provider): string {
  try {
    return new URL(provider.baseUrl).hostname.toLowerCase().replace(/^www\./, '')
  } catch {
    return ''
  }
}

function hostMatches(host: string, expected: string): boolean {
  return host === expected || host.endsWith(`.${expected}`)
}

function isLocalHost(host: string): boolean {
  return host === 'localhost' || host === '127.0.0.1' || host === '0.0.0.0' || host === '::1'
}

function compactProviderName(value: string): string {
  return value
    .replace(/\bOfficial\b/gi, '')
    .replace(/\bOpenAI[- ]Compatible\b/gi, 'OpenAI 兼容')
    .replace(/\s+/g, ' ')
    .trim()
}

export function providerToForm(provider: Provider): ProviderFormState {
  const toolUse = provider.toolUse ?? defaultToolUseForProviderForm(
    provider.id,
    provider.protocol,
    provider.deduplicateStreamText,
  )

  return {
    id: provider.id,
    displayName: provider.displayName,
    protocol: provider.protocol,
    baseUrl: provider.baseUrl,
    apiKeyEnv: provider.apiKeyEnv || '',
    apiKeyRequired: provider.apiKeyRequired,
    defaultModel: provider.defaultModel,
    models: provider.models.join('\n'),
    modelPrefixes: provider.modelPrefixes.join(', '),
    passthroughUnknownModels: provider.passthroughUnknownModels,
    maxTokensField: provider.maxTokensField,
    deduplicateStreamText: provider.deduplicateStreamText,
    bufferStreamText: provider.bufferStreamText,
    fidelityMode: provider.fidelityMode || 'best_effort',
    toolUseSupported: toolUse.supported,
    toolChoice: toolUse.toolChoice,
    parallelToolCalls: toolUse.parallelToolCalls,
    toolStreamingArguments: toolUse.streamingArguments,
    disabled: provider.status === 'disabled',
  }
}

export function credentialToForm(
  provider: Provider,
  credential?: ProviderCredential,
): ProviderCredentialFormState {
  if (!credential) {
    return {
      ...DEFAULT_CREDENTIAL_FORM,
      apiKeyEnv: provider.apiKeyEnv ? `${provider.apiKeyEnv}_ALT` : '',
    }
  }
  return {
    id: credential.id,
    name: credential.name,
    apiKeyEnv: credential.apiKeyEnv,
    baseUrl: credential.baseUrl || '',
    status: credential.status,
  }
}

export function providerPayloadFromForm(
  form: ProviderFormState,
  includeId: boolean,
): ProviderWritePayload {
  const apiKeyEnv = form.apiKeyEnv.trim()
  return {
    ...(includeId ? { id: form.id.trim() } : {}),
    displayName: form.displayName.trim() || form.id.trim(),
    protocol: form.protocol,
    baseUrl: form.baseUrl.trim(),
    ...(apiKeyEnv ? { apiKeyEnv } : { clearApiKeyEnv: true }),
    apiKeyRequired: form.apiKeyRequired,
    defaultModel: form.defaultModel.trim(),
    models: parseList(form.models),
    modelPrefixes: parseList(form.modelPrefixes),
    passthroughUnknownModels: form.passthroughUnknownModels,
    maxTokensField: form.maxTokensField,
    deduplicateStreamText: form.deduplicateStreamText,
    bufferStreamText: form.bufferStreamText,
    fidelityMode: form.fidelityMode,
    toolUse: {
      supported: form.toolUseSupported,
      toolChoice: form.toolChoice,
      parallelToolCalls: form.parallelToolCalls,
      streamingArguments: form.toolStreamingArguments,
    },
    disabled: form.disabled,
  }
}

export function credentialPayloadFromForm(
  form: ProviderCredentialFormState,
  includeId: boolean,
): ProviderCredentialWritePayload {
  return {
    ...(includeId && form.id.trim() ? { id: form.id.trim() } : {}),
    name: form.name.trim(),
    apiKeyEnv: form.apiKeyEnv.trim(),
    baseUrl: form.baseUrl.trim() || null,
    status: form.status,
  }
}

export function parseList(value: string): string[] {
  return Array.from(new Set(
    value
      .split(/[\n,]/)
      .map((item) => item.trim())
      .filter(Boolean),
  ))
}

export function defaultToolUseForProviderForm(
  providerId: string,
  protocol: ProviderProtocol,
  deduplicateStreamText: boolean,
): NonNullable<Provider['toolUse']> {
  return {
    supported: true,
    toolChoice: true,
    parallelToolCalls: !LOCAL_PROVIDER_IDS.has(providerId),
    streamingArguments: defaultToolStreamingArguments(protocol, deduplicateStreamText, providerId),
  }
}

export function defaultToolStreamingArguments(
  protocol: ProviderProtocol,
  deduplicateStreamText: boolean,
  providerId: string,
): ToolStreamingArguments {
  if (protocol === 'anthropic') return 'native'
  if (deduplicateStreamText) return 'cumulative'
  if (LOCAL_PROVIDER_IDS.has(providerId) || providerId === 'custom') return 'best_effort'
  return 'delta'
}

export function providerNeedsRecharge(provider: Provider): boolean {
  return Boolean(
    provider.health?.rechargeRequired
    || provider.credentials?.some((credential) => credential.health?.rechargeRequired),
  )
}

export function providerRuntimeState(provider: Provider): 'healthy' | 'degraded' | 'cooldown' {
  return provider.runtimeStatus || provider.health?.status || 'healthy'
}

export function providerIsHealthy(provider: Provider): boolean {
  return provider.status === 'active'
    && providerRuntimeState(provider) === 'healthy'
    && !providerNeedsRecharge(provider)
}

export function providerIsDegraded(provider: Provider): boolean {
  return provider.status !== 'active'
    || providerRuntimeState(provider) !== 'healthy'
    || providerNeedsRecharge(provider)
}

export function providerFilterCount(
  filter: ProviderOperationalFilter,
  providers: Provider[],
  rechargeProviders: Provider[],
  degradedProviders: Provider[],
): number {
  if (filter === 'recharge') return rechargeProviders.length
  if (filter === 'healthy') return providers.filter(providerIsHealthy).length
  if (filter === 'degraded') return degradedProviders.length
  return providers.length
}

export function providerDeleteBlockedFromError(error: unknown): ProviderDeleteBlocked | null {
  if (!(error instanceof ApiError) || error.status !== 409) return null
  const payload = error.payload as Partial<ProviderDeleteBlocked> | undefined
  if (!payload?.blocked || !Array.isArray(payload.dependencies)) return null
  return payload as ProviderDeleteBlocked
}

export function dependencyLabel(type: string): string {
  if (type === 'alias') return '模型别名'
  if (type === 'apiKey') return 'API 密钥'
  if (type === 'team') return '团队策略'
  if (type === 'defaultProvider') return '默认 Provider'
  if (type === 'providerOrder') return 'Provider 顺序'
  if (type === 'route') return '路由配置'
  return type
}
