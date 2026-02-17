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

const select = tv({
  slots: {
    root: "group flex flex-col gap-1.5",
    label:
      "font-medium text-sm leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
    trigger:
      "flex h-10 w-full items-center justify-between rounded-md border border-border bg-background-card px-3 py-2 text-sm transition-colors placeholder:text-foreground-muted focus-visible:outline focus-visible:outline-2 focus-visible:outline-foreground focus-visible:outline-offset-2 disabled:cursor-not-allowed disabled:opacity-50",
    value: "flex-1 text-left placeholder-shown:text-foreground-muted",
    icon: "h-4 w-4 opacity-50",
    description: "text-foreground-muted text-sm",
    error: "font-medium text-destructive text-sm",
    popover:
      "data-[entering]:fade-in-0 data-[exiting]:fade-out-0 data-[exiting]:zoom-out-95 data-[entering]:zoom-in-95 data-[placement=bottom]:slide-in-from-top-2 data-[placement=left]:slide-in-from-right-2 data-[placement=right]:slide-in-from-left-2 data-[placement=top]:slide-in-from-bottom-2 z-50 w-[var(--trigger-width)] min-w-[8rem] overflow-hidden rounded-md border border-border bg-background-elevated text-foreground shadow-md data-[entering]:animate-in data-[exiting]:animate-out",
    listBox: "max-h-72 overflow-y-auto p-1 outline-none",
    listBoxItem:
      "relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pr-2 pl-8 text-sm outline-none data-[disabled]:pointer-events-none data-[focused]:bg-background-subtle data-[focused]:text-foreground data-[disabled]:opacity-50",
    checkIcon: "absolute left-2 flex h-3.5 w-3.5 items-center justify-center",
  },
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
  const {
    root: rootClass,
    label: labelClass,
    trigger: triggerClass,
    value: valueClass,
    icon: iconClass,
    description: descriptionClass,
    error: errorClass,
    popover: popoverClass,
    listBox: listBoxClass,
  } = select()

  return (
    <RASelect className={rootClass({ className })} {...props}>
      {label && <Label className={labelClass()}>{label}</Label>}
      <Button className={triggerClass()}>
        <SelectValue className={valueClass()}>
          {({ defaultChildren, isPlaceholder }) =>
            isPlaceholder ? placeholder || defaultChildren : defaultChildren
          }
        </SelectValue>
        <ChevronDown aria-hidden="true" className={iconClass()} />
      </Button>
      {description && <p className={descriptionClass()}>{description}</p>}
      {errorMessage && <p className={errorClass()}>{errorMessage}</p>}
      <Popover className={popoverClass()}>
        <ListBox className={listBoxClass()}>{children}</ListBox>
      </Popover>
    </RASelect>
  )
}

export function SelectItem(props: ListBoxItemProps) {
  const { listBoxItem: itemClass, checkIcon: checkIconClass } = select()

  return (
    <ListBoxItem {...props} className={itemClass()}>
      {({ isSelected }) => (
        <>
          <span className={checkIconClass()}>
            {isSelected && <Check className="h-4 w-4" />}
          </span>
          {props.children}
        </>
      )}
    </ListBoxItem>
  )
}
