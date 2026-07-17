import { describe, expect, it } from 'vitest'

import {
  buildOidcStartUrl,
  oidcErrorMessage,
  safeReturnPath,
  withoutOidcError,
} from './login-auth'

describe('login auth helpers', () => {
  it('adds a safely encoded return path without discarding existing OIDC query parameters', () => {
    expect(buildOidcStartUrl(
      '/admin/auth/oidc/start?connection=corporate#authorize',
      '/logs?status=error#request',
      'https://modelport.example',
    )).toBe(
      '/admin/auth/oidc/start?connection=corporate&returnTo=%2Flogs%3Fstatus%3Derror%23request#authorize',
    )
  })

  it('replaces an existing returnTo value and preserves a same-origin absolute start URL', () => {
    const result = buildOidcStartUrl(
      'https://modelport.example/admin/auth/oidc/start?returnTo=%2Funtrusted&connection=corporate',
      '/dashboard',
      'https://modelport.example',
    )
    const url = new URL(result)

    expect(url.origin).toBe('https://modelport.example')
    expect(url.searchParams.getAll('returnTo')).toEqual(['/dashboard'])
    expect(url.searchParams.get('connection')).toBe('corporate')
  })

  it('rejects executable start URL protocols', () => {
    expect(() => buildOidcStartUrl(
      'javascript:alert(1)',
      '/dashboard',
      'https://modelport.example',
    )).toThrow('Unsupported OIDC start URL protocol')
  })

  it('rejects cross-origin, protocol-relative, and unexpected start endpoints', () => {
    expect(() => buildOidcStartUrl(
      'https://auth.example/admin/auth/oidc/start',
      '/dashboard',
      'https://modelport.example',
    )).toThrow('OIDC start URL must be same-origin')
    expect(() => buildOidcStartUrl(
      '//modelport.example/admin/auth/oidc/start',
      '/dashboard',
      'https://modelport.example',
    )).toThrow('OIDC start URL must be same-origin')
    expect(() => buildOidcStartUrl(
      '/admin/auth/another-start',
      '/dashboard',
      'https://modelport.example',
    )).toThrow('Unexpected OIDC start URL path')
  })

  it('accepts only internal return paths', () => {
    expect(safeReturnPath('/logs?status=error#request')).toBe('/logs?status=error#request')
    expect(safeReturnPath('//attacker.example/path')).toBe('')
    expect(safeReturnPath('/\\attacker.example/path')).toBe('')
    expect(safeReturnPath('https://attacker.example/path')).toBe('')
    expect(safeReturnPath('/login?next=/logs')).toBe('')
  })

  it('maps known errors to Chinese messages and never reflects an unknown query value', () => {
    expect(oidcErrorMessage('?oidc_error=invalid_state')).toContain('会话已失效')

    const maliciousValue = '<img src=x onerror=alert(1)>'
    const message = oidcErrorMessage(`?oidc_error=${encodeURIComponent(maliciousValue)}`)
    expect(message).toBe('企业单点登录失败，请重试或联系管理员。')
    expect(message).not.toContain(maliciousValue)
    expect(message).not.toContain('<')
  })

  it('removes every oidc_error query value while preserving unrelated parameters', () => {
    expect(withoutOidcError(
      '?view=compact&oidc_error=invalid_state&next=%2Flogs&oidc_error=provider_error',
    )).toBe('?view=compact&next=%2Flogs')
    expect(withoutOidcError('?oidc_error=invalid_state')).toBe('')
  })
})
