import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/lib/mock-mode', () => ({
  isMockMode: false,
  mockDelay: <T>(value: T) => Promise.resolve(value),
}))

import { enterpriseService } from './enterprise.service'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('enterprise service', () => {
  it('sends tenant, lifecycle, protocol, search, and pagination filters to the ledger API', async () => {
    const payload = { requests: [], total: 0, page: 3, pageSize: 50 }
    const fetchMock = vi.fn().mockResolvedValue(Response.json(payload))
    vi.stubGlobal('fetch', fetchMock)

    await expect(enterpriseService.getRequests({
      state: 'failed',
      protocol: 'openai-chat-completions',
      organizationId: 'org_acme',
      projectId: 'prj_gateway',
      environmentId: 'env_prod',
      search: ' req one ',
    }, 3, 50)).resolves.toEqual(payload)

    const [rawUrl] = fetchMock.mock.calls[0] as [string, RequestInit]
    const url = new URL(rawUrl, 'http://modelport.local')
    expect(url.pathname).toBe('/admin/enterprise/requests')
    expect(Object.fromEntries(url.searchParams)).toEqual({
      page: '3',
      pageSize: '50',
      state: 'failed',
      protocol: 'openai-chat-completions',
      organizationId: 'org_acme',
      projectId: 'prj_gateway',
      environmentId: 'env_prod',
      search: 'req one',
    })
  })

  it('encodes ledger ids on the detail endpoint', async () => {
    const payload = { request: {}, attempts: [] }
    const fetchMock = vi.fn().mockResolvedValue(Response.json(payload))
    vi.stubGlobal('fetch', fetchMock)

    await enterpriseService.getRequest('grq/one two')

    expect(fetchMock.mock.calls[0]?.[0]).toBe('/admin/enterprise/requests/grq%2Fone%20two')
  })

  it('reads an exact tenant budget scope', async () => {
    const payload = { account: {}, recentEvents: [] }
    const fetchMock = vi.fn().mockResolvedValue(Response.json(payload))
    vi.stubGlobal('fetch', fetchMock)

    await enterpriseService.getBudget({
      organizationId: 'org_acme',
      projectId: 'prj_gateway',
      environmentId: 'env_prod',
    })

    const [rawUrl] = fetchMock.mock.calls[0] as [string, RequestInit]
    const url = new URL(rawUrl, 'http://modelport.local')
    expect(url.pathname).toBe('/admin/enterprise/budget')
    expect(Object.fromEntries(url.searchParams)).toEqual({
      organizationId: 'org_acme',
      projectId: 'prj_gateway',
      environmentId: 'env_prod',
    })
  })

  it('writes budget limits and evidence adjustments with CSRF protection', async () => {
    const payload = { account: {}, recentEvents: [] }
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(Response.json(payload)))
    vi.stubGlobal('fetch', fetchMock)
    const scope = {
      organizationId: 'org_local',
      projectId: 'prj_default',
      environmentId: 'env_default',
    }

    await enterpriseService.updateBudget({ ...scope, limitMicrounits: 1_000_000, unlimited: false })
    await enterpriseService.adjustBudget({
      ...scope,
      deltaMicrounits: -250_000,
      reason: 'invoice credit',
      evidenceReference: 'invoice://credit-42',
    })

    expect(fetchMock.mock.calls[0]?.[0]).toBe('/admin/enterprise/budget')
    expect(fetchMock.mock.calls[1]?.[0]).toBe('/admin/enterprise/budget/adjustments')
    for (const [, init] of fetchMock.mock.calls as Array<[string, RequestInit]>) {
      expect(new Headers(init.headers).get('X-ModelPort-CSRF')).toBe('1')
    }
    expect(JSON.parse(String((fetchMock.mock.calls[1]?.[1] as RequestInit).body))).toMatchObject({
      deltaMicrounits: -250_000,
      evidenceReference: 'invoice://credit-42',
    })
  })
})
