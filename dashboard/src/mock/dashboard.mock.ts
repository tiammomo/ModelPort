import type { DashboardStats, TimeSeriesPoint } from '@/types'

function generateTimeSeries(hours: number, baseValue: number, variance: number): TimeSeriesPoint[] {
  const points: TimeSeriesPoint[] = []
  const now = Date.now()
  for (let i = hours; i >= 0; i--) {
    const timestamp = new Date(now - i * 3600000).toISOString()
    const value = Math.max(0, baseValue + (Math.random() - 0.5) * variance * 2)
    points.push({ timestamp, value: Math.round(value) })
  }
  return points
}

export const mockDashboardStats: DashboardStats = {
  uptimeSeconds: 285900,
  totalRequests: 56780,
  successRate: 96.8,
  activeProviders: 12,
  totalProviders: 15,
  activeUsers: 6,
  totalModels: 32,
  avgLatencyMs: 1850,
  requestTimeSeries: generateTimeSeries(24, 250, 100),
  errorTimeSeries: generateTimeSeries(24, 8, 6),
  topModels: [
    { model: 'mimo-v2.5-pro', provider: 'mimo', requests: 18500 },
    { model: 'deepseek-v4-pro', provider: 'deepseek', requests: 12300 },
    { model: 'claude-sonnet-4-20250514', provider: 'anthropic', requests: 8900 },
    { model: 'gpt-4o', provider: 'openai', requests: 6200 },
    { model: 'gemini-2.5-flash', provider: 'gemini', requests: 4100 },
    { model: 'qwen-plus', provider: 'dashscope', requests: 2800 },
    { model: 'kimi-k2.6', provider: 'kimi', requests: 1900 },
    { model: 'glm-4.7', provider: 'zhipu', requests: 1200 },
  ],
  providerHealth: [
    { providerId: 'mimo', displayName: 'Xiaomi Mimo', status: 'healthy', requestsTotal: 18500, successRate: 98.2, avgLatencyMs: 1200 },
    { providerId: 'deepseek', displayName: 'DeepSeek', status: 'healthy', requestsTotal: 12300, successRate: 97.5, avgLatencyMs: 2100 },
    { providerId: 'anthropic', displayName: 'Anthropic Claude', status: 'healthy', requestsTotal: 8900, successRate: 99.1, avgLatencyMs: 2800 },
    { providerId: 'openai', displayName: 'OpenAI', status: 'degraded', requestsTotal: 6200, successRate: 94.3, avgLatencyMs: 3200 },
    { providerId: 'openrouter', displayName: 'OpenRouter', status: 'healthy', requestsTotal: 3500, successRate: 96.0, avgLatencyMs: 2500 },
    { providerId: 'gemini', displayName: 'Google Gemini', status: 'healthy', requestsTotal: 4100, successRate: 97.8, avgLatencyMs: 1800 },
    { providerId: 'dashscope', displayName: 'DashScope Qwen', status: 'healthy', requestsTotal: 2800, successRate: 98.5, avgLatencyMs: 900 },
    { providerId: 'kimi', displayName: 'Moonshot Kimi', status: 'healthy', requestsTotal: 1900, successRate: 97.0, avgLatencyMs: 1500 },
    { providerId: 'zhipu', displayName: 'Zhipu GLM', status: 'healthy', requestsTotal: 1200, successRate: 96.5, avgLatencyMs: 1100 },
    { providerId: 'xai', displayName: 'xAI Grok', status: 'down', requestsTotal: 300, successRate: 0, avgLatencyMs: 0 },
    { providerId: 'groq', displayName: 'Groq', status: 'degraded', requestsTotal: 800, successRate: 88.5, avgLatencyMs: 600 },
    { providerId: 'mistral', displayName: 'Mistral AI', status: 'down', requestsTotal: 0, successRate: 0, avgLatencyMs: 0 },
  ],
  recentActivity: [
    { id: 'act_001', timestamp: '2026-06-10T09:30:00Z', type: 'request', message: '用户 alice 发送了 15 个请求到 mimo-v2.5-pro', severity: 'info' },
    { id: 'act_002', timestamp: '2026-06-10T09:28:00Z', type: 'error', message: 'OpenAI API 返回 429 Rate Limit', severity: 'warning' },
    { id: 'act_003', timestamp: '2026-06-10T09:25:00Z', type: 'config_change', message: '管理员更新了 mimo 提供商配置', severity: 'info' },
    { id: 'act_004', timestamp: '2026-06-10T09:20:00Z', type: 'error', message: 'xAI Grok 提供商连接超时', severity: 'error' },
    { id: 'act_005', timestamp: '2026-06-10T09:15:00Z', type: 'request', message: '用户 eve 发送了 50 个请求到 deepseek-v4-pro', severity: 'info' },
    { id: 'act_006', timestamp: '2026-06-10T09:10:00Z', type: 'config_change', message: '新增别名 sonnet-router -> openrouter:anthropic/claude-sonnet-4', severity: 'info' },
    { id: 'act_007', timestamp: '2026-06-10T09:05:00Z', type: 'error', message: 'Groq API 返回部分降级响应', severity: 'warning' },
    { id: 'act_008', timestamp: '2026-06-10T09:00:00Z', type: 'request', message: '系统启动完成，已加载 15 个提供商', severity: 'info' },
  ],
}
