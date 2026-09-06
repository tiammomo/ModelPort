import { expect, test } from '@playwright/test'
import { csrfHeaders, login, requireE2EEnv } from './helpers'

test('opens the user guide through the five-entry task navigation', async ({ page }) => {
  await login(page, requireE2EEnv())
  const navigation = page.getByRole('complementary', { name: '主导航' })
  await expect(navigation.getByRole('link')).toHaveCount(5)
  await navigation.getByRole('link', { name: '模型接入' }).click()
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

test('checks the selected key and disables copying when a refresh fails', async ({ page }) => {
  await login(page, requireE2EEnv())
  const users = await (await page.request.get('/admin/users')).json() as Array<{ id: string; username: string }>
  const admin = users.find((user) => user.username === requireE2EEnv().adminUsername)!
  const name = `e2e_setup_${Date.now()}`
  const response = await page.request.post('/admin/api-keys', {
    headers: csrfHeaders(),
    data: { userId: admin.id, name, allowedProviders: ['custom'], allowedModels: ['ci-model'] },
  })
  expect(response.ok()).toBeTruthy()
  const key = await response.json() as { id: string }
  try {
    await page.goto('/guide')
    await page.getByLabel('选择用于查询模型目录的 API 密钥').click()
    await page.getByRole('option', { name: new RegExp(name) }).click()
    const status = page.getByRole('status', { name: '客户端接入检查' })
    await expect(status).toContainText('所选密钥的模型权限、Provider 凭证和默认外发策略检查通过')
    const copy = page.getByRole('button', { name: '复制Claude Code / Anthropic SDK 配置' })
    await expect(copy).toBeEnabled()
    await page.route(`**/admin/api-keys/${key.id}/setup?*`, (route) => route.fulfill({ status: 503, contentType: 'application/json', body: '{"message":"setup temporarily unavailable"}' }))
    await status.getByRole('button', { name: '重新检查接入' }).click()
    await expect(copy).toBeDisabled()
    await expect(status).toContainText('接入检查失败')
  } finally {
    await page.request.delete(`/admin/api-keys/${key.id}`, { headers: csrfHeaders() })
  }
})
