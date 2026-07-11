import { describe, expect, it } from 'vitest'

import type { Provider, SystemSettings } from '@/types'
import { resolveDefaultProvider, withDefaultProvider } from './default-provider'

const providers = [
  { id: 'deepseek', status: 'active' },
  { id: 'mimo', status: 'active' },
] as Provider[]

const settings = {
  gateway: {
    defaultProvider: 'mimo',
    providerOrder: ['mimo', 'deepseek'],
  },
} as SystemSettings

describe('default provider state', () => {
  it('waits without inventing a provider and then follows the authoritative setting', () => {
    expect(resolveDefaultProvider(undefined, [])).toBe('')
    expect(resolveDefaultProvider(undefined, providers)).toBe('deepseek')
    expect(resolveDefaultProvider('mimo', providers)).toBe('mimo')
  })

  it('applies an immutable optimistic update without losing provider order', () => {
    const updated = withDefaultProvider(settings, 'deepseek')

    expect(updated.gateway).toEqual({
      defaultProvider: 'deepseek',
      providerOrder: ['mimo', 'deepseek'],
    })
    expect(settings.gateway.defaultProvider).toBe('mimo')
  })
})
