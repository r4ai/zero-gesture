import {
  Switch as RASwitch,
  type SwitchProps as RASwitchProps,
} from "react-aria-components"
import { tv } from "tailwind-variants"

const switchVariants = tv({
  slots: {
    base: "group flex items-center gap-2 text-foreground text-sm transition [-webkit-tap-highlight-color:transparent] disabled:text-foreground-muted",
    track:
      "flex h-[22px] w-[38px] shrink-0 cursor-pointer items-center rounded-full px-0.5 shadow-inner transition-all duration-200",
    thumb:
      "block h-[18px] w-[18px] rounded-full shadow-sm transition-transform duration-200 ease-in-out",
  },
  variants: {
    isSelected: {
      true: {
        track: "bg-foreground",
        thumb: "translate-x-4 bg-background",
      },
      false: {
        track: "bg-background-muted",
        thumb: "translate-x-0 bg-foreground",
      },
    },
    isFocusVisible: {
      true: {
        track: "outline outline-2 outline-foreground outline-offset-2",
      },
    },
    isDisabled: {
      true: {
        track: "cursor-not-allowed opacity-50",
        thumb: "bg-foreground-faint",
      },
    },
  },
  compoundVariants: [
    {
      isSelected: false,
      isDisabled: false,
      class: {
        track: "bg-background-subtle hover:bg-background-subtle",
      },
    },
    {
      isSelected: true,
      isDisabled: false,
      class: {
        track: "hover:bg-foreground/90",
      },
    },
    {
      isSelected: true,
      isDisabled: true,
      class: {
        thumb: "bg-foreground-faint",
      },
    },
  ],
})

export interface SwitchProps extends RASwitchProps {
  className?: string
}

export function Switch({ className, children, ...props }: SwitchProps) {
  return (
    <RASwitch {...props}>
      {(renderProps) => {
        const { base, track, thumb } = switchVariants(renderProps)
        return (
          <div className={base({ className })}>
            <div className={track()}>
              <span className={thumb()} />
            </div>
            {typeof children === "function" ? children(renderProps) : children}
          </div>
        )
      }}
    </RASwitch>
  )
}
