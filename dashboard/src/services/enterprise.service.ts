import { api } from '@/lib/api-client'
import { isMockMode, mockDelay } from '@/lib/mock-mode'
import type {
  EnterpriseBudgetAdjustment,
  EnterpriseBudgetScope,
  EnterpriseBudgetUpdate,
  EnterpriseBudgetView,
  EnterpriseLedgerOverview,
  EnterpriseRequestDetail,
  EnterpriseRequestFilters,
  EnterpriseRequestPage,
} from '@/types'

export const DEFAULT_ENTERPRISE_BUDGET_SCOPE: EnterpriseBudgetScope = {
  organizationId: 'org_local',
  projectId: 'prj_default',
  environmentId: 'env_default',
}

const emptyOverview: EnterpriseLedgerOverview = {
  backend: 'memory',
  location: 'memory://enterprise-ledger',
  leaseTtlSecs: 300,
  reconcileIntervalSecs: 60,
  totalRequests: 0,
  startedRequests: 0,
  completedRequests: 0,
  failedRequests: 0,
  cancelledRequests: 0,
  unreconciledRequests: 0,
  idempotentRequests: 0,
  activeLeases: 0,
  expiredLeases: 0,
  chargeableRequests: 0,
  totalCostMicrounits: 0,
  organizationCount: 0,
  projectCount: 0,
  environmentCount: 0,
}

function requestPath(filters: EnterpriseRequestFilters, page: number, pageSize: number) {
  const params = new URLSearchParams({
    page: String(page),
    pageSize: String(pageSize),
  })
  for (const [key, value] of Object.entries(filters)) {
    const normalized = value?.trim()
    if (normalized) params.set(key, normalized)
  }
  return `/admin/enterprise/requests?${params.toString()}`
}

function budgetPath(scope: EnterpriseBudgetScope) {
  const params = new URLSearchParams({
    organizationId: scope.organizationId,
    projectId: scope.projectId,
    environmentId: scope.environmentId,
  })
  return `/admin/enterprise/budget?${params.toString()}`
}

export const enterpriseService = {
  getOverview: (): Promise<EnterpriseLedgerOverview> => (
    isMockMode ? mockDelay(emptyOverview) : api.get('/admin/enterprise/overview')
  ),

  getRequests: (
    filters: EnterpriseRequestFilters,
    page: number,
    pageSize: number,
  ): Promise<EnterpriseRequestPage> => (
    isMockMode
      ? mockDelay({ requests: [], total: 0, page, pageSize })
      : api.get(requestPath(filters, page, pageSize))
  ),

  getRequest: (ledgerId: string): Promise<EnterpriseRequestDetail> => (
    api.get(`/admin/enterprise/requests/${encodeURIComponent(ledgerId)}`)
  ),

  getBudget: (scope: EnterpriseBudgetScope): Promise<EnterpriseBudgetView> => (
    api.get(budgetPath(scope))
  ),

  updateBudget: (input: EnterpriseBudgetUpdate): Promise<EnterpriseBudgetView> => (
    api.put('/admin/enterprise/budget', input)
  ),

  adjustBudget: (input: EnterpriseBudgetAdjustment): Promise<EnterpriseBudgetView> => (
    api.post('/admin/enterprise/budget/adjustments', input)
  ),
}
