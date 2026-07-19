import type { SystemSettings } from '@/types'

export const mockSettings: SystemSettings = {
  server: {
    bindAddress: '127.0.0.1:38082',
    maxRequestBodyBytes: 33554432,
    maxConcurrentRequests: 64,
  },
  auth: {
    enabled: true,
    tokenEnvVar: 'MODELPORT_AUTH_TOKEN',
    allowNoAuth: false,
  },
  gateway: {
    defaultProvider: 'deepseek',
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
  runtime: {
    apiEndpoint: 'http://127.0.0.1:38082/v1/messages',
    anthropicEndpoint: 'http://127.0.0.1:38082/v1/messages',
    openaiEndpoint: 'http://127.0.0.1:38082/v1/chat/completions',
    modelsEndpoint: 'http://127.0.0.1:38082/v1/models',
    adminEndpoint: 'http://127.0.0.1:38082/admin',
    controlDataPath: '/home/user/.modelport/control-plane.json',
    authDataPath: '/home/user/.modelport/auth.json',
  },
  setup: {
    ready: true,
    activeProviderCount: 2,
    defaultProviderReady: true,
    checks: [
      { id: 'admin', label: '管理员账号', status: 'ok', detail: '至少一个活跃管理员' },
      { id: 'auth', label: 'API 认证', status: 'ok', detail: '已启用请求认证' },
      { id: 'providers', label: '供应商凭证', status: 'ok', detail: '2 个供应商可用' },
      { id: 'defaultProvider', label: '默认供应商', status: 'ok', detail: 'deepseek 可用' },
      { id: 'persistence', label: '控制面数据', status: 'ok', detail: '已启用本地持久化' },
      { id: 'config', label: '配置校验', status: 'ok', detail: '无配置告警' },
    ],
    issues: [],
  },
}
