export interface SystemSettings {
  server: ServerSettings
  auth: AuthSettings
  gateway: GatewaySettings
  rateLimits: RateLimitSettings
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
