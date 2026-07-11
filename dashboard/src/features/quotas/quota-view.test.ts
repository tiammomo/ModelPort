import { describe, expect, it } from 'vitest'
import type { Quota } from '@/types'
import { filterQuotas, isQuotaFilterActive, isQuotaLimitValid, quotaPeriodRange, quotaRisk, quotaUsagePercent } from './quota-view'

const quotas: Quota[] = [
  {
    id: 'q1', userId: 'usr_alice', username: 'Alice', quotaType: 'tokens', limit: 100, used: 40,
    period: 'daily', periodStart: '', periodEnd: '', resetAt: '',
  },
  {
    id: 'q2', userId: 'usr_bob', username: 'Bob', quotaType: 'cost', limit: 10, used: 9,
    period: 'monthly', periodStart: '', periodEnd: '', resetAt: '',
  },
]

describe('quota usage state', () => {
  it('keeps overages visible and classifies warning thresholds', () => {
    expect(quotaUsagePercent(120, 100)).toBe(120)
    expect(quotaRisk(79, 100)).toBe('healthy')
    expect(quotaRisk(79.6, 100)).toBe('healthy')
    expect(quotaRisk(80, 100)).toBe('warning')
    expect(quotaRisk(100, 100)).toBe('exhausted')
    expect(quotaRisk(0, 0)).toBe('exhausted')
  })
})

it('combines quota filters', () => {
  expect(filterQuotas(quotas, { search: 'BOB', quotaType: 'cost', period: 'monthly', risk: 'warning' })).toEqual([quotas[1]])
})

it('detects active quota filters', () => {
  expect(isQuotaFilterActive({ search: ' ', quotaType: 'all', period: 'all', risk: 'all' })).toBe(false)
  expect(isQuotaFilterActive({ search: '', quotaType: 'tokens', period: 'all', risk: 'all' })).toBe(true)
})

it('validates units without rejecting fractional cost limits', () => {
  expect(isQuotaLimitValid(1.5, 'cost')).toBe(true)
  expect(isQuotaLimitValid(1.5, 'tokens')).toBe(false)
  expect(isQuotaLimitValid(10, 'requests')).toBe(true)
  expect(isQuotaLimitValid(0, 'requests')).toBe(true)
  expect(isQuotaLimitValid(-1, 'requests')).toBe(false)
})

describe('quotaPeriodRange', () => {
  const now = new Date('2026-07-11T12:34:56.000Z')

  it('uses natural UTC day boundaries', () => {
    expect(quotaPeriodRange('daily', now)).toEqual({
      periodStart: '2026-07-11T00:00:00.000Z',
      periodEnd: '2026-07-12T00:00:00.000Z',
      resetAt: '2026-07-12T00:00:00.000Z',
    })
  })

  it('uses Monday-based UTC week boundaries', () => {
    expect(quotaPeriodRange('weekly', now)).toEqual({
      periodStart: '2026-07-06T00:00:00.000Z',
      periodEnd: '2026-07-13T00:00:00.000Z',
      resetAt: '2026-07-13T00:00:00.000Z',
    })
  })

  it('uses UTC calendar month boundaries', () => {
    expect(quotaPeriodRange('monthly', now)).toEqual({
      periodStart: '2026-07-01T00:00:00.000Z',
      periodEnd: '2026-08-01T00:00:00.000Z',
      resetAt: '2026-08-01T00:00:00.000Z',
    })
  })
})
