import { expect, test } from '@playwright/test'
import { login, requireE2EEnv } from './helpers'

test.describe('models', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
  })

  test('public model catalog only exposes active configured providers', async ({ page }) => {
    const env = requireE2EEnv()
    const providersResponse = await page.request.get('/admin/providers')
    expect(providersResponse.ok()).toBeTruthy()
    const providers = await providersResponse.json() as Array<{ displayName: string; status: string }>
    const activeProviderNames = new Set(
      providers.filter((provider) => provider.status === 'active').map((provider) => provider.displayName),
    )

    const modelsResponse = await page.request.get('/v1/models', {
      headers: { 'x-api-key': env.authToken },
    })
    expect(modelsResponse.ok()).toBeTruthy()
    const body = await modelsResponse.json() as { data: Array<{ id: string; display_name: string }> }
    expect(body.data.length).toBeGreaterThan(0)

    for (const model of body.data) {
      expect(activeProviderNames.has(model.display_name)).toBeTruthy()
    }
  })

  test('models page shows the currently usable Mimo models', async ({ page }) => {
    await page.goto('/models')
    await expect(page.getByRole('heading', { name: '模型管理' })).toBeVisible()
    await expect(page.getByText('mimo-v2.5-pro').first()).toBeVisible()
    await expect(page.getByText('Xiaomi Mimo OpenAI-Compatible').first()).toBeVisible()
  })
})
