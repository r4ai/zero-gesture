import { Check } from "lucide-react"
import {
  Button,
  MenuTrigger,
  Popover,
  Menu as RAMenu,
  MenuItem as RAMenuItem,
  type MenuItemProps as RAMenuItemProps,
} from "react-aria-components"
import { tv } from "tailwind-variants"

const dropdownMenu = tv({
  slots: {
    trigger:
      "flex items-center justify-center rounded-lg border border-border bg-background-card transition-colors hover:bg-background-subtle focus-visible:outline focus-visible:outline-2 focus-visible:outline-foreground focus-visible:outline-offset-2 disabled:cursor-not-allowed disabled:opacity-50",
    popover:
      "data-[entering]:fade-in-0 data-[exiting]:fade-out-0 data-[exiting]:zoom-out-95 data-[entering]:zoom-in-95 data-[placement=bottom]:slide-in-from-top-2 data-[placement=left]:slide-in-from-right-2 data-[placement=right]:slide-in-from-left-2 data-[placement=top]:slide-in-from-bottom-2 z-50 min-w-[8rem] overflow-hidden rounded-md border border-border bg-background-elevated text-foreground shadow-md data-[entering]:animate-in data-[exiting]:animate-out",
    menu: "p-1 outline-none",
    menuItem:
      "relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pr-2 pl-8 text-sm outline-none data-[disabled]:pointer-events-none data-[focused]:bg-background-subtle data-[focused]:text-foreground data-[disabled]:opacity-50",
    checkIcon: "absolute left-2 flex h-3.5 w-3.5 items-center justify-center",
  },
})

export interface DropdownMenuProps {
  triggerContent: React.ReactNode
  triggerClassName?: string
  children: React.ReactNode
}

export function DropdownMenu({
  triggerContent,
  triggerClassName,
  children,
}: DropdownMenuProps) {
  const {
    trigger: triggerClass,
    popover: popoverClass,
    menu: menuClass,
  } = dropdownMenu()

  return (
    <MenuTrigger>
      <Button className={triggerClass({ className: triggerClassName })}>
        {triggerContent}
      </Button>
      <Popover className={popoverClass()}>
        <RAMenu className={menuClass()}>{children}</RAMenu>
      </Popover>
    </MenuTrigger>
  )
}

export interface MenuItemProps extends Omit<RAMenuItemProps, "children"> {
  checked?: boolean
  children?: React.ReactNode
}

export function MenuItem({ checked, children, ...props }: MenuItemProps) {
  const { menuItem: itemClass, checkIcon: checkIconClass } = dropdownMenu()

  return (
    <RAMenuItem {...props} className={itemClass()}>
      <span className={checkIconClass()}>
        {checked && <Check className="h-4 w-4" />}
      </span>
      {children}
    </RAMenuItem>
  )
}
