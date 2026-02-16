import {
  Switch as RASwitch,
  type SwitchProps as RASwitchProps,
} from "react-aria-components"
import { tv } from "tailwind-variants"

const switchStyles = tv({
  base: "group flex items-center gap-2 text-foreground text-sm transition [-webkit-tap-highlight-color:transparent] disabled:text-foreground-muted",
})

const trackStyles = tv({
  base: "flex h-[22px] w-[38px] shrink-0 cursor-pointer items-center rounded-full px-0.5 shadow-inner transition-all duration-200",
  variants: {
    isSelected: {
      true: "bg-foreground",
      false: "bg-background-muted",
    },
    isFocusVisible: {
      true: "outline outline-2 outline-foreground outline-offset-2",
    },
    isDisabled: {
      true: "cursor-not-allowed opacity-50",
    },
  },
  compoundVariants: [
    {
      isSelected: false,
      isDisabled: false,
      class: "bg-background-subtle hover:bg-background-subtle",
    },
    {
      isSelected: true,
      isDisabled: false,
      class: "hover:bg-foreground/90",
    },
  ],
})

const thumbStyles = tv({
  base: "block h-[18px] w-[18px] rounded-full shadow-sm transition-transform duration-200 ease-in-out",
  variants: {
    isSelected: {
      true: "translate-x-4 bg-background",
      false: "translate-x-0 bg-foreground",
    },
    isDisabled: {
      true: "bg-foreground-faint",
    },
  },
  compoundVariants: [
    {
      isSelected: true,
      isDisabled: true,
      class: "bg-foreground-faint",
    },
  ],
})

export interface SwitchProps extends RASwitchProps {
  className?: string
}

export function Switch({ className, children, ...props }: SwitchProps) {
  return (
    <RASwitch {...props} className={switchStyles({ className })}>
      {(renderProps) => (
        <>
          <div className={trackStyles(renderProps)}>
            <span className={thumbStyles(renderProps)} />
          </div>
          {typeof children === "function" ? children(renderProps) : children}
        </>
      )}
    </RASwitch>
  )
}
