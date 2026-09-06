import { describe, expect, it } from 'vitest'
import { NAV_ITEMS, navItemsForRole, navGroupsForRole, ROUTES } from './constants'

describe('navItemsForRole', () => {
  it('offers five administrator entry points with every existing destination reachable', () => {
    const groups = navGroupsForRole('admin')
    expect(groups).toHaveLength(5)
    expect(groups.flatMap((group) => group.items.map((item) => item.path)).sort()).toEqual(NAV_ITEMS.map((item) => item.path).sort())
    for (const role of ['user', 'viewer', undefined]) {
      expect(navGroupsForRole(role).flatMap((group) => group.items).every((item) => !item.adminOnly)).toBe(true)
    }
  })
  it('does not expose administrator destinations to normal users', () => {
    const paths = navItemsForRole('user').map((item) => item.path)

    expect(paths).toContain(ROUTES.DASHBOARD)
    expect(paths).toContain(ROUTES.API_KEYS)
    expect(paths).toContain(ROUTES.LOGS)
    expect(paths).toContain(ROUTES.GUIDE)
    expect(paths).not.toContain(ROUTES.USERS)
    expect(paths).not.toContain(ROUTES.QUOTAS)
    expect(paths).toContain(ROUTES.MODELS)
    expect(paths).not.toContain(ROUTES.ENTERPRISE)
    expect(paths).not.toContain(ROUTES.SETTINGS)
  })

  it('keeps every destination available to administrators', () => {
    expect(navItemsForRole('admin')).toHaveLength(NAV_ITEMS.length)
    expect(navItemsForRole('admin').map((item) => item.path)).toContain(ROUTES.ENTERPRISE)
    expect(navItemsForRole('admin').map((item) => item.path)).toContain(ROUTES.GUIDE)
  })
})
