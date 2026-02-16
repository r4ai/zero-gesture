import { tv } from "tailwind-variants"

export const panel = tv({
  base: "flex h-full flex-col overflow-hidden bg-background text-foreground",
})

export const panelHeader = tv({
  base: "flex items-center justify-between border-border border-b px-6 py-4",
})

export const panelBody = tv({
  base: "flex-1 overflow-y-auto p-6",
})

export const panelFooter = tv({
  base: "flex items-center justify-end border-border border-t bg-background-muted px-6 py-4",
})

export function Panel({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={panel({ className })} {...props} />
}

export function PanelHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={panelHeader({ className })} {...props} />
}

export function PanelBody({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={panelBody({ className })} {...props} />
}

export function PanelFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={panelFooter({ className })} {...props} />
}
