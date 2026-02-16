import { tv } from "tailwind-variants"

export const sidebar = tv({
  base: "flex h-full w-[200px] flex-col border-r bg-background-elevated transition-[width]",
  variants: {
    collapsed: {
      true: "w-[72px]",
    },
  },
})

export const sidebarHeader = tv({
  base: "flex h-14 items-center border-b px-4",
})

export const sidebarBody = tv({
  base: "flex-1 overflow-y-auto py-2",
})

export const sidebarFooter = tv({
  base: "flex h-14 items-center border-t px-4",
})

export const sidebarItem = tv({
  base: "flex w-full items-center gap-3 rounded-md px-3 py-2 font-medium text-sm transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
  variants: {
    active: {
      true: "bg-accent text-accent-foreground",
    },
  },
})

export function Sidebar({
  className,
  collapsed,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { collapsed?: boolean }) {
  return <div className={sidebar({ collapsed, className })} {...props} />
}

export function SidebarHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={sidebarHeader({ className })} {...props} />
}

export function SidebarBody({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={sidebarBody({ className })} {...props} />
}

export function SidebarFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={sidebarFooter({ className })} {...props} />
}

export function SidebarItem({
  className,
  active,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { active?: boolean }) {
  return (
    <button
      type="button"
      className={sidebarItem({ active, className })}
      {...props}
    />
  )
}
