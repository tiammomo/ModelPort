import type { Quota } from '@/types'

export const mockQuotas: Quota[] = [
  { id: 'quota_001', userId: 'usr_001', username: 'admin', quotaType: 'tokens', limit: 10_000_000, used: 3_250_000, period: 'monthly', periodStart: '2026-06-01T00:00:00Z', periodEnd: '2026-06-30T23:59:59Z', resetAt: '2026-07-01T00:00:00Z' },
  { id: 'quota_002', userId: 'usr_002', username: 'alice', quotaType: 'tokens', limit: 5_000_000, used: 4_800_000, period: 'monthly', periodStart: '2026-06-01T00:00:00Z', periodEnd: '2026-06-30T23:59:59Z', resetAt: '2026-07-01T00:00:00Z' },
  { id: 'quota_003', userId: 'usr_003', username: 'bob', quotaType: 'tokens', limit: 2_000_000, used: 890_000, period: 'monthly', periodStart: '2026-06-01T00:00:00Z', periodEnd: '2026-06-30T23:59:59Z', resetAt: '2026-07-01T00:00:00Z' },
  { id: 'quota_004', userId: 'usr_004', username: 'charlie', quotaType: 'requests', limit: 500, used: 120, period: 'daily', periodStart: '2026-06-10T00:00:00Z', periodEnd: '2026-06-10T23:59:59Z', resetAt: '2026-06-11T00:00:00Z' },
  { id: 'quota_005', userId: 'usr_006', username: 'eve', quotaType: 'tokens', limit: 8_000_000, used: 7_950_000, period: 'monthly', periodStart: '2026-06-01T00:00:00Z', periodEnd: '2026-06-30T23:59:59Z', resetAt: '2026-07-01T00:00:00Z' },
  { id: 'quota_006', userId: 'usr_008', username: 'grace', quotaType: 'tokens', limit: 10_000_000, used: 2_100_000, period: 'monthly', periodStart: '2026-06-01T00:00:00Z', periodEnd: '2026-06-30T23:59:59Z', resetAt: '2026-07-01T00:00:00Z' },
  { id: 'quota_007', userId: 'usr_002', username: 'alice', quotaType: 'requests', limit: 5000, used: 3200, period: 'monthly', periodStart: '2026-06-01T00:00:00Z', periodEnd: '2026-06-30T23:59:59Z', resetAt: '2026-07-01T00:00:00Z' },
  { id: 'quota_008', userId: 'usr_006', username: 'eve', quotaType: 'cost', limit: 100, used: 92.5, period: 'monthly', periodStart: '2026-06-01T00:00:00Z', periodEnd: '2026-06-30T23:59:59Z', resetAt: '2026-07-01T00:00:00Z' },
]
