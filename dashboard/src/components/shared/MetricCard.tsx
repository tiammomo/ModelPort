import { cn } from '@/lib/utils'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { type LucideIcon } from 'lucide-react'

interface MetricCardProps {
  title: string
  value: string | number
  description?: string
  icon?: LucideIcon
  trend?: { value: number; label: string }
  className?: string
}

export function MetricCard({ title, value, description, icon: Icon, trend, className }: MetricCardProps) {
  return (
    <Card className={cn("", className)}>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-sm font-medium">{title}</CardTitle>
        {Icon && <Icon className="h-4 w-4 text-muted-foreground" />}
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{value}</div>
        {(description || trend) && (
          <p className="text-xs text-muted-foreground mt-1">
            {trend && (
              <span className={cn("mr-1", trend.value >= 0 ? "text-green-600" : "text-red-600")}>
                {trend.value >= 0 ? '+' : ''}{trend.value}%
              </span>
            )}
            {trend?.label || description}
          </p>
        )}
      </CardContent>
    </Card>
  )
}
