import { cn } from '@/lib/utils'
import { Card, CardContent } from '@/components/ui/card'
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
        <CardContent className="p-4 sm:p-5">
          <div className="flex items-start gap-4">
            <Skeleton className="h-10 w-10 shrink-0 rounded-lg" />
            <div className="min-w-0 flex-1 space-y-2">
              <Skeleton className="h-4 w-24" />
              <Skeleton className="h-8 w-28" />
              <Skeleton className="h-3 w-36" />
            </div>
          </div>
        </CardContent>
      </Card>
    )
  }

  const numericValue = typeof value === 'number' ? value : undefined

  return (
    <Card
      className={cn(
        'group transition-all duration-200 motion-safe:hover:-translate-y-0.5 motion-safe:hover:shadow-md',
        className,
      )}
    >
      <CardContent className="p-4 sm:p-5">
        <div className="flex items-start gap-2.5 sm:gap-4">
          {Icon && (
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary transition-colors group-hover:bg-primary/20 sm:h-10 sm:w-10">
              <Icon className="h-4 w-4 sm:h-5 sm:w-5" />
            </div>
          )}

          <div className="min-w-0 flex-1">
            <div className="flex items-end justify-between gap-3">
              <div className="min-w-0">
                <p className="whitespace-nowrap text-xs font-medium text-muted-foreground sm:text-sm">{title}</p>
                <div className="mt-2 whitespace-nowrap text-lg font-bold leading-tight tabular-nums tracking-tight sm:mt-3 sm:text-2xl">
                  {numericValue !== undefined ? (
                    <AnimatedNumber value={numericValue} />
                  ) : (
                    value
                  )}
                </div>
              </div>
              {sparkline && sparkline.length > 0 && (
                <Sparkline data={sparkline} width={52} height={28} className="hidden shrink-0 opacity-70 xl:block" />
              )}
            </div>

            {(description || trend) && (
              <p className="mt-2 flex items-start gap-1.5 text-xs leading-relaxed text-muted-foreground">
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
          </div>
        </div>
      </CardContent>
    </Card>
  )
}
