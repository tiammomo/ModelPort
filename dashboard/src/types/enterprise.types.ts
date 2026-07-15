export type EnterpriseRequestState = 'started' | 'completed' | 'failed' | 'cancelled'
export type EnterpriseClientProtocol = 'anthropic-messages' | 'openai-chat-completions'

export interface EnterpriseLedgerOverview {
  backend: 'memory' | 'postgres'
  location: string
  leaseTtlSecs: number
  reconcileIntervalSecs: number
  totalRequests: number
  startedRequests: number
  completedRequests: number
  failedRequests: number
  cancelledRequests: number
  unreconciledRequests: number
  idempotentRequests: number
  activeLeases: number
  expiredLeases: number
  chargeableRequests: number
  totalCostMicrounits: number
  organizationCount: number
  projectCount: number
  environmentCount: number
}

export interface EnterpriseRequest {
  ledgerId: string
  requestId: string
  organizationId: string
  projectId: string
  environmentId: string
  principalId: string
  clientProtocol: EnterpriseClientProtocol
  requestedModel: string
  stream: boolean
  state: EnterpriseRequestState
  statusCode: number | null
  terminalReason: string | null
  errorMessage: string | null
  inputTokens: number
  outputTokens: number
  cacheWriteTokens: number
  cacheReadTokens: number
  costAmountMicrounits: number
  currency: string
  billingMode: string | null
  chargeable: boolean
  hasIdempotencyKey: boolean
  leaseOwner: string
  leaseExpiresAtMs: number
  createdAtMs: number
  updatedAtMs: number
  completedAtMs: number | null
  attemptCount: number
}

export interface EnterpriseAttempt {
  attemptId: string
  requestLedgerId: string
  organizationId: string
  projectId: string
  environmentId: string
  providerId: string
  resolvedModel: string
  providerProtocol: string
  state: EnterpriseRequestState
  statusCode: number | null
  terminalReason: string | null
  errorMessage: string | null
  inputTokens: number
  outputTokens: number
  cacheWriteTokens: number
  cacheReadTokens: number
  costAmountMicrounits: number
  currency: string
  billingMode: string | null
  chargeable: boolean
  leaseOwner: string
  leaseExpiresAtMs: number
  createdAtMs: number
  updatedAtMs: number
  completedAtMs: number | null
}

export interface EnterpriseRequestPage {
  requests: EnterpriseRequest[]
  total: number
  page: number
  pageSize: number
}

export interface EnterpriseRequestDetail {
  request: EnterpriseRequest
  attempts: EnterpriseAttempt[]
}

export interface EnterpriseRequestFilters {
  state?: EnterpriseRequestState
  protocol?: EnterpriseClientProtocol
  organizationId?: string
  projectId?: string
  environmentId?: string
  search?: string
}

export interface EnterpriseBudgetScope {
  organizationId: string
  projectId: string
  environmentId: string
}

export interface EnterpriseBudgetAccount extends EnterpriseBudgetScope {
  currency: string
  limitMicrounits: number | null
  reservedMicrounits: number
  settledMicrounits: number
  availableMicrounits: number | null
  utilizationBasisPoints: number | null
  version: number
  updatedAtMs: number
}

export interface EnterpriseBudgetEvent extends EnterpriseBudgetScope {
  eventId: string
  currency: string
  reservationId: string | null
  requestLedgerId: string | null
  attemptId: string | null
  eventType: 'reservation_created' | 'settled' | 'released' | 'adjustment'
  reservedDeltaMicrounits: number
  settledDeltaMicrounits: number
  evidenceSource: string
  billingMode: string | null
  reason: string | null
  actorId: string | null
  inputTokens: number
  outputTokens: number
  cacheWriteTokens: number
  cacheReadTokens: number
  createdAtMs: number
}

export interface EnterpriseBudgetView {
  account: EnterpriseBudgetAccount
  recentEvents: EnterpriseBudgetEvent[]
}

export interface EnterpriseBudgetUpdate extends EnterpriseBudgetScope {
  limitMicrounits?: number
  unlimited: boolean
}

export interface EnterpriseBudgetAdjustment extends EnterpriseBudgetScope {
  deltaMicrounits: number
  reason: string
  evidenceReference: string
}
