import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { quotasService } from '@/services/quotas.service'
import { queryKeys } from './use-dashboard'
import type { Quota } from '@/types'

export function useQuotas() {
  return useQuery({
    queryKey: queryKeys.quotas,
    queryFn: () => quotasService.getQuotas(),
  })
}

export function useUpdateQuota() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<Quota> }) => quotasService.updateQuota(id, data),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.quotas }),
  })
}

export function useCreateQuota() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (data: Omit<Quota, 'id'>) => quotasService.createQuota(data),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.quotas }),
  })
}

export function useDeleteQuota() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => quotasService.deleteQuota(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.quotas }),
  })
}
