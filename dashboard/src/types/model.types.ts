export type ProviderProtocol = 'anthropic' | 'openai-compat'
export type MaxTokensField = 'max_completion_tokens' | 'max_tokens' | 'both'
export type FidelityMode = 'strict' | 'best_effort' | 'stability'
export type ProviderStatus = 'active' | 'inactive' | 'disabled' | 'error'
export type ProviderModelStatus = 'active' | 'disabled'

export interface ProviderModelInventory {
  providerId?: string
  model: string
  status: ProviderModelStatus
  displayName?: string | null
  family?: string | null
  contextWindow?: number | null
  default?: boolean
  createdAt?: string
  updatedAt?: string
}

export interface Provider {
  id: string
  displayName: string
  source?: 'config' | 'control'
  protocol: ProviderProtocol
  baseUrl: string
  apiKeyEnv: string | null
  apiKeyRequired: boolean
  defaultModel: string
  models: string[]
  modelPrefixes: string[]
  passthroughUnknownModels: boolean
  maxTokensField: MaxTokensField
  deduplicateStreamText: boolean
  bufferStreamText: boolean
  fidelityMode?: FidelityMode
  status: ProviderStatus
  runtimeStatus?: 'healthy' | 'degraded' | 'cooldown'
  hasApiKey: boolean
  health?: {
    providerId: string
    requestsTotal: number
    successesTotal: number
    failuresTotal: number
    consecutiveFailures: number
    successRate: number
    status: 'healthy' | 'degraded' | 'cooldown'
    lastSuccessAt?: string | null
    lastFailureAt?: string | null
    cooldownUntil?: string | null
    lastError?: string | null
  } | null
  lastTest?: {
    testedAt: string
    success: boolean
    message: string
    models?: string[]
    modelCount?: number
  } | null
  modelInventory?: ProviderModelInventory[]
}

export interface ProviderWritePayload {
  id?: string
  displayName?: string
  protocol?: ProviderProtocol
  baseUrl?: string
  apiKeyEnv?: string | null
  apiKeyRequired?: boolean
  defaultModel?: string
  models?: string[]
  modelPrefixes?: string[]
  passthroughUnknownModels?: boolean
  maxTokensField?: MaxTokensField
  deduplicateStreamText?: boolean
  bufferStreamText?: boolean
  fidelityMode?: FidelityMode
  disabled?: boolean
}

export interface ProviderModelWritePayload {
  model: string
  status?: ProviderModelStatus
  displayName?: string | null
  family?: string | null
  contextWindow?: number | null
}

export interface ProviderDeleteDependency {
  type: string
  id?: string
  name?: string
  field?: string
  target?: string
}

export interface ProviderDeleteBlocked {
  ok: false
  blocked: true
  providerId: string
  message: string
  dependencies: ProviderDeleteDependency[]
}

export interface ProviderModelDiscovery {
  providerId: string
  success: boolean
  message: string
  models: string[]
  modelCount: number
  discoveredAt: string
}

export interface ModelAlias {
  alias: string
  target: string
  resolvedProvider: string
  resolvedModel: string
}

export interface ModelInfo {
  id: string
  type: 'model'
  displayName: string
  providerId: string
  enabled: boolean
}
