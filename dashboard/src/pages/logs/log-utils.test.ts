import { describe, expect, it } from 'vitest'

import type { RequestLog } from '@/types'
import {
  clampLogPage,
  logViewSearchParams,
  logViewStateFromSearchParams,
  mergeProviderOptions,
  timeRangeToDates,
} from './log-utils'

describe('log page helpers', () => {
  it('merges the provider catalog with legacy log values and the active selection', () => {
    const logs = [
      { provider: 'legacy-provider' },
      { provider: 'DEEPSEEK' },
    ] as RequestLog[]

    expect(mergeProviderOptions(
      ['deepseek', 'openai'],
      logs,
      'removed-provider',
    )).toEqual([
      'deepseek',
      'legacy-provider',
      'openai',
      'removed-provider',
    ])
  })

  it('clamps a stale page to the current valid range', () => {
    expect(clampLogPage(8, 3)).toBe(3)
    expect(clampLogPage(0, 3)).toBe(1)
    expect(clampLogPage(2, 0)).toBe(1)
  })

  it('creates datetime-local ranges without applying the timezone offset twice', () => {
    const now = Date.UTC(2026, 6, 11, 12, 30)
    const range = timeRangeToDates('1h', now)

    expect(new Date(range.dateTo).getTime()).toBe(now)
    expect(new Date(range.dateFrom).getTime()).toBe(now - 60 * 60 * 1_000)
  })

  it('defaults an unfiltered log view to the most recent 24 hours', () => {
    const now = Date.UTC(2026, 6, 19, 1, 30)
    const state = logViewStateFromSearchParams(new URLSearchParams(), now)

    expect(new Date(state.filters.dateTo!).getTime()).toBe(now)
    expect(new Date(state.filters.dateFrom!).getTime()).toBe(now - 24 * 60 * 60 * 1_000)
  })

  it('round-trips shareable filters using timezone-stable epoch values', () => {
    const dateFrom = '2026-07-11T08:30'
    const dateTo = '2026-07-11T09:45'
    const params = logViewSearchParams({
      search: 'request-123',
      provider: 'deepseek',
      status: 'error',
      stream: 'stream',
      toolUse: 'requested',
      dateFrom,
      dateTo,
    }, 3, 100)

    expect(params.get('dateFrom')).toBe(String(new Date(dateFrom).getTime()))
    expect(params.get('dateTo')).toBe(String(new Date(dateTo).getTime()))

    const state = logViewStateFromSearchParams(params)
    expect(state).toMatchObject({
      page: 3,
      pageSize: 100,
      filters: {
        search: 'request-123',
        provider: 'deepseek',
        status: 'error',
        stream: 'stream',
        toolUse: 'requested',
      },
    })
    expect(new Date(state.filters.dateFrom!).getTime()).toBe(new Date(dateFrom).getTime())
    expect(new Date(state.filters.dateTo!).getTime()).toBe(new Date(dateTo).getTime())
  })

  it('ignores invalid status, paging, and date URL values', () => {
    const state = logViewStateFromSearchParams(new URLSearchParams({
      status: 'maybe',
      stream: 'sometimes',
      toolUse: 'sometimes',
      page: '-2',
      pageSize: '999',
      dateFrom: 'not-a-date',
    }))

    expect(state).toEqual({ filters: {}, page: 1, pageSize: 50 })
  })
})
