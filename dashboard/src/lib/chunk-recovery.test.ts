import { describe, expect, it } from 'vitest'
import { isChunkLoadError } from './chunk-recovery'

describe('chunk recovery', () => {
  it('recognizes stale dynamic import failures across browsers', () => {
    expect(isChunkLoadError(new TypeError('Failed to fetch dynamically imported module: /assets/page-old.js'))).toBe(true)
    expect(isChunkLoadError(new Error('Importing a module script failed.'))).toBe(true)
    expect(isChunkLoadError(new Error('Error loading dynamically imported module'))).toBe(true)
  })

  it('does not classify ordinary route errors as deployment changes', () => {
    expect(isChunkLoadError(new Error('Request failed with status 500'))).toBe(false)
  })
})
