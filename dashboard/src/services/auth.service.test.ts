import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/lib/mock-mode', () => ({
  isMockMode: false,
  mockDelay: <T>(value: T) => Promise.resolve(value),
}))

import { authService, type AuthMethods } from './auth.service'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('auth service', () => {
  it('discovers enabled login methods from the public auth endpoint', async () => {
    const methods: AuthMethods = {
      passwordEnabled: true,
      oidc: {
        enabled: true,
        label: '公司单点登录',
        startUrl: '/admin/auth/oidc/start?connection=corporate',
      },
    }
    const fetchMock = vi.fn().mockResolvedValue(Response.json(methods))
    vi.stubGlobal('fetch', fetchMock)

    await expect(authService.getMethods()).resolves.toEqual(methods)

    const [path, options] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(path).toBe('/admin/auth/methods')
    expect(options.credentials).toBe('include')
    expect(new Headers(options.headers).has('Content-Type')).toBe(false)
  })
})
