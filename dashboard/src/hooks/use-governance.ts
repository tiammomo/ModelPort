import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { governanceService } from '@/services/governance.service'
import type { GovernanceChangeInput } from '@/types'

const governanceKey = ['governance'] as const

export function useGovernance() {
  return useQuery({
    queryKey: governanceKey,
    queryFn: () => governanceService.getOverview(),
    refetchInterval: 10_000,
  })
}

export function useCreateGovernanceChange() {
  const client = useQueryClient()
  return useMutation({
    mutationFn: (input: GovernanceChangeInput) => governanceService.createChange(input),
    onSuccess: () => client.invalidateQueries({ queryKey: governanceKey }),
  })
}

export function useApproveGovernanceChange() {
  const client = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => governanceService.approveChange(id),
    onSuccess: () => client.invalidateQueries({ queryKey: governanceKey }),
  })
}

export function useApplyGovernanceChange() {
  const client = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => governanceService.applyChange(id),
    onSuccess: () => Promise.all([
      client.invalidateQueries({ queryKey: governanceKey }),
      client.invalidateQueries({ queryKey: ['client-setup'] }),
      client.invalidateQueries({ queryKey: ['providers'] }),
      client.invalidateQueries({ queryKey: ['aliases'] }),
      client.invalidateQueries({ queryKey: ['dashboard'] }),
    ]),
  })
}
