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
        allowedModels: ['mimo-v2.5-pro'],
        allowedProviders: ['mimo'],
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
    await dialog.getByPlaceholder('输入邮箱').fill(updatedEmail)
    await dialog.getByPlaceholder('留空不修改').fill('e2e-password-updated-12345')
    await dialog.getByRole('button', { name: '保存' }).click()
    await expect(dialog).toBeHidden()
    await expect(page.getByRole('row').filter({ hasText: updatedEmail })).toBeVisible()

    await page.goto('/api-keys')
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
    expect(savedKey?.allowedModels).toContain('mimo-v2.5-pro')
    expect(savedKey?.allowedProviders).toContain('mimo')
  })
})
