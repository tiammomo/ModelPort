import { expect, test } from '@playwright/test'
import { requireE2EEnv } from './helpers'

test('expired session explains the redirect and returns to the protected URL', async ({ page }) => {
  const env = requireE2EEnv()
  await page.goto('/login')
  await page.evaluate(() => {
    window.sessionStorage.setItem('modelport_auth_notice', '会话已过期，请重新登录后继续。')
    window.sessionStorage.setItem('modelport_return_to', '/logs?status=error')
  })
  await page.reload()

  await expect(page.getByRole('status')).toContainText('会话已过期')
  await page.locator('#username').fill(env.adminUsername)
  await page.locator('#password').fill(env.adminPassword)
  await page.getByRole('button', { name: /^登录$/ }).click()

  await expect(page).toHaveURL(/\/logs\?status=error$/)
  await expect(page.getByRole('heading', { name: '请求日志' })).toBeVisible()
  await expect(page.getByRole('button', { name: '只看错误' })).toHaveAttribute('aria-pressed', 'true')
})
