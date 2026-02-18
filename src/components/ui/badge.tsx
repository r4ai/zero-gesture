import { tv, type VariantProps } from "tailwind-variants"

const badge = tv({
  base: "inline-flex items-center justify-center font-medium transition-colors",
  variants: {
    variant: {
      default:
        "h-[22px] rounded-[4px] bg-background-subtle px-2 font-medium text-[11px] text-foreground",
      outline:
        "h-[22px] rounded-[4px] border border-border px-2 font-medium text-[11px] text-foreground-subtle",
      key: "h-[26px] rounded-[4px] border border-border-muted bg-background-subtle px-2 font-semibold text-[11px] text-foreground-muted",
      fallback:
        "h-[22px] rounded-[4px] bg-background-glass px-2 font-medium text-[10px] text-foreground-muted",
      success:
        "h-[22px] rounded-[6px] bg-success-subtle px-2 font-semibold text-[11px] text-success",
    },
  },
  defaultVariants: {
    variant: "default",
  },
})

export interface BadgeProps
  extends React.HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof badge> {}

export function Badge({ className, variant, ...props }: BadgeProps) {
  return <span className={badge({ variant, className })} {...props} />
}
