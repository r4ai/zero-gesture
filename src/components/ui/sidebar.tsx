import { tv } from "tailwind-variants"

const sidebar = tv({
  slots: {
    root: "flex h-full w-[200px] flex-col border-r bg-background-elevated transition-[width]",
    header: "flex h-14 items-center border-b px-4",
    body: "flex-1 overflow-y-auto py-2",
    footer: "flex h-14 items-center border-t px-4",
    item: "flex w-full items-center gap-3 rounded-md px-3 py-2 font-medium text-sm transition-colors hover:bg-background-subtle hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-foreground focus-visible:outline-offset-2",
  },
  variants: {
    collapsed: {
      true: {
        root: "w-[72px]",
      },
    },
    active: {
      true: {
        item: "bg-background-subtle text-foreground",
      },
    },
  },
})

export function Sidebar({
  className,
  collapsed,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { collapsed?: boolean }) {
  const { root } = sidebar({ collapsed })
  return <div className={root({ className })} {...props} />
}

export function SidebarHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { header } = sidebar()
  return <div className={header({ className })} {...props} />
}

export function SidebarBody({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { body } = sidebar()
  return <div className={body({ className })} {...props} />
}

export function SidebarFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { footer } = sidebar()
  return <div className={footer({ className })} {...props} />
}

export function SidebarItem({
  className,
  active,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { active?: boolean }) {
  const { item } = sidebar({ active })
  return <button type="button" className={item({ className })} {...props} />
}
