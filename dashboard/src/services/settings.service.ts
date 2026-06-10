import type { SystemSettings } from '@/types'
import { api } from '@/lib/api-client'

export const settingsService = {
  getSettings: (): Promise<SystemSettings> => api.get('/admin/settings'),

  updateSettings: (settings: Partial<SystemSettings>): Promise<SystemSettings> =>
    api.put('/admin/settings', settings),

  testProviderConnection: (providerId: string): Promise<{ success: boolean; message: string }> =>
    api.post('/admin/settings/test-provider', { providerId }),
}
