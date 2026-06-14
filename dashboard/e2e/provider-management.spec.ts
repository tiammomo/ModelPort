import { expect, test } from '@playwright/test'
import { cleanupE2EProviders, login, requireE2EEnv } from './helpers'

test.describe('provider management', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
    await cleanupE2EProviders(page)
  })

  test.afterEach(async ({ page }) => {
    await cleanupE2EProviders(page)
  })

  test('creates, edits, disables, updates models, and deletes a provider', async ({ page }) => {
    const suffix = Date.now().toString(36)
    const providerId = `e2e_provider_${suffix}`
    const displayName = `E2E Provider ${suffix}`
    const firstModel = `e2e-model-a-${suffix}`
    const secondModel = `e2e-model-b-${suffix}`

    await page.goto('/models')
    await page.getByRole('tab', { name: '供应商' }).click()
    await page.getByRole('button', { name: '新增供应商' }).click()

    await page.getByPlaceholder('例如: siliconflow', { exact: true }).fill(providerId)
    await page.getByPlaceholder('例如: 第三方 · OpenAI').fill(displayName)
    await page.getByPlaceholder('https://example.com/v1').fill('http://127.0.0.1:9/v1')
    await page.getByPlaceholder('例如: gpt-4o-mini').fill(firstModel)
    await page.getByPlaceholder(/每行一个模型/).fill(`${firstModel}\n${secondModel}`)
    await page.getByRole('switch', { name: '需要 API Key' }).click()

    await page.getByRole('button', { name: '保存' }).click()

    const card = page.getByTestId(`provider-card-${providerId}`)
    await expect(card).toBeVisible()
    await expect(card).toContainText(providerId)
    await expect(card).toContainText('可路由')

    await card.getByRole('button', { name: '禁用' }).click()
    await expect(card).toContainText('禁用')
    await card.getByRole('button', { name: '恢复' }).click()
    await expect(card).toContainText('活跃')

    await card.getByRole('button', { name: /查看.*模型列表|查看列表/ }).click()
    await expect(card).toContainText(secondModel)
    await card.getByRole('button', { name: /^默认$/ }).click()
    await expect(card).toContainText(`复制默认路由：${providerId}:${secondModel}`)

    await card.getByRole('switch', { name: `禁用 ${firstModel}` }).click()
    await expect(card).toContainText('已禁用')
    await card.getByRole('switch', { name: `启用 ${firstModel}` }).click()
    await expect(card.getByText('已禁用')).toHaveCount(0)

    await card.getByRole('button', { name: '删除' }).click()
    const deleteDialog = page.getByRole('dialog')
    await deleteDialog.getByRole('button', { name: '删除' }).click()
    await expect(deleteDialog.getByText(/发现 \d+ 个依赖/)).toBeVisible()
    const forceDelete = deleteDialog.getByRole('button', { name: '强制删除' })
    await expect(forceDelete).toBeVisible()
    await forceDelete.click()
    await expect(deleteDialog).toBeHidden()
    await expect(card).toHaveCount(0)
  })
})
