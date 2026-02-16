import {
  Switch as RASwitch,
  type SwitchProps as RASwitchProps,
} from "react-aria-components"
import { tv } from "tailwind-variants"

const switchTrack = tv({
  base: "group inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent bg-input shadow-sm transition-colors hover:bg-input/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[selected]:bg-primary data-[selected]:text-primary-foreground data-[selected]:hover:bg-primary/90",
})

const switchThumb = tv({
  base: "pointer-events-none block h-5 w-5 rounded-full bg-background shadow-lg ring-0 transition-transform group-active:translate-x-1 data-[selected]:translate-x-5 group-active:data-[selected]:translate-x-4",
})

export interface SwitchProps extends RASwitchProps {
  className?: string
}

export function Switch({ className, children, ...props }: SwitchProps) {
  return (
    <RASwitch className={switchTrack({ className })} {...props}>
      <span className={switchThumb()} />
      {children}
    </RASwitch>
  )
}
