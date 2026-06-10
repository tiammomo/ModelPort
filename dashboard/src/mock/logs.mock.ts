import type { RequestLog } from '@/types'

const providers = ['mimo', 'deepseek', 'openrouter', 'openai', 'anthropic', 'gemini', 'dashscope', 'kimi', 'zhipu']
const models = ['mimo-v2.5-pro', 'deepseek-v4-pro', 'openrouter/auto', 'gpt-4o', 'claude-sonnet-4-20250514', 'gemini-2.5-flash', 'qwen-plus', 'kimi-k2.6', 'glm-4.7']
const users = [
  { id: 'usr_001', username: 'admin' },
  { id: 'usr_002', username: 'alice' },
  { id: 'usr_003', username: 'bob' },
  { id: 'usr_004', username: 'charlie' },
  { id: 'usr_006', username: 'eve' },
  { id: 'usr_008', username: 'grace' },
]

function randomInt(min: number, max: number): number {
  return Math.floor(Math.random() * (max - min + 1)) + min
}

function randomItem<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)]
}

function generateLog(id: number, hoursAgo: number): RequestLog {
  const providerIdx = randomInt(0, providers.length - 1)
  const provider = providers[providerIdx]
  const model = models[providerIdx]
  const user = randomItem(users)
  const isSuccess = Math.random() > 0.08
  const isStream = Math.random() > 0.3

  const timestamp = new Date(Date.now() - hoursAgo * 3600000 - randomInt(0, 3599) * 1000)

  return {
    id: `req_${String(id).padStart(6, '0')}_${Math.random().toString(36).slice(2, 10)}`,
    timestamp: timestamp.toISOString(),
    userId: user.id,
    username: user.username,
    model,
    resolvedModel: model,
    provider,
    protocol: provider === 'anthropic' || provider === 'deepseek' ? 'anthropic' : 'openai-compat',
    stream: isStream ? 'stream' : 'non-stream',
    status: isSuccess ? 'success' : Math.random() > 0.5 ? 'error' : 'timeout',
    statusCode: isSuccess ? 200 : randomItem([400, 401, 429, 500, 502, 503]),
    inputTokens: randomInt(50, 4000),
    outputTokens: randomInt(100, 8000),
    latencyMs: randomInt(200, 12000),
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
