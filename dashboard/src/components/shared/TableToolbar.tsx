import { cn } from '@/lib/utils'

interface TableToolbarProps {
  children: React.ReactNode
  actions?: React.ReactNode
  className?: string
}

export function TableToolbar({ children, actions, className }: TableToolbarProps) {
  return (
    <div className={cn("flex flex-wrap items-center justify-between gap-3", className)}>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-3">
        {children}
      </div>
      {actions && (
        <div className="flex shrink-0 items-center justify-center gap-2">
          {actions}
        </div>
      )}
    </div>
  )
}
