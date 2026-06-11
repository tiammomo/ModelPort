import type { RequestLog } from '@/types'

const routes = [
  { provider: 'mimo', model: 'mimo-v2.5-pro' },
  { provider: 'deepseek', model: 'deepseek-v4-pro' },
  { provider: 'openrouter', model: 'openrouter/auto' },
  { provider: 'openai', model: 'gpt-5.5' },
  { provider: 'anthropic', model: 'claude-fable-5' },
  { provider: 'gemini', model: 'gemini-3.5-flash' },
  { provider: 'dashscope', model: 'qwen-plus' },
  { provider: 'kimi', model: 'kimi-k2.6' },
  { provider: 'zhipu', model: 'glm-4.7' },
]
const users = [
  { id: 'usr_001', username: 'admin' },
  { id: 'usr_002', username: 'alice' },
  { id: 'usr_003', username: 'bob' },
  { id: 'usr_004', username: 'charlie' },
  { id: 'usr_006', username: 'eve' },
  { id: 'usr_008', username: 'grace' },
]
const groups = ['letacode-back-new', 'prod', 'cache-heavy', 'dev-test', 'ops']
const tokenNames = ['xulei11', 'alice-prod-chat', 'bob-dev-test', 'ops-observer']

function pricingForModel(model: string) {
  if (model.includes('claude-opus')) return { inputPerMillion: 5, outputPerMillion: 25, cacheWritePerMillion: 6.25, cacheReadPerMillion: 0.5 }
  if (model.includes('mimo')) return { inputPerMillion: 0.8, outputPerMillion: 3.2, cacheWritePerMillion: 1, cacheReadPerMillion: 0.08 }
  if (model.includes('deepseek')) return { inputPerMillion: 0.14, outputPerMillion: 0.28, cacheWritePerMillion: 0.14, cacheReadPerMillion: 0.0028 }
  return { inputPerMillion: 1.25, outputPerMillion: 7.5, cacheWritePerMillion: 1.25, cacheReadPerMillion: 0.125 }
}

function randomInt(min: number, max: number): number {
  return Math.floor(Math.random() * (max - min + 1)) + min
}

function randomItem<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)]
}

function generateLog(id: number, hoursAgo: number): RequestLog {
  const route = randomItem(routes)
  const user = randomItem(users)
  const isSuccess = Math.random() > 0.08
  const isStream = Math.random() > 0.3
  const inputTokens = randomInt(50, 4000)
  const outputTokens = randomInt(100, 8000)
  const cacheReadTokens = Math.random() > 0.55 ? randomInt(0, 2400) : 0
  const cacheWriteTokens = Math.random() > 0.8 ? randomInt(0, 1200) : 0
  const pricing = pricingForModel(route.model)
  const inputCost = (inputTokens / 1_000_000) * pricing.inputPerMillion
  const outputCost = (outputTokens / 1_000_000) * pricing.outputPerMillion
  const cacheWriteCost = (cacheWriteTokens / 1_000_000) * pricing.cacheWritePerMillion
  const cacheReadCost = (cacheReadTokens / 1_000_000) * pricing.cacheReadPerMillion
  const costEstimate = inputCost + outputCost + cacheWriteCost + cacheReadCost
  const group = randomItem(groups)
  const tokenName = randomItem(tokenNames)
  const latencyMs = randomInt(200, 12000)

  const timestamp = new Date(Date.now() - hoursAgo * 3600000 - randomInt(0, 3599) * 1000)

  return {
    id: `req_${String(id).padStart(6, '0')}_${Math.random().toString(36).slice(2, 10)}`,
    timestamp: timestamp.toISOString(),
    userId: user.id,
    username: user.username,
    apiKeyId: `key_${String(id % 12).padStart(3, '0')}`,
    apiKeyName: tokenName,
    apiKeyGroup: group,
    tokenName,
    group,
    channelId: route.provider,
    channelName: `${route.provider}-primary`,
    model: route.model,
    resolvedModel: route.model,
    provider: route.provider,
    protocol: route.provider === 'anthropic' || route.provider === 'deepseek' ? 'anthropic' : 'openai-compat',
    requestType: isSuccess ? 'consume' : 'error',
    stream: isStream ? 'stream' : 'non-stream',
    status: isSuccess ? 'success' : Math.random() > 0.5 ? 'error' : 'timeout',
    statusCode: isSuccess ? 200 : randomItem([400, 401, 429, 500, 502, 503]),
    inputTokens,
    outputTokens,
    cacheWriteTokens,
    cacheReadTokens,
    billedInputTokens: inputTokens + cacheWriteTokens + cacheReadTokens,
    totalTokens: inputTokens + outputTokens + cacheWriteTokens + cacheReadTokens,
    cacheHitRate: cacheReadTokens > 0 ? (cacheReadTokens / Math.max(inputTokens + cacheWriteTokens + cacheReadTokens, 1)) * 100 : 0,
    costEstimate,
    modelPricing: pricing,
    costBreakdown: {
      inputCost,
      outputCost,
      cacheWriteCost,
      cacheReadCost,
      totalCost: costEstimate,
    },
    latencyMs,
    firstByteLatencyMs: Math.max(20, Math.round(latencyMs * randomInt(35, 85) / 100)),
    retryCount: isSuccess ? randomInt(0, 1) : randomInt(0, 3),
    clientIp: `10.0.${randomInt(0, 12)}.${randomInt(10, 240)}`,
    requestPath: '/v1/messages',
    billingMode: 'upstream-returned',
    detail: `模型: ${route.model} · 缓存创建: ${pricing.cacheWritePerMillion}/1M · 缓存命中: ${pricing.cacheReadPerMillion}/1M · 分组: ${group}`,
    errorMessage: isSuccess ? null : randomItem([
      'Rate limit exceeded',
      'Invalid API key',
      'Model not found',
      'Request timeout',
      'Upstream server error',
    ]),
  }
}

export const mockLogs: RequestLog[] = Array.from({ length: 300 }, (_, i) =>
  generateLog(i, randomInt(0, 168))
).sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())
