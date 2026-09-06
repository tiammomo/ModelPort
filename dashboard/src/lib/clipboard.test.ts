import { afterEach, describe, expect, it, vi } from 'vitest'
import { copyToClipboard } from './utils'

afterEach(() => vi.unstubAllGlobals())

describe('clipboard feedback', () => {
  it('copies through the browser clipboard without touching the document', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    expect(await copyToClipboard('modelport')).toBe(true)
    expect(writeText).toHaveBeenCalledWith('modelport')
  })

  it.each([true, false, 'throw'])('reports fallback result %s and removes temporary secret text', async (result) => {
    const textarea = { value: '', style: {}, select: vi.fn(), remove: vi.fn() }
    const focus = vi.fn()
    vi.stubGlobal('navigator', {})
    vi.stubGlobal('document', {
      activeElement: { focus },
      createElement: () => textarea,
      body: { appendChild: vi.fn() },
      execCommand: () => { if (result === 'throw') throw new Error('denied'); return result },
    })
    expect(await copyToClipboard('synthetic-key')).toBe(result === true)
    expect(textarea.remove).toHaveBeenCalledOnce()
    expect(focus).toHaveBeenCalledOnce()
  })
})
