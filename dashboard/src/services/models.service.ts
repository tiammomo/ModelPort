import type { Provider, ModelAlias } from '@/types'
import { api } from '@/lib/api-client'

export const modelsService = {
  getProviders: (): Promise<Provider[]> => api.get('/admin/providers'),

  getProvider: async (id: string): Promise<Provider> => {
    const providers = await api.get<Provider[]>('/admin/providers')
    const provider = providers.find((item) => item.id === id)
    if (!provider) throw new Error('提供商不存在')
    return provider
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

  getAliases: (): Promise<ModelAlias[]> => api.get('/admin/aliases'),

  createAlias: (alias: Omit<ModelAlias, 'resolvedProvider' | 'resolvedModel'>): Promise<ModelAlias> =>
    api.post('/admin/aliases', alias),

  deleteAlias: (alias: string): Promise<void> => api.delete(`/admin/aliases/${encodeURIComponent(alias)}`),

  updateProviderOrder: async (order: string[]): Promise<void> => {
    void order
  },

  updateDefaultProvider: async (providerId: string): Promise<void> => {
    void providerId
  },
}
