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
    const providers = await providersResponse.json() as Array<{ id: string; status: string; models: string[] }>
    const activeProviders = providers.filter((provider) => provider.status === 'active')
    const activeProviderIds = new Set(activeProviders.map((provider) => provider.id))
    const publicModelIds = new Set(activeProviders.flatMap((provider) => provider.models))

    const aliasesResponse = await page.request.get('/admin/aliases')
    expect(aliasesResponse.ok()).toBeTruthy()
    const aliases = await aliasesResponse.json() as Array<{ alias: string; resolvedProvider: string }>
    for (const alias of aliases) {
      if (activeProviderIds.has(alias.resolvedProvider)) publicModelIds.add(alias.alias)
    }

    const modelsResponse = await page.request.get('/v1/models', {
      headers: { 'x-api-key': env.authToken },
    })
    expect(modelsResponse.ok()).toBeTruthy()
    const body = await modelsResponse.json() as { data: Array<{ id: string; display_name: string }> }
    expect(body.data.length).toBeGreaterThan(0)

    for (const model of body.data) {
      expect(publicModelIds.has(model.id)).toBeTruthy()
      expect(model.display_name.trim().length).toBeGreaterThan(0)
    }
  })

  test('models page shows the standard DeepSeek model', async ({ page }) => {
    await page.goto('/models')
    await expect(page.getByRole('heading', { name: 'Provider 与模型' })).toBeVisible()
    await expect(page.getByText('deepseek-v4-flash').first()).toBeVisible()
    await expect(page.getByText(/DeepSeek/).first()).toBeVisible()
  })

  test('exposes provider priority controls without changing the route', async ({ page }) => {
    const settingsWrites: string[] = []
    page.on('request', (request) => {
      if (new URL(request.url()).pathname === '/admin/settings' && request.method() !== 'GET') {
        settingsWrites.push(request.method())
      }
    })

    await page.goto('/models')
    await page.getByRole('tab', { name: '默认路由' }).click()

    await expect(page.getByRole('heading', { name: '默认路由策略' })).toBeVisible()
    await expect(page.getByText('Provider 解析顺序')).toBeVisible()
    await expect(page.getByRole('button', { name: /^上移 / }).first()).toBeVisible()
    await expect(page.getByRole('button', { name: /^下移 / }).first()).toBeVisible()
    expect(settingsWrites).toEqual([])
  })
})
