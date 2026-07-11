import { afterEach, describe, expect, it, vi } from 'vitest'

import { ApiError, api } from './api-client'
import { queryClient } from './query-client'

afterEach(() => {
  vi.unstubAllGlobals()
  queryClient.clear()
})

describe('api client', () => {
  it('does not add a JSON content type to bodyless GET requests', async () => {
    const fetchMock = vi.fn().mockResolvedValue(Response.json({ ok: true }))
    vi.stubGlobal('fetch', fetchMock)

    await api.get('/admin/health')

    const [, options] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(new Headers(options.headers).has('Content-Type')).toBe(false)
    expect(options.credentials).toBe('include')
  })

  it('adds JSON and CSRF headers to write requests', async () => {
    const fetchMock = vi.fn().mockResolvedValue(Response.json({ ok: true }))
    vi.stubGlobal('fetch', fetchMock)

    await api.post('/admin/settings', { enabled: true })

    const [, options] = fetchMock.mock.calls[0] as [string, RequestInit]
    const headers = new Headers(options.headers)
    expect(headers.get('Content-Type')).toBe('application/json')
    expect(headers.get('X-ModelPort-CSRF')).toBe('1')
  })

  it('preserves valid falsy JSON request bodies', async () => {
    const fetchMock = vi.fn().mockResolvedValue(Response.json({ ok: true }))
    vi.stubGlobal('fetch', fetchMock)

    await api.put('/admin/flag', false)

    const [, options] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(options.body).toBe('false')
  })

  it('preserves structured server errors and hints', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({
      error: { message: 'provider failed', hint: 'check credentials' },
    }, { status: 502 })))

    await expect(api.get('/admin/provider')).rejects.toMatchObject({
      name: 'ApiError',
      status: 502,
      message: 'provider failed · check credentials',
    } satisfies Partial<ApiError>)
  })

  it('reports malformed successful responses as protocol errors', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('not-json', { status: 200 })))

    await expect(api.get('/admin/settings')).rejects.toBeInstanceOf(ApiError)
  })

  it('isolates cached data and remembers the protected route after a 401', async () => {
    const values = new Map<string, string>()
    const windowMock = {
      location: {
        pathname: '/logs',
        search: '?status=error',
        hash: '#request',
        href: '',
      },
      sessionStorage: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, value),
        removeItem: (key: string) => values.delete(key),
      },
    }
    vi.stubGlobal('window', windowMock)
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ message: 'expired' }, { status: 401 })))
    queryClient.setQueryData(['private', 'logs'], [{ id: 'previous-session' }])

    await expect(api.get('/admin/logs')).rejects.toMatchObject({ status: 401 })

    expect(queryClient.getQueryData(['private', 'logs'])).toBeUndefined()
    expect(values.get('modelport_return_to')).toBe('/logs?status=error#request')
    expect(values.get('modelport_auth_notice')).toContain('会话已过期')
    expect(windowMock.location.href).toBe('/login')
  })
})
