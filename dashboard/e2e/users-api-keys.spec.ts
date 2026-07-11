import { expect, test } from '@playwright/test'
import { cleanupE2EUsers, csrfHeaders, login, requireE2EEnv } from './helpers'

test.describe('users and API keys', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
    await cleanupE2EUsers(page)
  })

  test.afterEach(async ({ page }) => {
    await cleanupE2EUsers(page)
  })

  test('admin can see, edit, and clean up a user and API key', async ({ page }) => {
    const suffix = Date.now()
    const username = `e2e_user_${suffix}`
    const email = `${username}@modelport.local`
    const updatedEmail = `${username}+updated@modelport.local`
    const password = 'e2e-password-12345'

    const createUserResponse = await page.request.post('/admin/users', {
      headers: csrfHeaders(),
      data: {
        username,
        email,
        password,
        role: 'user',
        status: 'active',
      },
    })
    expect(createUserResponse.ok()).toBeTruthy()
    const user = await createUserResponse.json() as { id: string }

    const keyName = `e2e-key-${suffix}`
    const createKeyResponse = await page.request.post('/admin/api-keys', {
      headers: csrfHeaders(),
      data: {
        userId: user.id,
        username,
        name: keyName,
        group: 'e2e',
        allowedModels: ['deepseek-v4-flash'],
        allowedProviders: ['deepseek'],
      },
    })
    expect(createKeyResponse.ok()).toBeTruthy()

    await page.goto('/users')
    const userRow = page.getByRole('row').filter({ hasText: username })
    await expect(userRow).toBeVisible()
    await userRow.getByRole('button').click()
    await page.getByRole('menuitem', { name: /编辑用户/ }).click()

    const dialog = page.getByRole('dialog', { name: '编辑用户' })
    await expect(dialog).toBeVisible()
    await dialog.getByLabel('邮箱').fill(updatedEmail)
    await dialog.getByLabel('新密码').fill('e2e-password-updated-12345')
    await dialog.getByRole('button', { name: '保存更改' }).click()
    await expect(dialog).toBeHidden()
    await expect(page.getByRole('row').filter({ hasText: updatedEmail })).toBeVisible()

    await page.goto('/api-keys')
    await page.getByPlaceholder('搜索名称、用户、密钥标识或项目').fill(keyName)
    const keyRow = page.getByRole('row').filter({ hasText: keyName })
    await expect(keyRow).toBeVisible()
    await expect(keyRow).toContainText(username)

    const apiKeysResponse = await page.request.get('/admin/api-keys')
    expect(apiKeysResponse.ok()).toBeTruthy()
    const apiKeys = await apiKeysResponse.json() as Array<{
      name: string
      allowedModels?: string[]
      allowedProviders?: string[]
    }>
    const savedKey = apiKeys.find((apiKey) => apiKey.name === keyName)
    expect(savedKey?.allowedModels).toContain('deepseek-v4-flash')
    expect(savedKey?.allowedProviders).toContain('deepseek')
  })

  test('new client key reveal provides one-time ready-to-copy Anthropic settings', async ({ page }) => {
    const suffix = Date.now()
    const username = `e2e_reveal_${suffix}`
    const createUserResponse = await page.request.post('/admin/users', {
      headers: csrfHeaders(),
      data: {
        username,
        email: `${username}@modelport.local`,
        password: 'e2e-password-12345',
        role: 'user',
        status: 'active',
      },
    })
    expect(createUserResponse.ok()).toBeTruthy()

    await page.goto('/api-keys')
    await page.getByRole('button', { name: '创建密钥' }).click()
    const dialog = page.getByRole('dialog', { name: '创建 API 密钥' })
    await dialog.locator('#create-key-user').click()
    await page.getByRole('option', { name: new RegExp(username) }).click()
    await dialog.getByLabel('名称').fill(`e2e_reveal_key_${suffix}`)
    await dialog.getByRole('button', { name: '创建密钥' }).click()

    const revealedKey = dialog.locator('#new-api-key')
    await expect(revealedKey).toHaveValue(/^sk-mp-/)
    await expect(dialog.getByText('完整密钥只显示这一次。', { exact: false })).toBeVisible()
    await expect(dialog.getByText('Claude Code / Anthropic SDK')).toBeVisible()
    await expect(dialog.getByText(/OpenAI-compatible 是上游适配能力/)).toBeVisible()
    await dialog.getByRole('button', { name: '已保存，关闭' }).click()
    await expect(dialog).toBeHidden()
  })
})
