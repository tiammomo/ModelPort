import { expect, test } from '@playwright/test'
import { dateTimeLocal, login, requireE2EEnv } from './helpers'

test.describe('dashboard', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
  })

  test('shows trend filters and fetches each supported range', async ({ page }) => {
    await expect(page.getByText('请求量趋势')).toBeVisible()
    await expect(page.getByText('错误率趋势')).toBeVisible()
    await expect(page.getByText('Invalid Date')).toHaveCount(0)

    await Promise.all([
      page.waitForResponse((response) => response.url().includes('/admin/dashboard') && response.url().includes('range=3d') && response.ok()),
      page.getByRole('button', { name: '近3天' }).click(),
    ])
    await expect(page.getByText('Invalid Date')).toHaveCount(0)

    await Promise.all([
      page.waitForResponse((response) => response.url().includes('/admin/dashboard') && response.url().includes('range=7d') && response.ok()),
      page.getByRole('button', { name: '近7天' }).click(),
    ])
    await expect(page.getByText('Invalid Date')).toHaveCount(0)

    const now = Date.now()
    await Promise.all([
      page.waitForResponse((response) => response.url().includes('/admin/dashboard') && response.url().includes('range=custom') && response.ok()),
      page.getByRole('button', { name: '自定义' }).click(),
    ])
    await page.getByLabel('开始时间').fill(dateTimeLocal(now - 2 * 60 * 60 * 1000))
    await Promise.all([
      page.waitForResponse((response) => response.url().includes('/admin/dashboard') && response.url().includes('range=custom') && response.ok()),
      page.getByLabel('结束时间').fill(dateTimeLocal(now)),
    ])
    await expect(page.getByText('Invalid Date')).toHaveCount(0)
  })
})
