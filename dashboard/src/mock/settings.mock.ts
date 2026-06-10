import type { SystemSettings } from '@/types'

export const mockSettings: SystemSettings = {
  server: {
    bindAddress: '127.0.0.1:17878',
    maxRequestBodyBytes: 33554432,
    maxConcurrentRequests: 64,
  },
  auth: {
    enabled: true,
    tokenEnvVar: 'MODELPORT_AUTH_TOKEN',
    allowNoAuth: false,
  },
  gateway: {
    defaultProvider: 'mimo',
    providerOrder: [
      'deepseek',
      'mimo',
      'openrouter',
      'openai',
      'anthropic',
      'gemini',
      'xai',
      'groq',
      'dashscope',
      'kimi',
      'zhipu',
      'mistral',
      'ark',
      'ollama',
      'custom',
    ],
  },
  rateLimits: {
    maxConcurrentRequests: 64,
    maxRequestBodyBytes: 33554432,
    requestTimeoutSecs: 300,
    streamIdleTimeoutSecs: 120,
  },
}
