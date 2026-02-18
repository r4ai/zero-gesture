import { Check, ChevronDown } from "lucide-react"
import type { ReactNode } from "react"
import {
  Button,
  FieldError,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  type ListBoxItemProps,
  Popover,
  ComboBox as RAComboBox,
  type ComboBoxProps as RAComboBoxProps,
  type ValidationResult,
} from "react-aria-components"
import { tv } from "tailwind-variants"

const combobox = tv({
  slots: {
    root: "group flex flex-col gap-1.5",
    label:
      "font-medium text-sm leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70",
    field:
      "flex h-10 w-full items-center rounded-md border border-border bg-background-card text-sm transition-colors focus-within:outline focus-within:outline-2 focus-within:outline-foreground focus-within:outline-offset-2 data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50",
    input:
      "h-full min-w-0 flex-1 bg-transparent px-3 py-2 text-foreground outline-none placeholder:text-foreground-muted disabled:cursor-not-allowed",
    button:
      "mr-1 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-sm text-foreground-muted transition-colors hover:bg-background-subtle",
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

export interface ComboBoxProps<T extends object>
  extends Omit<RAComboBoxProps<T>, "children"> {
  label?: string
  description?: string
  errorMessage?: string | ((validation: ValidationResult) => string)
  placeholder?: string
  className?: string
  children: ReactNode | ((item: T) => ReactNode)
}

export function ComboBox<T extends object>({
  label,
  description,
  errorMessage,
  placeholder,
  className,
  children,
  ...props
}: ComboBoxProps<T>) {
  const {
    root: rootClass,
    label: labelClass,
    field: fieldClass,
    input: inputClass,
    button: buttonClass,
    icon: iconClass,
    description: descriptionClass,
    error: errorClass,
    popover: popoverClass,
    listBox: listBoxClass,
  } = combobox()

  return (
    <RAComboBox className={rootClass({ className })} {...props}>
      {label && <Label className={labelClass()}>{label}</Label>}
      <div className={fieldClass()}>
        <Input className={inputClass()} placeholder={placeholder} />
        <Button className={buttonClass()}>
          <ChevronDown aria-hidden="true" className={iconClass()} />
        </Button>
      </div>
      {description && <p className={descriptionClass()}>{description}</p>}
      {errorMessage && (
        <FieldError className={errorClass()}>{errorMessage}</FieldError>
      )}
      <Popover className={popoverClass()}>
        <ListBox className={listBoxClass()}>{children}</ListBox>
      </Popover>
    </RAComboBox>
  )
}

export function ComboBoxItem(props: ListBoxItemProps) {
  const { listBoxItem: itemClass, checkIcon: checkIconClass } = combobox()

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
