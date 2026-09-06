import { describe, expect, it } from 'vitest'
import { setupAllowsCopy, setupMessage, type ClientSetupCheck } from './client-setup'

describe('client configuration readiness', () => {
  const ready: ClientSetupCheck = { apiKeyId: 'key-a', model: 'code', status: 'ready', code: 'ready', checkedAtMs: 1, upstreamVerified: false }

  it('does not reuse success from a previous key or model selection', () => {
    expect(setupAllowsCopy(ready, 'key-a', 'code')).toBe(true)
    expect(setupAllowsCopy(ready, 'key-b', 'code')).toBe(false)
    expect(setupAllowsCopy(ready, 'key-a', 'other-model')).toBe(false)
    expect(setupAllowsCopy(undefined, 'key-a', 'code')).toBe(false)
    expect(setupAllowsCopy(ready, '', '')).toBe(false)
  })

  it('fails closed for blocked or inconsistent responses and explains the unknown-data cloud block', () => {
    expect(setupAllowsCopy({ ...ready, status: 'blocked', code: 'local_only' }, 'key-a', 'code')).toBe(false)
    expect(setupAllowsCopy({ ...ready, code: 'unexpected' }, 'key-a', 'code')).toBe(false)
    expect(setupMessage('local_only')).toContain('请选择本地模型')
    expect(setupMessage('unexpected')).toContain('复制已禁用')
  })
})
