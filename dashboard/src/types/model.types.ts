export type ProviderProtocol = 'anthropic' | 'openai-compat'
export type MaxTokensField = 'max_completion_tokens' | 'max_tokens' | 'both'
export type FidelityMode = 'strict' | 'best_effort' | 'stability'
export type ToolStreamingArguments = 'native' | 'delta' | 'cumulative' | 'best_effort'
export type ProviderStatus = 'active' | 'inactive' | 'disabled' | 'error'
export type ProviderModelStatus = 'active' | 'disabled'
export type ProviderCredentialPoolMode = 'manual' | 'failover' | 'round_robin'

export interface ToolUseCapabilities {
  supported: boolean
  toolChoice: boolean
  parallelToolCalls: boolean
  streamingArguments: ToolStreamingArguments
}

export interface ProviderHealth {
  providerId: string
  credentialId?: string
  requestsTotal: number
  successesTotal: number
  failuresTotal: number
  consecutiveFailures: number
  successRate: number
  status: 'healthy' | 'degraded' | 'cooldown'
  lastSuccessAt?: string | null
  lastFailureAt?: string | null
  lastUsedAt?: string | null
  cooldownUntil?: string | null
  lastError?: string | null
  lastStatusCode?: number | null
  failureKind?: 'none' | 'account' | 'rate_limit' | 'upstream_unavailable' | 'config' | 'unknown'
  accountIssue?: 'none' | 'insufficient_balance' | 'auth'
  rechargeRequired?: boolean
  rechargeBadge?: string | null
  recommendedAction?: string | null
}

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

export interface ProviderCredential {
  id: string
  providerId: string
  name: string
  apiKeyEnv: string
  baseUrl?: string | null
  status: 'active' | 'disabled'
  active: boolean
  hasApiKey: boolean
  health?: ProviderHealth | null
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
  toolUse?: ToolUseCapabilities
  status: ProviderStatus
  credentials?: ProviderCredential[]
  activeCredentialId?: string | null
  credentialPoolMode?: ProviderCredentialPoolMode
  runtimeStatus?: 'healthy' | 'degraded' | 'cooldown'
  hasApiKey: boolean
  health?: ProviderHealth | null
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
  apiKeyEnv?: string
  clearApiKeyEnv?: boolean
  apiKeyRequired?: boolean
  defaultModel?: string
  models?: string[]
  modelPrefixes?: string[]
  passthroughUnknownModels?: boolean
  maxTokensField?: MaxTokensField
  deduplicateStreamText?: boolean
  bufferStreamText?: boolean
  fidelityMode?: FidelityMode
  toolUse?: ToolUseCapabilities
  disabled?: boolean
}

export interface ProviderModelWritePayload {
  model: string
  status?: ProviderModelStatus
  displayName?: string | null
  family?: string | null
  contextWindow?: number | null
}

export interface ProviderCredentialWritePayload {
  id?: string
  name: string
  apiKeyEnv: string
  baseUrl?: string | null
  status?: 'active' | 'disabled'
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
