import { describe, expect, it } from 'vitest'

import type { DashboardStats, Provider, SystemSettings } from '@/types'
import { buildOnboardingState, nextSetupPath, setupJourneySteps } from './onboarding'

const stats = {
  totalRequests: 0,
  activeUsers: 1,
  apiKeysActive: 0,
} as DashboardStats

const settings = {
  server: { bindAddress: '127.0.0.1:38082', maxRequestBodyBytes: 1024, maxConcurrentRequests: 8 },
  auth: { enabled: true, tokenEnvVar: 'MODELPORT_AUTH_TOKEN', allowNoAuth: false },
  gateway: { defaultProvider: 'local', providerOrder: ['local'] },
  smartRouting: { mode: 'off', defaultProfile: 'balanced', policyVersion: 'test', activationPercent: 0, groupCount: 0, candidateCount: 0 },
  rateLimits: { maxConcurrentRequests: 8, maxRequestBodyBytes: 1024, requestTimeoutSecs: 30, streamIdleTimeoutSecs: 30 },
  setup: { ready: false, activeProviderCount: 0, defaultProviderReady: false, checks: [], issues: [] },
} satisfies SystemSettings

const provider = {
  id: 'local',
  displayName: 'Local',
  status: 'active',
  apiKeyRequired: false,
  hasApiKey: true,
  models: ['coder'],
  modelInventory: [{ model: 'coder', status: 'active' }],
  lastTest: { testedAt: '1', success: true, message: 'ok' },
} as Provider

describe('administrator onboarding state', () => {
  it('resumes from persisted policy state and allows an administrator to use a scoped key', () => {
    const state = buildOnboardingState({
      providers: [provider],
      settings: { ...settings, setup: { ...settings.setup!, defaultProviderReady: true } },
      stats: { ...stats, apiKeysActive: 1, onboardingMilestones: { hasRequestEver: false, hasSuccessfulRequestEver: false, hasDefaultProjectPolicy: true } },
    })
    expect(setupJourneySteps(state)).toHaveLength(4)
    expect(nextSetupPath(state)).toBe('/guide?setup=1')
    expect(setupJourneySteps(state).at(-1)?.complete).toBe(false)
  })
  it('keeps saved provider configuration incomplete until connection evidence exists', () => {
    const state = buildOnboardingState({
      providers: [{ ...provider, lastTest: null }],
      settings,
      stats,
    })

    expect(state.steps.find((step) => step.id === 'provider')?.complete).toBe(false)
    expect(state.steps.find((step) => step.id === 'model')?.complete).toBe(false)
  })

  it('completes the governed request only when its route evidence is present', () => {
    const state = buildOnboardingState({
      providers: [provider],
      settings: { ...settings, setup: { ...settings.setup!, defaultProviderReady: true } },
      stats: {
        ...stats,
        totalRequests: 0,
        activeUsers: 2,
        apiKeysActive: 1,
        onboardingMilestones: {
          hasRequestEver: true,
          hasSuccessfulRequestEver: true,
          hasDefaultProjectPolicy: true,
        },
      },
    })

    expect(state.complete).toBe(true)
    expect(state.percent).toBe(100)
  })

  it('keeps the default route incomplete until an explicit project policy is applied', () => {
    const state = buildOnboardingState({
      providers: [provider],
      settings: { ...settings, setup: { ...settings.setup!, defaultProviderReady: true } },
      stats: {
        ...stats,
        onboardingMilestones: {
          hasRequestEver: false,
          hasSuccessfulRequestEver: false,
          hasDefaultProjectPolicy: false,
        },
      },
    })

    const route = state.steps.find((step) => step.id === 'route')
    expect(route?.complete).toBe(false)
    expect(route?.to).toBe('/governance')
  })
})
