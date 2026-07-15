import { expect, test } from '@playwright/test'
import { login, requireE2EEnv } from './helpers'

test.describe('enterprise operations', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
  })

  test('loads ledger evidence and opens budget details without writing state', async ({ page }) => {
    const enterpriseWrites: string[] = []
    page.on('request', (request) => {
      const url = new URL(request.url())
      if (url.pathname.startsWith('/admin/enterprise/') && request.method() !== 'GET') {
        enterpriseWrites.push(`${request.method()} ${url.pathname}`)
      }
    })

    const overviewResponse = page.waitForResponse((response) => (
      new URL(response.url()).pathname === '/admin/enterprise/overview' && response.ok()
    ))
    const budgetResponse = page.waitForResponse((response) => (
      new URL(response.url()).pathname === '/admin/enterprise/budget' && response.ok()
    ))
    const requestsResponse = page.waitForResponse((response) => (
      new URL(response.url()).pathname === '/admin/enterprise/requests' && response.ok()
    ))

    await page.goto('/enterprise')
    await Promise.all([overviewResponse, budgetResponse, requestsResponse])

    await expect(page.getByRole('heading', { name: '企业运行' })).toBeVisible()
    await expect(page.getByRole('region', { name: '企业账本概览' })).toBeVisible()
    await expect(page.getByRole('region', { name: '企业预算控制' })).toContainText('事务预算控制')
    await expect(page.getByRole('region', { name: '企业请求记录' })).toContainText('Gateway Requests')
    await expect(page.getByLabel('搜索企业账本')).toBeVisible()

    await page.getByRole('button', { name: '管理' }).click()
    const budgetDialog = page.getByRole('dialog', { name: '事务预算与证据' })
    await expect(budgetDialog).toBeVisible()
    await expect(budgetDialog.getByText('推理硬上限')).toBeVisible()
    await expect(budgetDialog.getByText('人工账务调整')).toBeVisible()
    await expect(budgetDialog.getByText('最近证据事件')).toBeVisible()

    await page.keyboard.press('Escape')
    await expect(budgetDialog).toBeHidden()
    expect(enterpriseWrites).toEqual([])
  })
})
