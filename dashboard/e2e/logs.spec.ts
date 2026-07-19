import { expect, test } from '@playwright/test'
import { login, requireE2EEnv } from './helpers'

test.describe('request logs', () => {
  test.beforeEach(async ({ page }) => {
    await login(page, requireE2EEnv())
  })

  test('debounces search and automatic refresh removes a fixed end time', async ({ page }) => {
    const defaultRangeResponse = page.waitForResponse((response) => {
      const url = new URL(response.url())
      return url.pathname === '/admin/logs'
        && url.searchParams.has('dateFrom')
        && url.searchParams.has('dateTo')
        && response.ok()
    })
    await page.goto('/logs')
    await defaultRangeResponse
    await expect(page.getByRole('heading', { name: '请求日志' })).toBeVisible()
    await expect(page.getByRole('button', { name: '最近 24 小时' })).toHaveAttribute('aria-pressed', 'true')

    await Promise.all([
      page.waitForResponse((response) => {
        const url = new URL(response.url())
        return url.pathname === '/admin/logs'
          && url.searchParams.has('dateFrom')
          && url.searchParams.has('dateTo')
          && response.ok()
      }),
      page.getByRole('button', { name: '最近 1 小时' }).click(),
    ])

    const searchRequests: string[] = []
    page.on('request', (request) => {
      const url = new URL(request.url())
      if (url.pathname === '/admin/logs' && url.searchParams.has('search')) {
        searchRequests.push(url.searchParams.get('search') || '')
      }
    })
    await page.getByLabel('搜索请求日志').pressSequentially('demo-chat', { delay: 20 })
    await page.waitForResponse((response) => {
      const url = new URL(response.url())
      return url.pathname === '/admin/logs'
        && url.searchParams.get('search') === 'demo-chat'
        && response.ok()
    })
    expect(searchRequests).toEqual(['demo-chat'])
    await expect(page).toHaveURL(/search=demo-chat/)

    await page.reload({ waitUntil: 'networkidle' })
    await expect(page.getByLabel('搜索请求日志')).toHaveValue('demo-chat')

    await page.getByRole('button', { name: '自动刷新' }).click()
    await expect(page.getByRole('status')).toContainText('每 3 秒刷新当前结果')
    await page.getByRole('button', { name: /更多筛选/ }).click()
    await expect(page.getByLabel('日志结束时间')).toHaveValue('')
    expect(new URL(page.url()).searchParams.has('dateTo')).toBeFalsy()
  })

  test('uses mobile cards and exposes truthful accessible request detail', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto('/logs')
    await expect(page.getByRole('heading', { name: '请求日志' })).toBeVisible()

    const detailButton = page.getByRole('button', { name: /查看 .* 请求详情/ }).first()
    await expect(detailButton).toBeVisible()
    await detailButton.click()

    const drawer = page.getByRole('dialog', { name: '请求详情' })
    await expect(drawer).toBeVisible()
    await drawer.getByRole('tab', { name: '协议 Trace' }).click()
    await expect(drawer.getByText(/仅呈现服务端已保存字段/)).toBeVisible()
    await drawer.getByRole('tab', { name: 'Tool Use' }).click()
    await expect(drawer.getByText('本次请求没有工具级遥测')).toBeVisible()

    const hasPageOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth)
    expect(hasPageOverflow).toBeFalsy()
    await page.keyboard.press('Escape')
    await expect(drawer).toBeHidden()
    await expect(detailButton).toBeFocused()
  })

  test('uses the available desktop width without empty side gutters', async ({ page }) => {
    await page.setViewportSize({ width: 2048, height: 960 })
    await page.goto('/logs')
    await expect(page.getByRole('heading', { name: '请求日志' })).toBeVisible()

    const headerGrid = page.getByTestId('logs-table-header-grid')
    await expect(headerGrid).toBeVisible()

    const layout = await headerGrid.evaluate((element) => {
      const grid = element.getBoundingClientRect()
      const cells = Array.from(element.children, (child) => child.getBoundingClientRect())
      return {
        width: grid.width,
        leftGap: cells[0].left - grid.left,
        rightGap: grid.right - cells[cells.length - 1].right,
      }
    })

    expect(layout.width).toBeGreaterThan(1600)
    expect(layout.leftGap).toBeLessThanOrEqual(17)
    expect(layout.rightGap).toBeLessThanOrEqual(17)
  })
})
