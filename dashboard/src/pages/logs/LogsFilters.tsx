import { useEffect, useMemo, useRef, useState, type ElementType, type ReactNode } from 'react'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'
import {
  Activity,
  CalendarClock,
  ChevronDown,
  Filter,
  RotateCcw,
  Search,
  Server,
  UserRound,
  WalletCards,
  Zap,
} from 'lucide-react'
import type { LogFilters, RequestStatus, StreamMode } from '@/types'
import { timeRangeToDates, type TimeRange } from './log-utils'

const ALL = '__all__'

const TIME_RANGE_OPTIONS: { value: TimeRange; label: string }[] = [
  { value: '1h', label: '最近 1 小时' },
  { value: '6h', label: '最近 6 小时' },
  { value: '24h', label: '最近 24 小时' },
  { value: '7d', label: '最近 7 天' },
]

// ── Filter field wrapper ─────────────────────────────────────────

function FilterField({
  label,
  icon: Icon,
  children,
  className,
}: {
  label: string
  icon: ElementType
  children: ReactNode
  className?: string
}) {
  return (
    <div className={cn('space-y-1.5', className)}>
      <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Icon className="h-3.5 w-3.5" />
        <span>{label}</span>
      </div>
      {children}
    </div>
  )
}

// ── Debounced text input ─────────────────────────────────────────

function DebouncedInput({
  value,
  onChange,
  placeholder,
  className,
  ariaLabel,
}: {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  className?: string
  ariaLabel: string
}) {
  const inputRef = useRef<HTMLInputElement>(null)
  const timeoutRef = useRef<ReturnType<typeof setTimeout>>(undefined)
  const onChangeRef = useRef(onChange)

  useEffect(() => {
    onChangeRef.current = onChange
  }, [onChange])

  useEffect(() => {
    if (inputRef.current && inputRef.current.value !== value) {
      if (timeoutRef.current) clearTimeout(timeoutRef.current)
      inputRef.current.value = value
    }
  }, [value])

  useEffect(() => () => {
    if (timeoutRef.current) clearTimeout(timeoutRef.current)
  }, [])

  return (
    <Input
      ref={inputRef}
      placeholder={placeholder}
      className={className}
      defaultValue={value}
      aria-label={ariaLabel}
      onChange={(event) => {
        if (timeoutRef.current) clearTimeout(timeoutRef.current)
        const nextValue = event.target.value
        timeoutRef.current = setTimeout(() => onChangeRef.current(nextValue), 350)
      }}
    />
  )
}

// ── Main filters component ───────────────────────────────────────

