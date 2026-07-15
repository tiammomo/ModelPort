import * as React from "react"
import { cn } from "@/lib/utils"

const Input = React.forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          "flex h-9 w-full rounded-lg border border-input/85 bg-background/85 px-3 py-1 text-sm shadow-[0_1px_2px_oklch(0.2_0.02_230/0.04)] transition-[border-color,box-shadow,background-color] duration-150 ease-out file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground/80 hover:border-foreground/20 focus-visible:border-primary/55 focus-visible:bg-background focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/12 aria-[invalid=true]:border-destructive/60 aria-[invalid=true]:ring-destructive/10 disabled:cursor-not-allowed disabled:bg-muted/45 disabled:opacity-60",
          className
        )}
        ref={ref}
        {...props}
      />
    )
  }
)
Input.displayName = "Input"

export { Input }
