import type { SystemSettings } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import { mockProviders, mockSettings } from '@/mock'

let mockSettingsStore = mockSettings

export const settingsService = {
  getSettings: (): Promise<SystemSettings> =>
    isMockMode ? mockDelay(mockSettingsStore) : api.get('/admin/settings'),

  updateSettings: (settings: Partial<SystemSettings>): Promise<SystemSettings> => {
    if (!isMockMode) return api.put('/admin/settings', settings)
    mockSettingsStore = { ...mockSettingsStore, ...settings }
    return mockDelay(mockSettingsStore)
  },

  testProviderConnection: (providerId: string): Promise<{ success: boolean; message: string; testedAt?: string }> => {
    if (!isMockMode) return api.post('/admin/settings/test-provider', { providerId })
    const provider = mockProviders.find((item) => item.id === providerId)
    if (!provider) return mockDelay({ success: false, message: 'provider not found' }, 220)
    const success = provider.status === 'active' && (provider.hasApiKey || !provider.apiKeyRequired)
    return mockDelay({
      success,
      message: success ? 'mock connection ok' : 'mock missing API key or provider inactive',
      testedAt: new Date().toISOString(),
    }, 220)
  },
}
