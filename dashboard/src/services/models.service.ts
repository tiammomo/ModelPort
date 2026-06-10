import type { Provider, ModelAlias } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockAliases, mockProviders } from '@/mock'

let mockAliasStore = [...mockAliases]

export const modelsService = {
  getProviders: (): Promise<Provider[]> =>
    isMockMode ? mockDelay(mockProviders) : api.get('/admin/providers'),

  getProvider: async (id: string): Promise<Provider> => {
    const providers = isMockMode ? mockProviders : await api.get<Provider[]>('/admin/providers')
    const provider = providers.find((item) => item.id === id)
    if (!provider) throw new Error('提供商不存在')
    return isMockMode ? mockDelay(provider) : provider
  },

  toggleModel: async (providerId: string, model: string, enabled: boolean): Promise<void> => {
    void providerId
    void model
    void enabled
  },

  updateDefaultModel: async (providerId: string, model: string): Promise<void> => {
    void providerId
    void model
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
    if (isMockMode) return mockDelay(undefined)
    await api.put('/admin/settings', { gateway: { providerOrder: order } })
  },

  updateDefaultProvider: async (providerId: string): Promise<void> => {
    if (isMockMode) return mockDelay(undefined)
    await api.put('/admin/settings', { gateway: { defaultProvider: providerId } })
  },
}
