import { cn } from '@/lib/utils'

interface StatusBadgeProps {
  status: string
  className?: string
}

const statusConfig: Record<string, { label: string; className: string }> = {
  active: { label: '活跃', className: 'border-emerald-200/80 bg-emerald-50 text-emerald-700 dark:border-emerald-900/70 dark:bg-emerald-950/45 dark:text-emerald-300' },
  disabled: { label: '禁用', className: 'border-slate-200/80 bg-slate-50 text-slate-600 dark:border-slate-700 dark:bg-slate-900/60 dark:text-slate-300' },
  suspended: { label: '已暂停', className: 'border-red-200/80 bg-red-50 text-red-700 dark:border-red-900/70 dark:bg-red-950/45 dark:text-red-300' },
  success: { label: '成功', className: 'border-emerald-200/80 bg-emerald-50 text-emerald-700 dark:border-emerald-900/70 dark:bg-emerald-950/45 dark:text-emerald-300' },
  error: { label: '错误', className: 'border-red-200/80 bg-red-50 text-red-700 dark:border-red-900/70 dark:bg-red-950/45 dark:text-red-300' },
  timeout: { label: '超时', className: 'border-amber-200/80 bg-amber-50 text-amber-700 dark:border-amber-900/70 dark:bg-amber-950/45 dark:text-amber-300' },
  healthy: { label: '健康', className: 'border-emerald-200/80 bg-emerald-50 text-emerald-700 dark:border-emerald-900/70 dark:bg-emerald-950/45 dark:text-emerald-300' },
  degraded: { label: '降级', className: 'border-amber-200/80 bg-amber-50 text-amber-700 dark:border-amber-900/70 dark:bg-amber-950/45 dark:text-amber-300' },
  down: { label: '离线', className: 'border-red-200/80 bg-red-50 text-red-700 dark:border-red-900/70 dark:bg-red-950/45 dark:text-red-300' },
  inactive: { label: '未激活', className: 'border-slate-200/80 bg-slate-50 text-slate-600 dark:border-slate-700 dark:bg-slate-900/60 dark:text-slate-300' },
  revoked: { label: '已吊销', className: 'border-red-200/80 bg-red-50 text-red-700 dark:border-red-900/70 dark:bg-red-950/45 dark:text-red-300' },
}

export function StatusBadge({ status, className }: StatusBadgeProps) {
  const config = statusConfig[status] || { label: status, className: 'border-border bg-muted/55 text-muted-foreground' }

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium leading-4 before:h-1.5 before:w-1.5 before:shrink-0 before:rounded-full before:bg-current before:opacity-70",
        config.className,
        className
      )}
    >
      {config.label}
    </span>
  )
}
