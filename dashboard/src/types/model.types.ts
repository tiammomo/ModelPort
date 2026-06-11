export type ProviderProtocol = 'anthropic' | 'openai-compat'
export type MaxTokensField = 'max_completion_tokens' | 'max_tokens' | 'both'
export type FidelityMode = 'strict' | 'best_effort' | 'stability'

export interface Provider {
  id: string
  displayName: string
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
  status: 'active' | 'inactive' | 'error'
  hasApiKey: boolean
  lastTest?: {
    testedAt: string
    success: boolean
    message: string
    models?: string[]
    modelCount?: number
  } | null
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
