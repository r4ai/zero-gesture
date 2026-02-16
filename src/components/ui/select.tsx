import { Check, ChevronDown } from "lucide-react"
import {
  Button,
  Label,
  ListBox,
  ListBoxItem,
  type ListBoxItemProps,
  Popover,
  Select as RASelect,
  type SelectProps as RASelectProps,
  SelectValue,
} from "react-aria-components"
import { tv } from "tailwind-variants"

const selectTrigger = tv({
  base: "flex h-10 w-full items-center justify-between rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50",
})

const popover = tv({
  base: "data-[entering]:fade-in-0 data-[exiting]:fade-out-0 data-[exiting]:zoom-out-95 data-[entering]:zoom-in-95 data-[placement=bottom]:slide-in-from-top-2 data-[placement=left]:slide-in-from-right-2 data-[placement=right]:slide-in-from-left-2 data-[placement=top]:slide-in-from-bottom-2 z-50 w-[var(--trigger-width)] min-w-[8rem] overflow-hidden rounded-md border bg-popover text-popover-foreground shadow-md data-[entering]:animate-in data-[exiting]:animate-out",
})

const listBox = tv({
  base: "p-1 outline-none",
})

const listBoxItem = tv({
  base: "relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pr-2 pl-8 text-sm outline-none data-[disabled]:pointer-events-none data-[focused]:bg-accent data-[focused]:text-accent-foreground data-[disabled]:opacity-50",
})

export interface SelectProps<T extends object> extends RASelectProps<T> {
  label?: string
  description?: string
  errorMessage?: string
  placeholder?: string
  className?: string // Add className to props
}

export function Select<T extends object>({
  label,
  description,
  errorMessage,
  children,
  placeholder,
  className,
  ...props
}: SelectProps<T>) {
  return (
    <RASelect
      className={
        className
          ? `${className} group flex flex-col gap-1.5`
          : "group flex flex-col gap-1.5"
      }
      {...props}
    >
      {label && (
        <Label className="font-medium text-sm leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70">
          {label}
        </Label>
      )}
      <Button className={selectTrigger()}>
        <SelectValue className="flex-1 text-left placeholder-shown:text-muted-foreground">
          {({ defaultChildren, isPlaceholder }) =>
            isPlaceholder ? placeholder || defaultChildren : defaultChildren
          }
        </SelectValue>
        <ChevronDown aria-hidden="true" className="h-4 w-4 opacity-50" />
      </Button>
      {description && (
        <p className="text-muted-foreground text-sm">{description}</p>
      )}
      {errorMessage && (
        <p className="font-medium text-destructive text-sm">{errorMessage}</p>
      )}
      <Popover className={popover()}>
        <ListBox className={listBox()}>{children}</ListBox>
      </Popover>
    </RASelect>
  )
}

export function SelectItem(props: ListBoxItemProps) {
  return (
    <ListBoxItem {...props} className={listBoxItem()}>
      {({ isSelected }) => (
        <>
          <span className="absolute left-2 flex h-3.5 w-3.5 items-center justify-center">
            {isSelected && <Check className="h-4 w-4" />}
          </span>
          {props.children}
        </>
      )}
    </ListBoxItem>
  )
}
