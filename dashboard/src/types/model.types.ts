export type ProviderProtocol = 'anthropic' | 'openai-compat'
export type MaxTokensField = 'max_completion_tokens' | 'max_tokens' | 'both'

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
  status: 'active' | 'inactive' | 'error'
  hasApiKey: boolean
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
