import { describe, expect, it } from 'vitest'
import { navItemsForRole, ROUTES } from './constants'

describe('navItemsForRole', () => {
  it('does not expose administrator destinations to normal users', () => {
    const paths = navItemsForRole('user').map((item) => item.path)

    expect(paths).toContain(ROUTES.DASHBOARD)
    expect(paths).toContain(ROUTES.API_KEYS)
    expect(paths).toContain(ROUTES.LOGS)
    expect(paths).not.toContain(ROUTES.USERS)
    expect(paths).not.toContain(ROUTES.QUOTAS)
    expect(paths).not.toContain(ROUTES.MODELS)
    expect(paths).not.toContain(ROUTES.ENTERPRISE)
    expect(paths).not.toContain(ROUTES.SETTINGS)
  })

  it('keeps every destination available to administrators', () => {
    expect(navItemsForRole('admin')).toHaveLength(8)
    expect(navItemsForRole('admin').map((item) => item.path)).toContain(ROUTES.ENTERPRISE)
  })
})
