/* eslint-disable react-refresh/only-export-components */
import * as React from "react"
import { Slot } from "@radix-ui/react-slot"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"

const buttonVariants = cva(
  "inline-flex cursor-pointer items-center justify-center gap-2 whitespace-nowrap rounded-lg text-sm font-medium transition-[color,background-color,border-color,box-shadow,transform] duration-150 ease-out focus-visible:outline-none focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:pointer-events-none disabled:opacity-45 active:translate-y-px [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "border border-primary/70 bg-primary text-primary-foreground shadow-[0_1px_2px_oklch(0.28_0.06_185/0.18),inset_0_1px_0_oklch(1_0_0/0.16)] hover:bg-primary/92 hover:shadow-[0_4px_12px_oklch(0.35_0.08_185/0.16)]",
        destructive:
          "border border-destructive/70 bg-destructive text-destructive-foreground shadow-[0_1px_2px_oklch(0.35_0.12_25/0.16)] hover:bg-destructive/92",
        outline:
          "border border-input/85 bg-background/85 text-foreground shadow-[0_1px_2px_oklch(0.2_0.02_230/0.05)] hover:border-primary/30 hover:bg-accent/65 hover:text-accent-foreground",
        secondary: "border border-transparent bg-secondary text-secondary-foreground hover:bg-secondary/75",
        ghost: "text-foreground/80 hover:bg-accent/75 hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 rounded-md px-3 text-xs",
        lg: "h-10 px-6",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button"
    return <Comp className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />
  }
)
Button.displayName = "Button"

export { Button, buttonVariants }
