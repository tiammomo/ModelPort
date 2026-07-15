import * as React from "react"
import { cn } from "@/lib/utils"

const Badge = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement> & {
    variant?: 'default' | 'secondary' | 'destructive' | 'outline' | 'success' | 'warning'
  }
>(({ className, variant = 'default', ...props }, ref) => {
  const variants: Record<string, string> = {
    default: "border-primary/20 bg-primary/10 text-primary dark:bg-primary/15",
    secondary: "border-border/70 bg-secondary/80 text-secondary-foreground",
    destructive: "border-red-200/80 bg-red-50 text-red-700 dark:border-red-900/70 dark:bg-red-950/45 dark:text-red-300",
    outline: "border-border/85 bg-background/70 text-foreground/80",
    success: "border-emerald-200/80 bg-emerald-50 text-emerald-700 dark:border-emerald-900/70 dark:bg-emerald-950/45 dark:text-emerald-300",
    warning: "border-amber-200/80 bg-amber-50 text-amber-700 dark:border-amber-900/70 dark:bg-amber-950/45 dark:text-amber-300",
  }

  return (
    <div
      ref={ref}
      className={cn(
        "inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium leading-4 transition-colors focus:outline-none focus:ring-[3px] focus:ring-ring/20",
        variants[variant],
        className
      )}
      {...props}
    />
  )
})
Badge.displayName = "Badge"

export { Badge }
