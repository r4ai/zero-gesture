import { tv } from "tailwind-variants"

const sidebar = tv({
  slots: {
    root: "flex h-full w-[200px] flex-col border-border border-r bg-background-elevated transition-[width]",
    header: "flex flex-col gap-4 border-border border-b px-4 py-5 pb-4",
    body: "flex flex-1 flex-col gap-1 overflow-y-auto px-2 py-2",
    footer: "flex flex-col gap-2 border-border border-t px-3 py-3 pb-4",
    item: "flex w-full items-center gap-2.5 rounded-lg border border-transparent px-3 py-2.5 font-medium text-[13px] transition-colors hover:bg-background-subtle",
  },
  variants: {
    collapsed: {
      true: {
        root: "w-[72px]",
      },
    },
    active: {
      true: {
        item: "border-border-bright bg-background-card font-semibold text-foreground",
      },
      false: {
        item: "text-foreground-muted",
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
