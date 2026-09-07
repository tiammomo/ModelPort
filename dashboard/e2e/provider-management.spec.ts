import { expect, test } from '@playwright/test'
import { cleanupE2EProviders, csrfHeaders, login, requireE2EEnv } from './helpers'

test.describe('provider management', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
    await cleanupE2EProviders(page)
  })

  test.afterEach(async ({ page }) => {
    await cleanupE2EProviders(page)
  })

  test('manages an approved provider and rejects arbitrary endpoints', async ({ page }) => {
    const suffix = Date.now().toString(36)
    const providerId = 'deepseek'

    await page.goto('/models')
    await page.getByRole('tab', { name: 'Provider 与凭证' }).click()
    await expect(page.getByText('新 Provider 先进入组织审查目录')).toBeVisible()
    await expect(page.getByRole('button', { name: '接入新 Provider' })).toBeVisible()

    const rejected = await page.request.post('/admin/providers', {
      headers: csrfHeaders(),
      data: {
        id: `e2e_unreviewed_${suffix}`,
        displayName: 'Unreviewed endpoint',
        protocol: 'openai-compat',
        baseUrl: 'https://example.com/v1',
        apiKeyRequired: false,
        defaultModel: 'unreviewed-model',
        models: ['unreviewed-model'],
      },
    })
    expect(rejected.status()).toBe(403)
    await expect(rejected.json()).resolves.toMatchObject({
      error: { code: 'forbidden' },
    })

    const card = page.getByTestId(`provider-card-${providerId}`)
    await expect(card).toBeVisible()
    await expect(card).toContainText(providerId)

    await card.getByRole('button', { name: '禁用' }).click()
    await expect(card).toContainText('禁用')
    await card.getByRole('button', { name: '恢复' }).click()
    await expect(card.getByRole('button', { name: '禁用' })).toBeVisible()
    await expect(card).not.toContainText('已停用')

    await card.getByRole('button', { name: /查看.*模型列表|查看列表/ }).click()
    await expect(card).toContainText('deepseek-v4-pro')
    await card.getByRole('switch', { name: '禁用 deepseek-v4-pro' }).click()
    await expect(card).toContainText('已禁用')
    await card.getByRole('switch', { name: '启用 deepseek-v4-pro' }).click()
    await expect(card.getByText('已禁用')).toHaveCount(0)
  })

  test('validates, creates, edits and deletes credentials on provider cards', async ({ page }) => {
    const suffix = Date.now().toString(36)
    const providerId = 'deepseek'
    const credentialId = `e2e_pool_${suffix}`

    await page.goto('/models')
    await page.getByRole('tab', { name: 'Provider 与凭证' }).click()

    const card = page.getByTestId(`provider-card-${providerId}`)
    await expect(card).toBeVisible()
    await expect(card).toContainText('默认凭证')

    try {
      await card.getByRole('button', { name: '新增' }).click()
      const credentialDialog = page.getByRole('dialog')
      await credentialDialog.getByRole('button', { name: '新增账号' }).click()
      await expect(credentialDialog.locator('#credential-id')).toHaveAttribute('aria-invalid', 'true')
      await expect(credentialDialog.locator('#credential-id')).toBeFocused()
      await credentialDialog.getByPlaceholder('例如: account-a').fill(credentialId)
      await credentialDialog.getByPlaceholder('例如: Mimo 主账号').fill('Pool Account A')
      await credentialDialog.getByPlaceholder('例如: MIMO_OPENAI_API_KEY_ALT').fill(`E2E_POOL_KEY_${suffix.toUpperCase()}`)
      await credentialDialog.getByRole('button', { name: '新增账号' }).click()

      await expect(card).toContainText('Pool Account A')
      await expect(card).toContainText('故障切换')
      await expect(card).toContainText('Key 缺失')
      await expect(card).toContainText('暂无请求')

      await card.getByRole('combobox').first().click()
      await page.getByRole('option', { name: '轮询' }).click()
      await expect(card).toContainText('轮询')

      await card.getByRole('button', { name: '编辑上游账号 Pool Account A' }).click()
      const editDialog = page.getByRole('dialog')
      await expect(editDialog.locator('#credential-id')).toHaveCount(0)
      await editDialog.locator('#credential-name').fill('Pool Account Updated')
      await editDialog.getByRole('button', { name: '保存账号' }).click()
      await expect(editDialog).not.toBeVisible()
      await expect(card).toContainText('Pool Account Updated')

      await card.getByRole('button', { name: '删除上游账号 Pool Account Updated' }).click()
      await page.getByRole('dialog').getByRole('button', { name: '删除账号' }).click()
      await expect(page.getByRole('dialog')).not.toBeVisible()
      await expect(card).not.toContainText('Pool Account Updated')
      await expect(card).toContainText('默认凭证')
    } finally {
      await page.request.delete(
        `/admin/providers/${providerId}/credentials/${encodeURIComponent(credentialId)}`,
        { headers: csrfHeaders() },
      ).catch(() => undefined)
    }
  })
})
