import { expect, test } from '@playwright/test'
import { login, requireE2EEnv } from './helpers'

test.describe('settings', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
  })

  test('admin can reload runtime configuration from operations', async ({ page }) => {
    await page.goto('/settings')
    await expect(page.getByRole('heading', { name: '运行设置与运维' })).toBeVisible()

    await page.getByRole('tab', { name: '运维审计' }).click()
    await expect(page.getByText('配置热加载')).toBeVisible()
    await expect(page.getByText(/监听端口.*请求体上限/)).toBeVisible()

    const responsePromise = page.waitForResponse((response) =>
      response.url().includes('/admin/settings/reload-config') && response.ok(),
    )
    await page.getByRole('button', { name: '热重载配置' }).click()
    await expect(page.getByRole('dialog', { name: '确认热加载运行配置' })).toBeVisible()
    await page.getByRole('button', { name: '确认热加载' }).click()
    const response = await responsePromise
    const body = await response.json() as {
      ok: boolean
      settings: { gateway: { providerOrder: string[] } }
      reloadScope?: { requiresRestart?: string[] }
    }

    expect(body.ok).toBeTruthy()
    expect(body.settings.gateway.providerOrder.length).toBeGreaterThan(0)
    expect(body.reloadScope?.requiresRestart).toContain('bind address')
    await expect(page.getByText(/配置已热加载/)).toBeVisible()
  })

  test('reload configuration endpoint requires write protection', async ({ page }) => {
    const response = await page.request.post('/admin/settings/reload-config')
    expect(response.status()).toBe(403)
  })
})
