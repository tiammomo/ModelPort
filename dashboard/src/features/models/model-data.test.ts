import { describe, expect, it } from 'vitest'

import {
  dependencyLabel,
  defaultToolStreamingArguments,
  parseList,
  providerInventoryItems,
  providerCredentialState,
  providerOrigin,
  providerPayloadFromForm,
  providerToForm,
} from './model-data'
import type { Provider } from '@/types'

function provider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: 'openai',
    displayName: 'OpenAI',
    protocol: 'openai-compat',
    baseUrl: 'https://api.openai.com/v1',
    apiKeyEnv: 'OPENAI_API_KEY',
    apiKeyRequired: true,
    defaultModel: 'gpt-default',
    models: ['gpt-secondary', 'gpt-default'],
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

describe('model feature data', () => {
  it('classifies official, proxied and local provider origins', () => {
    expect(providerOrigin(provider())).toBe('官方')
    expect(providerOrigin(provider({ baseUrl: 'https://proxy.example.com/v1' }))).toBe('第三方')
    expect(providerOrigin(provider({ id: 'local_vllm', baseUrl: 'http://127.0.0.1:8000/v1' }))).toBe('本地')
  })

  it('sorts the default inventory item first and active items before disabled items', () => {
    const items = providerInventoryItems(provider({
      defaultModel: 'model-b',
      models: ['model-a', 'model-b', 'model-c'],
      modelInventory: [
        { model: 'model-c', status: 'active' },
        { model: 'model-a', status: 'disabled' },
        { model: 'model-b', status: 'active' },
      ],
    }))

    expect(items.map((item) => item.model)).toEqual(['model-b', 'model-c', 'model-a'])
  })

  it('normalizes and deduplicates form lists', () => {
    expect(parseList('a, b\na\n  c  ')).toEqual(['a', 'b', 'c'])
  })

  it('round-trips provider fields through the edit form mapper', () => {
    const source = provider({
      models: ['model-a', 'model-b'],
      modelPrefixes: ['model-'],
      toolUse: {
        supported: true,
        toolChoice: false,
        parallelToolCalls: false,
        streamingArguments: 'cumulative',
        responseValidation: 'strict',
      },
    })

    const payload = providerPayloadFromForm(providerToForm(source), false)
    expect(payload).toMatchObject({
      protocol: source.protocol,
      baseUrl: source.baseUrl,
      apiKeyEnv: source.apiKeyEnv,
      models: source.models,
      modelPrefixes: source.modelPrefixes,
      toolUse: source.toolUse,
    })
    expect(payload.clearApiKeyEnv).toBeUndefined()
  })

  it('uses an explicit flag when the API key environment variable is cleared', () => {
    const form = providerToForm(provider())
    form.apiKeyEnv = '   '

    const payload = providerPayloadFromForm(form, false)

    expect(payload).not.toHaveProperty('apiKeyEnv')
    expect(payload.clearApiKeyEnv).toBe(true)
  })

  it('serializes safe headers, provider timeouts, and bounded retry policy', () => {
    const form = providerToForm(provider())
    form.staticHeaders = 'HTTP-Referer: https://modelport.example\nX-Title: ModelPort'
    form.requestTimeoutMs = '120000'
    form.streamIdleTimeoutMs = '30000'
    form.retryMaxAttempts = '2'

    expect(providerPayloadFromForm(form, false)).toMatchObject({
      staticHeaders: {
        'HTTP-Referer': 'https://modelport.example',
        'X-Title': 'ModelPort',
      },
      requestTimeoutMs: 120000,
      streamIdleTimeoutMs: 30000,
      retry: {
        max_attempts: 2,
        initial_delay_ms: 250,
        max_delay_ms: 5000,
        jitter_ratio: 0.1,
      },
    })
  })

  it('chooses stream argument defaults from protocol and adapter behavior', () => {
    expect(defaultToolStreamingArguments('anthropic', false, 'anthropic')).toBe('native')
    expect(defaultToolStreamingArguments('openai-compat', true, 'proxy')).toBe('cumulative')
    expect(defaultToolStreamingArguments('openai-compat', false, 'local_vllm')).toBe('best_effort')
    expect(defaultToolStreamingArguments('openai-compat', false, 'openai')).toBe('delta')
  })

  it('uses operator-facing labels for provider deletion dependencies', () => {
    expect(dependencyLabel('defaultProvider')).toBe('默认 Provider')
    expect(dependencyLabel('providerOrder')).toBe('Provider 顺序')
    expect(dependencyLabel('apiKey')).toBe('API 密钥')
  })
})

describe('provider credential availability', () => {
  const credential = { id: 'pool-a', providerId: 'openai', name: 'Account A', apiKeyEnv: 'POOL_A_KEY', status: 'active' as const, active: false, hasApiKey: true }

  it('recognizes a resolved pool credential when the default environment key is missing', () => {
    const source = provider({ hasApiKey: false, credentials: [credential], activeCredentialId: credential.id })
    expect(providerCredentialState(source).credentialReady).toBe(true)
    expect(providerCredentialState(source).activeCredential?.id).toBe(credential.id)
  })

  it('ignores disabled or unresolved pool credentials for readiness', () => {
    expect(providerCredentialState(provider({ hasApiKey: false, credentials: [{ ...credential, status: 'disabled' }] })).credentialReady).toBe(false)
    expect(providerCredentialState(provider({ hasApiKey: false, credentials: [{ ...credential, hasApiKey: false }] })).credentialReady).toBe(false)
    expect(providerCredentialState(provider({ hasApiKey: false, apiKeyRequired: false })).credentialReady).toBe(true)
  })

  it('keeps the runtime active credential ahead of the configured fallback', () => {
    const active = { ...credential, id: 'pool-b', active: true }
    const source = provider({ credentials: [credential, active], activeCredentialId: credential.id })
    expect(providerCredentialState(source).activeCredential?.id).toBe(active.id)
  })
})
