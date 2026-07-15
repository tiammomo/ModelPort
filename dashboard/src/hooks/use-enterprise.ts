import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { enterpriseService } from '@/services/enterprise.service'
import type {
  EnterpriseBudgetAdjustment,
  EnterpriseBudgetScope,
  EnterpriseBudgetUpdate,
  EnterpriseRequestFilters,
} from '@/types'

export function useEnterpriseOverview() {
  return useQuery({
    queryKey: ['enterprise', 'overview'],
    queryFn: () => enterpriseService.getOverview(),
    refetchInterval: 10_000,
  })
}

export function useEnterpriseRequests(
  filters: EnterpriseRequestFilters,
  page: number,
  pageSize: number,
) {
  return useQuery({
    queryKey: ['enterprise', 'requests', filters, page, pageSize],
    queryFn: () => enterpriseService.getRequests(filters, page, pageSize),
    placeholderData: keepPreviousData,
    refetchInterval: 10_000,
  })
}

export function useEnterpriseRequest(ledgerId?: string) {
  return useQuery({
    queryKey: ['enterprise', 'request', ledgerId],
    queryFn: () => enterpriseService.getRequest(ledgerId!),
    enabled: Boolean(ledgerId),
  })
}

export function useEnterpriseBudget(scope: EnterpriseBudgetScope) {
  return useQuery({
    queryKey: ['enterprise', 'budget', scope],
    queryFn: () => enterpriseService.getBudget(scope),
    refetchInterval: 10_000,
  })
}

export function useUpdateEnterpriseBudget() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: EnterpriseBudgetUpdate) => enterpriseService.updateBudget(input),
    onSuccess: (view) => {
      queryClient.setQueryData(['enterprise', 'budget', {
        organizationId: view.account.organizationId,
        projectId: view.account.projectId,
        environmentId: view.account.environmentId,
      }], view)
    },
  })
}

export function useAdjustEnterpriseBudget() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: EnterpriseBudgetAdjustment) => enterpriseService.adjustBudget(input),
    onSuccess: (view) => {
      queryClient.setQueryData(['enterprise', 'budget', {
        organizationId: view.account.organizationId,
        projectId: view.account.projectId,
        environmentId: view.account.environmentId,
      }], view)
      void queryClient.invalidateQueries({ queryKey: ['enterprise', 'overview'] })
      void queryClient.invalidateQueries({ queryKey: ['enterprise', 'requests'] })
    },
  })
}
