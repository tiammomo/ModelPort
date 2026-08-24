import { expect, test } from '@playwright/test'
import { login, requireE2EEnv } from './helpers'

test('opens the standalone user guide from the sidebar', async ({ page }) => {
  await login(page, requireE2EEnv())
  await page.getByRole('link', { name: '用户使用说明' }).click()

  await expect(page).toHaveURL(/\/guide$/)
  await expect(page.getByRole('heading', { name: '用户使用说明' })).toBeVisible()
  await expect(page.getByText('最短调用路径')).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Claude Code / Anthropic SDK' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Qwen Code' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'OpenAI SDK' })).toBeVisible()
  await expect(page.getByRole('heading', { name: 'Codex CLI' })).toBeVisible()
  await expect(page.getByText(/尚未提供 POST \/v1\/responses/)).toBeVisible()
  await expect(page.getByRole('button', { name: /复制Codex CLI 配置/ })).toHaveCount(0)
  await expect(page.getByText('管理员：首次接入顺序')).toBeVisible()
})
