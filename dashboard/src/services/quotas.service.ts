import type { Quota } from '@/types'
import { api } from '@/lib/api-client'

export const quotasService = {
  getQuotas: (): Promise<Quota[]> => api.get('/admin/quotas'),

  updateQuota: (id: string, data: Partial<Quota>): Promise<Quota> =>
    api.put(`/admin/quotas/${encodeURIComponent(id)}`, data),

  createQuota: (data: Omit<Quota, 'id'>): Promise<Quota> =>
    api.post('/admin/quotas', { ...data, id: `quota_${Date.now()}` }),

  deleteQuota: (id: string): Promise<void> => api.delete(`/admin/quotas/${encodeURIComponent(id)}`),
}
