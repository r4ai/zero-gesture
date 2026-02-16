import {
  Button as RAButton,
  type ButtonProps as RAButtonProps,
} from "react-aria-components"
import { tv, type VariantProps } from "tailwind-variants"

const button = tv({
  base: "inline-flex items-center justify-center gap-2 rounded-md font-medium text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  variants: {
    variant: {
      default: "bg-foreground text-background hover:bg-foreground/90",
      destructive:
        "bg-destructive text-foreground-inverted hover:bg-destructive/90",
      outline:
        "border border-border bg-background hover:bg-background-subtle hover:text-foreground",
      secondary:
        "bg-background-subtle text-foreground hover:bg-background-subtle/80",
      ghost: "hover:bg-background-subtle hover:text-foreground",
      link: "text-foreground underline-offset-4 hover:underline",
    },
    size: {
      default: "h-10 px-4 py-2",
      sm: "h-9 rounded-md px-3",
      lg: "h-11 rounded-md px-8",
      icon: "h-10 w-10",
    },
  },
  defaultVariants: {
    variant: "default",
    size: "default",
  },
})

export interface ButtonProps
  extends RAButtonProps,
    VariantProps<typeof button> {
  className?: string // Explicitly allow className to be passed and merged if needed by consumers, though tv handles it via the returned class string usually.
}

export function Button({ className, variant, size, ...props }: ButtonProps) {
  return (
    <RAButton className={button({ variant, size, className })} {...props} />
  )
}
