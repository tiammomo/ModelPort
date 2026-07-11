import type { AuditEventsResponse, BackupExport, ConfigReloadResult, SystemSettings } from '@/types'
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

  updateDefaultProvider: async (providerId: string): Promise<void> => {
    if (!isMockMode) {
      await api.put('/admin/settings', { gateway: { defaultProvider: providerId } })
      return
    }
    mockSettingsStore = {
      ...mockSettingsStore,
      gateway: {
        ...mockSettingsStore.gateway,
        defaultProvider: providerId,
      },
    }
    await mockDelay(undefined)
  },

  testProviderConnection: (providerId: string): Promise<{ success: boolean; message: string; testedAt?: string; models?: string[]; modelCount?: number }> => {
    if (!isMockMode) return api.post('/admin/settings/test-provider', { providerId })
    const provider = mockProviders.find((item) => item.id === providerId)
    if (!provider) return mockDelay({ success: false, message: 'provider not found' }, 220)
    const success = provider.status === 'active' && (provider.hasApiKey || !provider.apiKeyRequired)
    const models = success ? provider.models : []
    return mockDelay({
      success,
      message: success ? 'mock connection ok' : 'mock missing API key or provider inactive',
      models,
      modelCount: models.length,
      testedAt: new Date().toISOString(),
    }, 220)
  },

  reloadConfig: (): Promise<ConfigReloadResult> => {
    if (!isMockMode) return api.post('/admin/settings/reload-config')
    return mockDelay({
      ok: true,
      settings: mockSettingsStore,
      providerCount: mockSettingsStore.gateway.providerOrder.length,
      defaultProvider: mockSettingsStore.gateway.defaultProvider,
      providerOrder: mockSettingsStore.gateway.providerOrder,
      issues: mockSettingsStore.setup?.issues ?? [],
      reloadScope: {
        applied: ['providers', 'provider credentials', 'base urls', 'model lists', 'aliases', 'legacy client auth token'],
        requiresRestart: ['bind address', 'request body limit', 'concurrency layer', 'HTTP client timeouts', 'trusted proxies', 'admin bootstrap account'],
      },
    }, 220)
  },

  getAuditEvents: (): Promise<AuditEventsResponse> => {
    if (!isMockMode) return api.get('/admin/audit')
    return mockDelay({
      total: 3,
      events: [
        {
          id: 'act_mock_login',
          timestamp: Date.now().toString(),
          type: 'config_change',
          actor: 'admin',
          target: 'user:admin',
          message: '管理员 admin 登录控制台',
          severity: 'info',
        },
        {
          id: 'act_mock_provider',
          timestamp: (Date.now() - 3600000).toString(),
          type: 'config_change',
          actor: 'admin',
          target: 'provider:mimo',
          message: '测试供应商 mimo: connected',
          severity: 'info',
        },
        {
          id: 'act_mock_key',
          timestamp: (Date.now() - 7200000).toString(),
          type: 'config_change',
          actor: 'alice',
          target: 'api_key:key_mock',
          message: '更新 API Key alice-dev (active)',
          severity: 'warning',
        },
      ],
    }, 180)
  },

  exportBackup: (): Promise<BackupExport> => {
    if (!isMockMode) return api.post('/admin/backup')
    return mockDelay({
      schemaVersion: 1,
      service: 'model-port',
      generatedAt: Date.now().toString(),
      containsSecrets: false,
      containsPersonalData: true,
      settings: mockSettingsStore,
      users: [],
      control: {
        apiKeys: [],
        quotas: [],
        usage: [],
        routeConfig: mockSettingsStore.gateway,
        activities: [],
        providerTests: [],
      },
    }, 180)
  },
}
