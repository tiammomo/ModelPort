import { useCallback, useEffect, useMemo, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useLogs, useProviders } from '@/hooks'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ErrorState } from '@/components/shared/ErrorState'
import { LoadingPage } from '@/components/shared/LoadingPage'
import { cn } from '@/lib/utils'
import { AlertTriangle, Radio, RefreshCw } from 'lucide-react'
import type { LogFilters, RequestLog } from '@/types'
import {
  clampLogPage,
  formatInteger,
  logViewSearchParams,
  logViewStateFromSearchParams,
  mergeProviderOptions,
} from './logs/log-utils'
import { LogsSummaryGrid } from './logs/LogsSummary'
import { LogsFilters } from './logs/LogsFilters'
import { LogsTable } from './logs/LogsTable'
import { LogsDrawer } from './logs/LogsDrawer'

const LOG_PAGE_SIZE_OPTIONS = [20, 50, 100, 200]
const EMPTY_LOGS: RequestLog[] = []

export function LogsPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const viewState = useMemo(() => logViewStateFromSearchParams(searchParams), [searchParams])
  const { filters, page: logPage, pageSize: logPageSize } = viewState
  const [liveMode, setLiveMode] = useState(false)
  const [selectedLog, setSelectedLog] = useState<RequestLog | null>(null)

  const { data: configuredProviders } = useProviders()
  const {
    data,
    dataUpdatedAt,
    error,
    isError,
    isFetching,
    isLoading,
    refetch,
  } = useLogs(filters, logPage, logPageSize)
  const logs = data?.logs ?? EMPTY_LOGS
  const total = data?.total || 0
  const summary = data?.summary
  const totalLogPages = Math.max(1, Math.ceil(total / logPageSize))
  const currentLogPage = clampLogPage(logPage, totalLogPages)
  const logPageStart = total === 0 ? 0 : (currentLogPage - 1) * logPageSize + 1
  const logPageEnd = Math.min(total, (currentLogPage - 1) * logPageSize + logs.length)
  const providerOptions = useMemo(
    () => mergeProviderOptions(
      configuredProviders?.map((provider) => provider.id) ?? [],
      logs,
      filters.provider,
    ),
    [configuredProviders, filters.provider, logs],
  )

  useEffect(() => {
    if (!data || isFetching || logPage === currentLogPage) return
    const timeoutId = window.setTimeout(() => {
      setSearchParams(logViewSearchParams(filters, currentLogPage, logPageSize), { replace: true })
      setSelectedLog(null)
    }, 0)
    return () => window.clearTimeout(timeoutId)
  }, [currentLogPage, data, filters, isFetching, logPage, logPageSize, setSearchParams])

  // Live mode: refetch every 3 seconds
  useEffect(() => {
    if (!liveMode) return
    const id = window.setInterval(() => {
      if (document.visibilityState === 'visible') void refetch()
    }, 3000)
    return () => clearInterval(id)
  }, [liveMode, refetch])

  const handleFiltersChange = (next: LogFilters) => {
    if (next.dateTo) setLiveMode(false)
    setSearchParams(logViewSearchParams(next, 1, logPageSize), { replace: true })
    setSelectedLog(null)
  }

  const handleLogPageChange = (page: number) => {
    setSearchParams(
      logViewSearchParams(filters, Math.min(Math.max(page, 1), totalLogPages), logPageSize),
      { replace: true },
    )
    setSelectedLog(null)
  }

  const handleLogPageSizeChange = (pageSize: number) => {
    setSearchParams(logViewSearchParams(filters, 1, pageSize), { replace: true })
    setSelectedLog(null)
  }

  const handleLiveToggle = () => {
    const nextLiveMode = !liveMode
    if (nextLiveMode && filters.dateTo) {
      handleFiltersChange({ ...filters, dateTo: undefined })
    }
    setLiveMode(nextLiveMode)
  }

  const toggleErrorsOnly = () => {
    handleFiltersChange({
      ...filters,
      status: filters.status === 'error' ? undefined : 'error',
    })
  }

  const closeDrawer = useCallback(() => setSelectedLog(null), [])

  if (isError && !data) {
    return (
      <ErrorState
        title="请求日志加载失败"
        message={error instanceof Error ? error.message : '无法读取持久化请求日志，请检查后端与登录状态。'}
        onRetry={() => void refetch()}
      />
    )
  }

  if (isLoading) {
    return <LoadingPage />
  }

  const updatedAt = dataUpdatedAt
    ? new Date(dataUpdatedAt).toLocaleTimeString('zh-CN', {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: false,
      })
    : '—'

  return (
    <div className="space-y-5">
      {/* Page header */}
      <section>
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0 space-y-1">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline" className="gap-1.5 border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-300">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                持久化日志
              </Badge>
              <Badge variant="outline" className="border-slate-200 bg-slate-50 text-slate-700 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300">
                {formatInteger(total)} 条结果
              </Badge>
            </div>
            <h1 className="text-2xl font-bold tracking-tight">请求日志</h1>
            <p className="max-w-3xl text-sm text-muted-foreground">
              从真实请求记录中排查路由、身份、缓存、计费与延迟；最后更新于 {updatedAt}。
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant={filters.status === 'error' ? 'secondary' : 'outline'}
              size="sm"
              onClick={toggleErrorsOnly}
              aria-pressed={filters.status === 'error'}
            >
              <AlertTriangle className="h-3.5 w-3.5" />
              只看错误
            </Button>
            <Button
              variant={liveMode ? 'default' : 'outline'}
              size="sm"
              className={cn(
                'gap-1.5 transition-all',
                liveMode && 'bg-emerald-600 hover:bg-emerald-700 text-white',
              )}
              onClick={handleLiveToggle}
              aria-pressed={liveMode}
            >
              {isFetching
                ? <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                : <Radio className={cn('h-3.5 w-3.5', liveMode && 'animate-pulse')} />}
              {liveMode ? '自动刷新中' : '自动刷新'}
            </Button>
          </div>
        </div>
      </section>

      {liveMode && (
        <div className="rounded-lg border border-emerald-200 bg-emerald-50 px-4 py-2 text-xs text-emerald-800 dark:border-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-200" role="status">
          每 3 秒刷新当前结果。为接收新请求，已移除固定结束时间；切换到后台标签页时会暂停请求。
        </div>
      )}

      {isError && data && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200" role="alert">
          <span>刷新失败，当前显示的是上次成功加载的数据（{updatedAt}）。</span>
          <Button variant="outline" size="sm" onClick={() => void refetch()}>
            <RefreshCw className="h-3.5 w-3.5" />
            重试
          </Button>
        </div>
      )}

      {/* Summary cards */}
      <LogsSummaryGrid summary={summary} />

      {/* Filters */}
      <LogsFilters
        filters={filters}
        onFiltersChange={handleFiltersChange}
        providers={providerOptions}
      />

      {/* Table */}
      <LogsTable
        logs={logs}
        total={total}
        page={currentLogPage}
        pageSize={logPageSize}
        totalPages={totalLogPages}
        start={logPageStart}
        end={logPageEnd}
        pageSizeOptions={LOG_PAGE_SIZE_OPTIONS}
        isLoading={isLoading}
        onPageChange={handleLogPageChange}
        onPageSizeChange={handleLogPageSizeChange}
        onSelectLog={setSelectedLog}
      />

      {/* Detail drawer */}
      <LogsDrawer
        log={selectedLog}
        onClose={closeDrawer}
      />
    </div>
  )
}
