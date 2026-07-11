import { describe, expect, it } from 'vitest'

import type { Provider } from '@/types'
import { DEFAULT_CREDENTIAL_FORM, DEFAULT_PROVIDER_FORM } from './model-data'
import {
  providerReadiness,
  settingsTabForCheck,
  validateAliasForm,
  validateCredentialForm,
  validateProviderForm,
} from './operator-state'

function provider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: 'openai',
    displayName: 'OpenAI',
    protocol: 'openai-compat',
    baseUrl: 'https://api.openai.com/v1',
    apiKeyEnv: 'OPENAI_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'gpt-5',
    models: ['gpt-5'],
    modelPrefixes: ['gpt-'],
    passthroughUnknownModels: false,
    maxTokensField: 'max_completion_tokens',
    deduplicateStreamText: false,
    bufferStreamText: false,
    status: 'active',
    hasApiKey: true,
    ...overrides,
  }
}

describe('operator-facing provider state', () => {
  it('prioritizes blocked credentials over generic active status', () => {
    expect(providerReadiness(provider({ hasApiKey: false })).level).toBe('blocked')
    expect(providerReadiness(provider({ hasApiKey: false })).label).toBe('缺少凭证')
  })

  it('distinguishes a healthy default route', () => {
    expect(providerReadiness(provider(), true)).toMatchObject({
      level: 'ready',
      label: '默认路由已配置',
    })
  })

  it('surfaces cooldown and backend-recommended recovery', () => {
    const state = providerReadiness(provider({
      runtimeStatus: 'cooldown',
      health: {
        providerId: 'openai',
        requestsTotal: 3,
        successesTotal: 0,
        failuresTotal: 3,
        consecutiveFailures: 3,
        successRate: 0,
        status: 'cooldown',
        recommendedAction: '检查余额',
      },
    }))
    expect(state).toMatchObject({ level: 'attention', nextStep: '检查余额' })
  })

  it('does not present an error-state provider as routable', () => {
    expect(providerReadiness(provider({ status: 'error' }))).toMatchObject({
      level: 'blocked',
      label: '配置异常',
    })
  })

  it('surfaces recharge state reported by a credential profile', () => {
    const state = providerReadiness(provider({
      credentials: [{
        id: 'backup',
        providerId: 'openai',
        name: 'Backup',
        apiKeyEnv: 'OPENAI_BACKUP_API_KEY',
        status: 'active',
        active: true,
        hasApiKey: true,
        health: {
          providerId: 'openai',
          credentialId: 'backup',
          requestsTotal: 1,
          successesTotal: 0,
          failuresTotal: 1,
          consecutiveFailures: 1,
          successRate: 0,
          status: 'degraded',
          rechargeRequired: true,
        },
      }],
    }))

    expect(state).toMatchObject({ level: 'attention', label: '需要处理账号' })
  })
})

describe('operator form validation', () => {
  it('rejects invalid provider URLs and strict stream rewriting', () => {
    const result = validateProviderForm({
      ...DEFAULT_PROVIDER_FORM,
      id: 'custom',
      baseUrl: 'ftp://example.com',
      defaultModel: 'model-a',
      fidelityMode: 'strict',
      deduplicateStreamText: true,
    })
    expect(result.errors.baseUrl).toContain('http://')
    expect(result.errors.fidelityMode).toContain('严格无损')
    expect(result.valid).toBe(false)
  })

  it('warns when a remote endpoint would send credentials over HTTP', () => {
    const result = validateProviderForm({
      ...DEFAULT_PROVIDER_FORM,
      id: 'proxy',
      baseUrl: 'http://proxy.example.com/v1',
      defaultModel: 'model-a',
      models: 'model-a',
      apiKeyEnv: 'PROXY_API_KEY',
    })
    expect(result.valid).toBe(true)
    expect(result.warnings.join(' ')).toContain('明文')
  })

  it('validates credential identifiers and environment variable names', () => {
    const result = validateCredentialForm({
      ...DEFAULT_CREDENTIAL_FORM,
      id: 'Account A',
      name: '主账号',
      apiKeyEnv: '1_BAD_KEY',
    }, true)
    expect(result.errors.id).toBeTruthy()
    expect(result.errors.apiKeyEnv).toBeTruthy()
  })

  it('warns before using an insecure remote credential endpoint', () => {
    const result = validateCredentialForm({
      ...DEFAULT_CREDENTIAL_FORM,
      id: 'account-a',
      name: '主账号',
      apiKeyEnv: 'ACCOUNT_A_API_KEY',
      baseUrl: 'http://proxy.example.com/v1',
    }, true)

    expect(result.valid).toBe(true)
    expect(result.warnings.join(' ')).toContain('明文')
  })

  it('validates alias syntax before sending a request', () => {
    expect(validateAliasForm('provider:model', '').valid).toBe(false)
    expect(validateAliasForm('sonnet', 'anthropic:claude-sonnet').valid).toBe(true)
  })
})

describe('settings setup navigation', () => {
  it('routes setup checks to the page section that can explain the next action', () => {
    expect(settingsTabForCheck('auth')).toBe('security')
    expect(settingsTabForCheck('providers')).toBe('providers')
    expect(settingsTabForCheck('config')).toBe('operations')
  })
})
