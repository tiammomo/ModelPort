import { describe, expect, it } from 'vitest'

import {
  DAY_MS,
  buildModelUsageRows,
  buildTokenTrend,
  compactModelUsageRows,
  computeTrend,
  customRangeError,
  currentLogFilters,
  dashboardTrendParams,
} from './dashboard-data'
import type { RequestLog } from '@/types'

function requestLog(overrides: Partial<RequestLog> = {}): RequestLog {
  return {
    id: 'log_test',
    timestamp: '0',
    userId: 'user_test',
    username: 'tester',
    model: 'client-model',
    resolvedModel: 'resolved-model',
    provider: 'provider-a',
    protocol: 'openai-compat',
    stream: 'non-stream',
    status: 'success',
    statusCode: 200,
    inputTokens: 10,
    outputTokens: 5,
    latencyMs: 25,
    errorMessage: null,
    ...overrides,
  }
}

describe('dashboard data helpers', () => {
  it('computes trend from two equally sized windows', () => {
    expect(computeTrend([{ value: 10 }, { value: 20 }, { value: 20 }, { value: 40 }])).toBe(100)
    expect(computeTrend([{ value: 0 }, { value: 0 }])).toBe(0)
    expect(computeTrend([{ value: 0 }, { value: 5 }])).toBe(100)
  })

  it('falls back to the one-day range for an invalid custom window', () => {
    expect(dashboardTrendParams('custom', '2026-07-11T12:00', '2026-07-11T11:00')).toEqual({
      range: '1d',
    })
    expect(customRangeError('custom', '2026-07-11T12:00', '2026-07-11T11:00')).toContain('晚于')
    expect(customRangeError('custom', '2026-07-11T11:00', '2026-07-11T12:00')).toBeNull()
  })

  it('creates stable log filters when the clock is injected', () => {
    const now = Date.UTC(2026, 6, 11, 12)
    const filters = currentLogFilters('1d', '', '', now)

    expect(new Date(filters.dateFrom ?? '').getTime()).toBe(now - DAY_MS)
    expect(new Date(filters.dateTo ?? '').getTime()).toBe(now)
  })

  it('aggregates model usage and prefers totalTokens supplied by the backend', () => {
    const rows = buildModelUsageRows(
      [
        requestLog({ totalTokens: 100, costEstimate: 0.1 }),
        requestLog({ id: 'log_2', inputTokens: 20, outputTokens: 10, costEstimate: 0.2 }),
        requestLog({ id: 'log_3', provider: 'provider-b', resolvedModel: 'other', totalTokens: 5 }),
      ],
      [],
    )

    expect(rows[0]).toMatchObject({
      model: 'resolved-model',
      provider: 'provider-a',
      requests: 2,
      tokens: 130,
    })
    expect(rows[0].cost).toBeCloseTo(0.3)
  })

  it('buckets token usage without scanning every bucket for every log', () => {
    const halfHour = 30 * 60 * 1_000
    const trend = buildTokenTrend(
      [
        requestLog({ timestamp: String(1), cacheReadTokens: 5 }),
        requestLog({ id: 'log_2', timestamp: String(halfHour + 1), inputTokens: 20, outputTokens: 7 }),
      ],
      0,
      halfHour * 2,
      halfHour,
    )

    expect(trend).toHaveLength(2)
    expect(trend[0]).toMatchObject({ input: 10, output: 5, cacheRead: 5, cacheHitRate: 33.3 })
    expect(trend[1]).toMatchObject({ input: 20, output: 7 })
  })

  it('collapses the long model tail without losing totals', () => {
    const rows = compactModelUsageRows(
      Array.from({ length: 8 }, (_, index) => ({
        model: `model-${index}`,
        provider: `provider-${index}`,
        requests: index + 1,
        tokens: (index + 1) * 10,
        cost: index + 0.5,
      })),
      3,
    )

    expect(rows).toHaveLength(3)
    expect(rows[0].model).toBe('model-7')
    expect(rows[1].model).toBe('model-6')
    expect(rows[2]).toMatchObject({ model: '其他模型', provider: 'multiple' })
    expect(rows.reduce((total, row) => total + row.tokens, 0)).toBe(360)
    expect(rows.reduce((total, row) => total + row.requests, 0)).toBe(36)
  })
})
