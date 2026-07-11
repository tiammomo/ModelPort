import { afterEach, describe, expect, it, vi } from 'vitest'

import type { LogSummary, RequestLog } from '@/types'

vi.mock('@/lib/mock-mode', () => ({
  isMockMode: false,
  mockDelay: <T>(value: T) => Promise.resolve(value),
}))

import { logsService } from './logs.service'

const emptySummary: LogSummary = {
  totalRequests: 0,
  successRequests: 0,
  totalInputTokens: 0,
  totalOutputTokens: 0,
  totalCacheWriteTokens: 0,
  totalCacheReadTokens: 0,
  totalTokens: 0,
  totalCostEstimate: 0,
  rpm: 0,
  tpm: 0,
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('logs service', () => {
  it('delegates filtering and pagination to the server using epoch milliseconds', async () => {
    const payload = { logs: [], total: 0, summary: emptySummary }
    const fetchMock = vi.fn().mockResolvedValue(Response.json(payload))
    vi.stubGlobal('fetch', fetchMock)

    const result = await logsService.getLogs({
      status: 'error',
      provider: 'deepseek',
      model: 'reasoner',
      userId: 'user/one',
      apiKeyId: 'key one',
      dateFrom: '2026-07-10T00:00:00.000Z',
      dateTo: '2026-07-11T00:00:00.000Z',
      search: 'request id',
      username: 'operator',
      group: 'production',
      stream: 'stream',
    }, 2, 50)

    const [rawUrl] = fetchMock.mock.calls[0] as [string, RequestInit]
    const url = new URL(rawUrl, 'http://modelport.local')
    expect(url.pathname).toBe('/admin/logs')
    expect(Object.fromEntries(url.searchParams)).toEqual({
      page: '2',
      pageSize: '50',
      status: 'error',
      provider: 'deepseek',
      model: 'reasoner',
      userId: 'user/one',
      apiKeyId: 'key one',
      dateFrom: String(Date.parse('2026-07-10T00:00:00.000Z')),
      dateTo: String(Date.parse('2026-07-11T00:00:00.000Z')),
      search: 'request id',
      username: 'operator',
      group: 'production',
      stream: 'stream',
    })
    expect(result).toEqual(payload)
  })

  it('clamps invalid client pagination before issuing a request', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({
      logs: [],
      total: 0,
      summary: emptySummary,
    })))

    await logsService.getLogs(undefined, 0, 5_000)

    const fetchMock = vi.mocked(fetch)
    const [rawUrl] = fetchMock.mock.calls[0] as [string, RequestInit]
    const url = new URL(rawUrl, 'http://modelport.local')
    expect(url.searchParams.get('page')).toBe('1')
    expect(url.searchParams.get('pageSize')).toBe('500')
  })

  it('loads a single log from the detail endpoint with an encoded id', async () => {
    const log = { id: 'log/one two' } as RequestLog
    const fetchMock = vi.fn().mockResolvedValue(Response.json(log))
    vi.stubGlobal('fetch', fetchMock)

    await expect(logsService.getLogById(log.id)).resolves.toEqual(log)

    expect(fetchMock.mock.calls[0]?.[0]).toBe('/admin/logs/log%2Fone%20two')
  })
})
