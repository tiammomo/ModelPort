import { describe, expect, it } from 'vitest'
import { moveProviderInOrder, normalizeProviderOrder } from './provider-order'

describe('normalizeProviderOrder', () => {
  it('preserves configured priority and appends newly-added providers', () => {
    expect(normalizeProviderOrder(
      ['anthropic', 'missing', 'openai', 'anthropic'],
      ['openai', 'gemini', 'anthropic'],
    )).toEqual(['anthropic', 'openai', 'gemini'])
  })
})

describe('moveProviderInOrder', () => {
  it('moves a provider by one priority position without mutating the source', () => {
    const source = ['openai', 'anthropic', 'gemini']

    expect(moveProviderInOrder(source, 'anthropic', 'up')).toEqual(['anthropic', 'openai', 'gemini'])
    expect(moveProviderInOrder(source, 'anthropic', 'down')).toEqual(['openai', 'gemini', 'anthropic'])
    expect(source).toEqual(['openai', 'anthropic', 'gemini'])
  })

  it('keeps boundary and unknown providers unchanged', () => {
    const source = ['openai', 'anthropic']

    expect(moveProviderInOrder(source, 'openai', 'up')).toEqual(source)
    expect(moveProviderInOrder(source, 'anthropic', 'down')).toEqual(source)
    expect(moveProviderInOrder(source, 'gemini', 'up')).toEqual(source)
  })
})
