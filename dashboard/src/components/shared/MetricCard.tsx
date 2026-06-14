import { cn } from '@/lib/utils'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from './Skeleton'
import { Sparkline } from './Sparkline'
import { AnimatedNumber } from './AnimatedNumber'
import { type LucideIcon } from 'lucide-react'

interface MetricCardProps {
  title: string
  value: string | number
  description?: string
  icon?: LucideIcon
  trend?: { value: number; label: string }
  sparkline?: number[]
  loading?: boolean
  className?: string
}

export function MetricCard({
  title,
  value,
  description,
  icon: Icon,
  trend,
  sparkline,
  loading,
  className,
}: MetricCardProps) {
  if (loading) {
    return (
      <Card className={cn('transition-all duration-200', className)}>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <Skeleton className="h-4 w-24" />
          <Skeleton className="h-8 w-8 rounded-lg" />
        </CardHeader>
        <CardContent className="space-y-2">
          <Skeleton className="h-8 w-28" />
          <Skeleton className="h-3 w-36" />
        </CardContent>
      </Card>
    )
  }

  const numericValue = typeof value === 'number' ? value : undefined

  return (
    <Card
      className={cn(
        'group transition-all duration-200 hover:shadow-md hover:-translate-y-0.5',
        className,
      )}
    >
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">{title}</CardTitle>
        {Icon && (
          <div className="rounded-lg bg-primary/10 p-2 transition-colors group-hover:bg-primary/20">
            <Icon className="h-4 w-4 text-primary" />
          </div>
        )}
      </CardHeader>
      <CardContent>
        <div className="flex items-end justify-between gap-3">
          <div className="min-w-0 break-words text-2xl font-bold leading-tight tabular-nums tracking-tight">
            {numericValue !== undefined ? (
              <AnimatedNumber value={numericValue} />
            ) : (
              value
            )}
          </div>
          {sparkline && sparkline.length > 0 && (
            <Sparkline data={sparkline} width={72} height={28} className="opacity-70" />
          )}
        </div>
        {(description || trend) && (
          <p className="mt-1 flex items-start gap-1.5 text-xs leading-relaxed text-muted-foreground">
            {trend && (
              <span
                className={cn(
                  'inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-semibold',
                  trend.value >= 0
                    ? 'bg-green-500/10 text-green-600 dark:text-green-400'
                    : 'bg-red-500/10 text-red-600 dark:text-red-400',
                )}
              >
                {trend.value >= 0 ? '↑' : '↓'} {Math.abs(trend.value)}%
              </span>
            )}
            <span className="min-w-0 break-words">{trend?.label || description}</span>
          </p>
        )}
      </CardContent>
    </Card>
  )
}
