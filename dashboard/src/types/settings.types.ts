export interface SystemSettings {
  server: ServerSettings
  auth: AuthSettings
  gateway: GatewaySettings
  rateLimits: RateLimitSettings
  runtime?: RuntimeSettings
  setup?: SetupStatus
}

export interface ServerSettings {
  bindAddress: string
  maxRequestBodyBytes: number
  maxConcurrentRequests: number
}

export interface AuthSettings {
  enabled: boolean
  tokenEnvVar: string
  allowNoAuth: boolean
}

export interface GatewaySettings {
  defaultProvider: string
  providerOrder: string[]
}

export interface RateLimitSettings {
  maxConcurrentRequests: number
  maxRequestBodyBytes: number
  requestTimeoutSecs: number
  streamIdleTimeoutSecs: number
}

export interface RuntimeSettings {
  apiEndpoint: string
  modelsEndpoint: string
  adminEndpoint: string
  controlDataPath?: string | null
  authDataPath?: string | null
}

export interface SetupStatus {
  ready: boolean
  activeProviderCount: number
  defaultProviderReady: boolean
  checks: SetupCheck[]
  issues: Array<{
    severity: 'error' | 'warning'
    message: string
  }>
}

export interface SetupCheck {
  id: string
  label: string
  status: 'ok' | 'warning' | 'error'
  detail: string
}

export interface AuditEvent {
  id: string
  timestamp: string
  type: 'request' | 'error' | 'config_change' | string
  actor?: string
  target?: string
  message: string
  severity: 'info' | 'warning' | 'error'
}

export interface AuditEventsResponse {
  events: AuditEvent[]
  total: number
}

export interface BackupExport {
  schemaVersion: number
  service: string
  generatedAt: string
  containsSecrets: boolean
  containsPersonalData: boolean
  settings: SystemSettings
  users: unknown[]
  control: unknown
}
