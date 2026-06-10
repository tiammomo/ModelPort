import { cn } from '@/lib/utils'

interface StatusBadgeProps {
  status: string
  className?: string
}

const statusConfig: Record<string, { label: string; className: string }> = {
  active: { label: '活跃', className: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300' },
  disabled: { label: '禁用', className: 'bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300' },
  suspended: { label: '已暂停', className: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300' },
  success: { label: '成功', className: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300' },
  error: { label: '错误', className: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300' },
  timeout: { label: '超时', className: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-300' },
  healthy: { label: '健康', className: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300' },
  degraded: { label: '降级', className: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-300' },
  down: { label: '离线', className: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300' },
  inactive: { label: '未激活', className: 'bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300' },
  revoked: { label: '已吊销', className: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300' },
}

export function StatusBadge({ status, className }: StatusBadgeProps) {
  const config = statusConfig[status] || { label: status, className: 'bg-gray-100 text-gray-800' }

  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium",
        config.className,
        className
      )}
    >
      {config.label}
    </span>
  )
}
