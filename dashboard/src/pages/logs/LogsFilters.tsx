import { useMemo, type ElementType, type ReactNode } from 'react'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Badge } from '@/components/ui/badge'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'
import {
  Activity,
  CalendarClock,
  Filter,
  RotateCcw,
  Search,
  Server,
  UserRound,
  WalletCards,
  Zap,
} from 'lucide-react'
import type { LogFilters, RequestLog, RequestStatus, StreamMode } from '@/types'
import { extractProviders, timeRangeToDates, type TimeRange } from './log-utils'

const ALL = '__all__'

const TIME_RANGE_OPTIONS: { value: TimeRange; label: string }[] = [
  { value: '1h', label: '最近1h' },
  { value: '6h', label: '最近6h' },
  { value: '24h', label: '最近24h' },
  { value: '7d', label: '最近7d' },
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
}: {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  className?: string
}) {
  return (
    <Input
      placeholder={placeholder}
      className={className}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  )
}

// ── Main filters component ───────────────────────────────────────

export function LogsFilters({
  filters,
  onFiltersChange,
  logs,
}: {
  filters: LogFilters
  onFiltersChange: (next: LogFilters) => void
  logs: RequestLog[]
}) {
  const activeFilterCount = useMemo(
    () => Object.values(filters).filter(Boolean).length,
    [filters],
  )

  const providers = useMemo(() => extractProviders(logs), [logs])

  const update = (patch: Partial<LogFilters>) => {
    onFiltersChange({ ...filters, ...patch })
  }

  const handleTimeRange = (range: TimeRange) => {
    const { dateFrom, dateTo } = timeRangeToDates(range)
    onFiltersChange({ ...filters, dateFrom, dateTo })
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
              onClick={() => onFiltersChange({})}
            >
              <RotateCcw className="h-4 w-4" />
              重置
            </Button>
          </div>
        </div>

        {/* Quick time range */}
        <div className="flex flex-wrap items-center gap-2 border-b bg-muted/10 px-4 py-2.5">
          <span className="text-xs font-medium text-muted-foreground">快速时间:</span>
          {TIME_RANGE_OPTIONS.map((opt) => {
            const dates = timeRangeToDates(opt.value)
            const isActive = filters.dateFrom === dates.dateFrom && filters.dateTo === dates.dateTo
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
        </div>

        {/* Filter fields */}
        <div className="grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-12">
          <FilterField label="开始时间" icon={CalendarClock} className="xl:col-span-2">
            <Input
              type="datetime-local"
              className="h-10"
              value={filters.dateFrom || ''}
              onChange={(e) => update({ dateFrom: e.target.value || undefined })}
            />
          </FilterField>

          <FilterField label="结束时间" icon={CalendarClock} className="xl:col-span-2">
            <Input
              type="datetime-local"
              className="h-10"
              value={filters.dateTo || ''}
              onChange={(e) => update({ dateTo: e.target.value || undefined })}
            />
          </FilterField>

          <FilterField label="关键词" icon={Search} className="md:col-span-2 xl:col-span-3">
            <div className="relative">
              <Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
              <DebouncedInput
                placeholder="模型、渠道、令牌、请求 ID"
                className="h-10 pl-9"
                value={filters.search || ''}
                onChange={(v) => update({ search: v || undefined })}
              />
            </div>
          </FilterField>

          <FilterField label="用户" icon={UserRound} className="xl:col-span-2">
            <DebouncedInput
              placeholder="用户名"
              className="h-10"
              value={filters.username || ''}
              onChange={(v) => update({ username: v || undefined })}
            />
          </FilterField>

          <FilterField label="分组" icon={WalletCards} className="xl:col-span-3">
            <DebouncedInput
              placeholder="API Key 分组"
              className="h-10"
              value={filters.group || ''}
              onChange={(v) => update({ group: v || undefined })}
            />
          </FilterField>

          <FilterField label="渠道" icon={Server} className="xl:col-span-2">
            <Select
              value={filters.provider || ALL}
              onValueChange={(v) => update({ provider: v === ALL ? undefined : v })}
            >
              <SelectTrigger className="h-10">
                <SelectValue placeholder="全部渠道" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>全部渠道</SelectItem>
                {providers.map((p) => (
                  <SelectItem key={p} value={p}>
                    {p.charAt(0).toUpperCase() + p.slice(1)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FilterField>

          <FilterField label="状态" icon={Activity} className="xl:col-span-2">
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

          <FilterField label="模式" icon={Zap} className="xl:col-span-2">
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
      </CardContent>
    </Card>
  )
}
