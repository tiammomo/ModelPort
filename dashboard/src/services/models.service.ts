import type {
  ModelAlias,
  Provider,
  ProviderCredential,
  ProviderCredentialPoolMode,
  ProviderModelDiscovery,
  ProviderOnlineBalance,
  ProviderModelInventory,
  ProviderCredentialWritePayload,
  ProviderModelWritePayload,
  ProviderWritePayload,
  ProviderProtocol,
  ToolUseCapabilities,
} from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockAliases, mockProviders } from '@/mock'
import { settingsService } from './settings.service'

let mockAliasStore = [...mockAliases]
let mockProviderStore: Provider[] = mockProviders.map((provider) => ({
  ...provider,
  toolUse: provider.toolUse ?? defaultToolUse(provider.protocol, provider.deduplicateStreamText, provider.id),
  modelInventory: provider.modelInventory ?? provider.models.map((model): ProviderModelInventory => ({
    model,
    status: 'active',
    default: model === provider.defaultModel,
  })),
}))

export const modelsService = {
  getProviders: (): Promise<Provider[]> =>
    isMockMode ? mockDelay(mockProviderStore) : api.get('/admin/providers'),

  getProvider: async (id: string): Promise<Provider> => {
    const providers = isMockMode ? mockProviderStore : await api.get<Provider[]>('/admin/providers')
    const provider = providers.find((item) => item.id === id)
    if (!provider) throw new Error('提供商不存在')
    return isMockMode ? mockDelay(provider) : provider
  },

  createProvider: async (payload: ProviderWritePayload): Promise<Provider> => {
    if (!isMockMode) return api.post('/admin/providers', payload)
    const id = payload.id?.trim().toLowerCase()
    if (!id) throw new Error('供应商 ID 不能为空')
    const models = normalizedModels(payload.models, payload.defaultModel)
    const provider: Provider = {
      id,
      displayName: payload.displayName || id,
      source: 'control',
      protocol: payload.protocol || 'openai-compat',
      baseUrl: payload.baseUrl || '',
      apiKeyEnv: payload.clearApiKeyEnv ? null : payload.apiKeyEnv || null,
      apiKeyRequired: payload.apiKeyRequired ?? true,
      defaultModel: payload.defaultModel || models[0] || '',
      models,
      modelPrefixes: payload.modelPrefixes || [],
      passthroughUnknownModels: payload.passthroughUnknownModels ?? false,
      maxTokensField: payload.maxTokensField || 'max_completion_tokens',
      deduplicateStreamText: payload.deduplicateStreamText ?? false,
      bufferStreamText: payload.bufferStreamText ?? false,
      fidelityMode: payload.fidelityMode || 'best_effort',
      toolUse: payload.toolUse ?? defaultToolUse(payload.protocol || 'openai-compat', payload.deduplicateStreamText ?? false, id),
      status: payload.disabled ? 'disabled' : 'active',
      runtimeStatus: 'healthy',
      hasApiKey: true,
      credentialPoolMode: 'failover',
      health: null,
      lastTest: null,
      modelInventory: models.map((model) => ({ model, status: 'active', default: model === payload.defaultModel })),
    }
    mockProviderStore = [provider, ...mockProviderStore.filter((item) => item.id !== id)]
    return mockDelay(provider)
  },

  updateProvider: async (providerId: string, payload: ProviderWritePayload): Promise<Provider> => {
    if (!isMockMode) return api.put(`/admin/providers/${encodeURIComponent(providerId)}`, payload)
    const current = mockProviderStore.find((item) => item.id === providerId)
    if (!current) throw new Error('供应商不存在')
    const { clearApiKeyEnv, ...providerPatch } = payload
    const models = normalizedModels(payload.models ?? current.models, payload.defaultModel ?? current.defaultModel)
    const next: Provider = {
      ...current,
      ...providerPatch,
      id: providerId,
      displayName: payload.displayName ?? current.displayName,
      apiKeyEnv: clearApiKeyEnv ? null : payload.apiKeyEnv ?? current.apiKeyEnv,
      defaultModel: payload.defaultModel ?? current.defaultModel,
      models,
      modelPrefixes: payload.modelPrefixes ?? current.modelPrefixes,
      toolUse: payload.toolUse ?? current.toolUse,
      modelInventory: models.map((model) => ({ model, status: 'active', default: model === (payload.defaultModel ?? current.defaultModel) })),
      status: payload.disabled === undefined ? current.status : payload.disabled ? 'disabled' : 'active',
    }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  setProviderDisabled: async (providerId: string, disabled: boolean): Promise<Provider> => {
    if (!isMockMode) return api.post(`/admin/providers/${encodeURIComponent(providerId)}/disable`, { disabled })
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const next = { ...provider, status: disabled ? 'disabled' as const : 'active' as const }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  deleteProvider: async (providerId: string, force = false): Promise<void> => {
    if (!isMockMode) {
      await api.delete(`/admin/providers/${encodeURIComponent(providerId)}${force ? '?force=true' : ''}`)
      return
    }
    mockProviderStore = mockProviderStore.filter((item) => item.id !== providerId)
    mockAliasStore = mockAliasStore.filter((alias) => alias.resolvedProvider !== providerId)
    return mockDelay(undefined)
  },

  discoverProviderModels: async (providerId: string): Promise<ProviderModelDiscovery> => {
    if (!isMockMode) return api.post(`/admin/providers/${encodeURIComponent(providerId)}/models`)
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('提供商不存在')

    const models = provider.models.length > 0 ? provider.models : [provider.defaultModel]
    const discoveredAt = Date.now().toString()
    const message = `discovered ${models.length} model(s)`
    provider.lastTest = {
      testedAt: discoveredAt,
      success: true,
      message,
      models,
      modelCount: models.length,
    }

    return mockDelay({
      providerId,
      success: true,
      message,
      models,
      modelCount: models.length,
      discoveredAt,
    })
  },

  checkProviderBalance: async (providerId: string): Promise<ProviderOnlineBalance> => {
    if (!isMockMode) {
      return api.post(`/admin/providers/${encodeURIComponent(providerId)}/balance`)
    }
    if (providerId !== 'deepseek') throw new Error('仅 DeepSeek 官方 Provider 支持线上余额查询')
    return mockDelay({
      providerId,
      isAvailable: true,
      balanceInfos: [{
        currency: 'CNY',
        totalBalance: '100.00',
        grantedBalance: '0.00',
        toppedUpBalance: '100.00',
      }],
      checkedAt: Date.now().toString(),
      managementScope: 'read-monitor-alert',
      billingAuthority: 'deepseek-console',
    })
  },

  updateProviderModel: async (providerId: string, payload: ProviderModelWritePayload): Promise<Provider> => {
    if (!isMockMode) {
      const result = await api.put<{ provider: Provider }>(`/admin/providers/${encodeURIComponent(providerId)}/models`, payload)
      return result.provider
    }
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const active = payload.status !== 'disabled'
    const models = active
      ? Array.from(new Set([...provider.models, payload.model]))
      : provider.models.filter((model) => model !== payload.model)
    const inventory = provider.modelInventory?.filter((item) => item.model !== payload.model) ?? []
    inventory.push({
      model: payload.model,
      status: active ? 'active' : 'disabled',
      displayName: payload.displayName,
      family: payload.family,
      contextWindow: payload.contextWindow,
      default: payload.model === provider.defaultModel,
    })
    const next = { ...provider, models, modelInventory: inventory }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  createProviderCredential: async (providerId: string, payload: ProviderCredentialWritePayload): Promise<Provider> => {
    if (!isMockMode) {
      const result = await api.post<{ provider: Provider }>(`/admin/providers/${encodeURIComponent(providerId)}/credentials`, payload)
      return result.provider
    }
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const credential = mockCredential(providerId, payload, (provider.credentials?.length ?? 0) === 0)
    const credentials = [...(provider.credentials ?? []).filter((item) => item.id !== credential.id), credential]
    const activeCredentialId = mockActiveCredentialId(credentials, provider.activeCredentialId ?? credential.id)
    const next = {
      ...provider,
      credentials: credentials.map((item) => ({ ...item, active: item.id === activeCredentialId })),
      activeCredentialId,
    }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  updateProviderCredential: async (providerId: string, credentialId: string, payload: ProviderCredentialWritePayload): Promise<Provider> => {
    if (!isMockMode) {
      const result = await api.put<{ provider: Provider }>(`/admin/providers/${encodeURIComponent(providerId)}/credentials/${encodeURIComponent(credentialId)}`, payload)
      return result.provider
    }
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const credential = mockCredential(providerId, { ...payload, id: credentialId }, provider.activeCredentialId === credentialId)
    const credentials = [...(provider.credentials ?? []).filter((item) => item.id !== credentialId), credential]
    const activeCredentialId = mockActiveCredentialId(credentials, provider.activeCredentialId)
    const next = {
      ...provider,
      activeCredentialId,
      credentials: credentials.map((item) => ({ ...item, active: item.id === activeCredentialId })),
    }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  selectProviderCredential: async (providerId: string, credentialId: string): Promise<Provider> => {
    if (!isMockMode) {
      const result = await api.post<{ provider: Provider }>(`/admin/providers/${encodeURIComponent(providerId)}/credentials/${encodeURIComponent(credentialId)}/select`)
      return result.provider
    }
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const selected = provider.credentials?.find((item) => item.id === credentialId)
    if (!selected) throw new Error('账号不存在')
    if (selected?.status === 'disabled') throw new Error('禁用账号不能被选中')
    const next = {
      ...provider,
      activeCredentialId: credentialId,
      credentials: (provider.credentials ?? []).map((item) => ({ ...item, active: item.id === credentialId })),
    }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  updateProviderCredentialPoolMode: async (providerId: string, mode: ProviderCredentialPoolMode): Promise<Provider> => {
    if (!isMockMode) {
      const result = await api.put<{ provider: Provider }>(`/admin/providers/${encodeURIComponent(providerId)}/credential-pool`, { mode })
      return result.provider
    }
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const next = {
      ...provider,
      credentialPoolMode: mode,
    }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  deleteProviderCredential: async (providerId: string, credentialId: string): Promise<Provider> => {
    if (!isMockMode) {
      const result = await api.delete<{ provider: Provider }>(`/admin/providers/${encodeURIComponent(providerId)}/credentials/${encodeURIComponent(credentialId)}`)
      return result.provider
    }
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const credentials = (provider.credentials ?? []).filter((item) => item.id !== credentialId)
    const activeCredentialId = mockActiveCredentialId(credentials, provider.activeCredentialId === credentialId ? null : provider.activeCredentialId)
    const next = {
      ...provider,
      activeCredentialId,
      credentials: credentials.map((item) => ({ ...item, active: item.id === activeCredentialId })),
    }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  toggleModel: async (providerId: string, model: string, enabled: boolean): Promise<Provider> =>
    modelsService.updateProviderModel(providerId, { model, status: enabled ? 'active' : 'disabled' }),

  updateDefaultModel: async (providerId: string, model: string): Promise<Provider> => {
    if (!isMockMode) return api.put(`/admin/providers/${encodeURIComponent(providerId)}`, { defaultModel: model })
    const provider = mockProviderStore.find((item) => item.id === providerId)
    if (!provider) throw new Error('供应商不存在')
    const models = normalizedModels(provider.models, model)
    const next = {
      ...provider,
      defaultModel: model,
      models,
      modelInventory: (provider.modelInventory ?? []).map((item) => ({ ...item, default: item.model === model })),
    }
    mockProviderStore = mockProviderStore.map((item) => item.id === providerId ? next : item)
    return mockDelay(next)
  },

  getAliases: (): Promise<ModelAlias[]> =>
    isMockMode ? mockDelay(mockAliasStore) : api.get('/admin/aliases'),

  createAlias: (alias: Omit<ModelAlias, 'resolvedProvider' | 'resolvedModel'>): Promise<ModelAlias> => {
    if (!isMockMode) return api.post('/admin/aliases', alias)
    const [providerId = '', resolvedModel = alias.target] = alias.target.includes(':')
      ? alias.target.split(/:(.*)/s)
      : ['', alias.target]
    const row = { ...alias, resolvedProvider: providerId, resolvedModel }
    mockAliasStore = [row, ...mockAliasStore.filter((item) => item.alias !== alias.alias)]
    return mockDelay(row)
  },

  deleteAlias: (alias: string): Promise<void> => {
    if (!isMockMode) return api.delete(`/admin/aliases/${encodeURIComponent(alias)}`)
    mockAliasStore = mockAliasStore.filter((item) => item.alias !== alias)
    return mockDelay(undefined)
  },

  updateProviderOrder: async (order: string[]): Promise<void> => {
    await settingsService.updateProviderOrder(order)
  },

  updateDefaultProvider: (providerId: string): Promise<void> =>
    settingsService.updateDefaultProvider(providerId),
}

function mockCredential(providerId: string, payload: ProviderCredentialWritePayload, active = false): ProviderCredential {
  const id = payload.id?.trim() || payload.name.trim().toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-|-$/g, '') || `account-${Date.now()}`
  return {
    id,
    providerId,
    name: payload.name,
    apiKeyEnv: payload.apiKeyEnv,
    baseUrl: payload.baseUrl,
    status: payload.status ?? 'active',
    active,
    hasApiKey: true,
    health: null,
    createdAt: Date.now().toString(),
    updatedAt: Date.now().toString(),
  }
}

function mockActiveCredentialId(credentials: ProviderCredential[], preferred?: string | null): string | null {
  if (preferred && credentials.some((item) => item.id === preferred && item.status !== 'disabled')) {
    return preferred
  }
  return credentials.find((item) => item.status !== 'disabled')?.id ?? null
}

function normalizedModels(models: string[] = [], defaultModel?: string): string[] {
  const rows = Array.from(new Set(models.map((model) => model.trim()).filter(Boolean)))
  const normalizedDefault = defaultModel?.trim()
  if (normalizedDefault && !rows.includes(normalizedDefault)) rows.unshift(normalizedDefault)
  return rows
}

function defaultToolUse(protocol: ProviderProtocol, deduplicateStreamText: boolean, providerId: string): ToolUseCapabilities {
  return {
    supported: true,
    toolChoice: true,
    parallelToolCalls: !['ollama', 'local_sglang', 'local_vllm', 'local_llamacpp'].includes(providerId),
    streamingArguments: protocol === 'anthropic'
      ? 'native'
      : deduplicateStreamText
        ? 'cumulative'
        : ['custom', 'ollama', 'local_sglang', 'local_vllm', 'local_llamacpp'].includes(providerId)
          ? 'best_effort'
          : 'delta',
    responseValidation: 'best_effort',
  }
}
