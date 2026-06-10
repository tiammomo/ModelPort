import type { Quota } from '@/types'
import { api } from '@/lib/api-client'
import { isMockMode, mockDelay, nextMockId } from '@/lib/mock-mode'
import { mockQuotas } from '@/mock'

let mockQuotaStore = [...mockQuotas]

export const quotasService = {
  getQuotas: (): Promise<Quota[]> =>
    isMockMode ? mockDelay(mockQuotaStore) : api.get('/admin/quotas'),

  updateQuota: (id: string, data: Partial<Quota>): Promise<Quota> => {
    if (!isMockMode) return api.put(`/admin/quotas/${encodeURIComponent(id)}`, data)
    const current = mockQuotaStore.find((quota) => quota.id === id)
    if (!current) throw new Error('配额不存在')
    const next = { ...current, ...data }
    mockQuotaStore = mockQuotaStore.map((quota) => quota.id === id ? next : quota)
    return mockDelay(next)
  },

  createQuota: (data: Omit<Quota, 'id'>): Promise<Quota> => {
    const row = { ...data, id: nextMockId('quota') }
    if (!isMockMode) return api.post('/admin/quotas', row)
    mockQuotaStore = [row, ...mockQuotaStore]
    return mockDelay(row)
  },

  deleteQuota: (id: string): Promise<void> => {
    if (!isMockMode) return api.delete(`/admin/quotas/${encodeURIComponent(id)}`)
    mockQuotaStore = mockQuotaStore.filter((quota) => quota.id !== id)
    return mockDelay(undefined)
  },
}