export function LogsFilters({
  filters,
  onFiltersChange,
  providers,
}: {
  filters: LogFilters
  onFiltersChange: (next: LogFilters) => void
  providers: string[]
}) {
  const [advancedOpen, setAdvancedOpen] = useState(false)
  const [quickRange, setQuickRange] = useState<{
    value: TimeRange
    dateFrom: string
    dateTo: string
  } | null>(null)
  const activeFilterCount = useMemo(
    () => Object.values(filters).filter(Boolean).length,
    [filters],
  )
  const advancedFilterCount = [filters.dateFrom, filters.dateTo, filters.username, filters.group, filters.stream]
    .filter(Boolean).length

  const update = (patch: Partial<LogFilters>) => {
    onFiltersChange({ ...filters, ...patch })
  }

  const handleTimeRange = (range: TimeRange) => {
    const { dateFrom, dateTo } = timeRangeToDates(range)
    setQuickRange({ value: range, dateFrom, dateTo })
    onFiltersChange({ ...filters, dateFrom, dateTo })
  }

  const reset = () => {
    setQuickRange(null)
    onFiltersChange({})
  }

  return (
    <Card className="overflow-hidden rounded-lg shadow-sm">
      <CardContent className="p-0">
        {/* Header */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b bg-muted/20 px-4 py-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <Filter className="h-4 w-4" />
            </div>
            <div>
              <p className="text-sm font-semibold">筛选条件</p>
              <p className="text-xs text-muted-foreground">时间 · 来源 · 身份 · 状态</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {activeFilterCount > 0 && (
              <Badge variant="outline" className="border-primary/20 bg-primary/5 text-primary">
                已启用 {activeFilterCount}
              </Badge>
            )}
            <Button
              variant="outline"
              size="sm"
              disabled={activeFilterCount === 0}
              onClick={reset}
            >
              <RotateCcw className="h-4 w-4" />
              重置
            </Button>
          </div>
        </div>

        {/* Quick time range */}
        <div className="flex flex-wrap items-center gap-2 border-b bg-muted/10 px-4 py-2.5">
          <span className="text-xs font-medium text-muted-foreground">快速范围</span>
          {TIME_RANGE_OPTIONS.map((opt) => {
            const isActive = quickRange?.value === opt.value
              && quickRange.dateFrom === filters.dateFrom
              && quickRange.dateTo === filters.dateTo
            return (
              <Button
                key={opt.value}
                variant={isActive ? 'default' : 'outline'}
                size="sm"
                className="h-7 text-xs"
                onClick={() => handleTimeRange(opt.value)}
              >
                {opt.label}
              </Button>
            )
          })}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="ml-auto h-7 gap-1 text-xs"
            onClick={() => setAdvancedOpen((open) => !open)}
            aria-expanded={advancedOpen}
          >
            更多筛选
            {advancedFilterCount > 0 && <Badge variant="secondary" className="px-1.5">{advancedFilterCount}</Badge>}
            <ChevronDown className={cn('h-3.5 w-3.5 transition-transform', advancedOpen && 'rotate-180')} />
          </Button>
        </div>

        {/* Primary filters */}
        <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-12">
          <FilterField label="关键词" icon={Search} className="md:col-span-2 xl:col-span-6">
            <div className="relative">
              <Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
              <DebouncedInput
                placeholder="模型、渠道、令牌、请求 ID"
                className="h-10 pl-9"
                value={filters.search || ''}
                onChange={(v) => update({ search: v || undefined })}
                ariaLabel="搜索请求日志"
              />
            </div>
          </FilterField>
          <FilterField label="Provider" icon={Server} className="xl:col-span-3">
            <Select
              value={filters.provider || ALL}
              onValueChange={(v) => update({ provider: v === ALL ? undefined : v })}
            >
              <SelectTrigger className="h-10">
                <SelectValue placeholder="全部 Provider" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>全部 Provider</SelectItem>
                {providers.map((p) => (
                  <SelectItem key={p} value={p}>
                    {p.charAt(0).toUpperCase() + p.slice(1)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FilterField>

          <FilterField label="状态" icon={Activity} className="xl:col-span-3">
            <Select
              value={filters.status || ALL}
              onValueChange={(v) =>
                update({ status: v === ALL ? undefined : (v as RequestStatus) })
              }
            >
              <SelectTrigger className="h-10">
                <SelectValue placeholder="全部状态" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>全部状态</SelectItem>
                <SelectItem value="success">成功</SelectItem>
                <SelectItem value="error">错误</SelectItem>
                <SelectItem value="timeout">超时</SelectItem>
              </SelectContent>
            </Select>
          </FilterField>

        </div>

        {advancedOpen && (
          <div className="grid gap-3 border-t bg-muted/[0.06] p-4 md:grid-cols-2 xl:grid-cols-12">
            <FilterField label="开始时间" icon={CalendarClock} className="xl:col-span-3">
              <Input
                type="datetime-local"
                aria-label="日志开始时间"
                className="h-10"
                value={filters.dateFrom || ''}
                onChange={(e) => {
                  setQuickRange(null)
                  update({ dateFrom: e.target.value || undefined })
                }}
              />
            </FilterField>

            <FilterField label="结束时间" icon={CalendarClock} className="xl:col-span-3">
              <Input
                type="datetime-local"
                aria-label="日志结束时间"
                className="h-10"
                value={filters.dateTo || ''}
                onChange={(e) => {
                  setQuickRange(null)
                  update({ dateTo: e.target.value || undefined })
                }}
              />
            </FilterField>

            <FilterField label="用户" icon={UserRound} className="xl:col-span-2">
              <DebouncedInput
                placeholder="用户名"
                className="h-10"
                value={filters.username || ''}
                onChange={(v) => update({ username: v || undefined })}
                ariaLabel="按用户名筛选"
              />
            </FilterField>

            <FilterField label="API Key 标签" icon={WalletCards} className="xl:col-span-2">
              <DebouncedInput
                placeholder="例如：研发组"
                className="h-10"
                value={filters.group || ''}
                onChange={(v) => update({ group: v || undefined })}
                ariaLabel="按 API Key 标签筛选"
              />
            </FilterField>

            <FilterField label="响应模式" icon={Zap} className="xl:col-span-2">
              <Select
                value={filters.stream || ALL}
                onValueChange={(v) =>
                  update({ stream: v === ALL ? undefined : (v as StreamMode) })
                }
              >
                <SelectTrigger className="h-10">
                  <SelectValue placeholder="全部模式" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL}>全部模式</SelectItem>
                  <SelectItem value="stream">流式</SelectItem>
                  <SelectItem value="non-stream">非流式</SelectItem>
                </SelectContent>
              </Select>
            </FilterField>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
