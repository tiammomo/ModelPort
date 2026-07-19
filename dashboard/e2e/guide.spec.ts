import { expect, test } from '@playwright/test'
import { login, requireE2EEnv } from './helpers'

test('opens the standalone user guide from the sidebar', async ({ page }) => {
  await login(page, requireE2EEnv())
  await page.getByRole('link', { name: '用户使用说明' }).click()

  await expect(page).toHaveURL(/\/guide$/)
  await expect(page.getByRole('heading', { name: '用户使用说明' })).toBeVisible()
  await expect(page.getByText('最短调用路径')).toBeVisible()
  await expect(page.getByText('Anthropic-compatible')).toBeVisible()
  await expect(page.getByText('OpenAI-compatible')).toBeVisible()
  await expect(page.getByText('管理员：首次接入顺序')).toBeVisible()
})
