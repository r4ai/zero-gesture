import { tv } from "tailwind-variants"

const panel = tv({
  slots: {
    root: "flex h-full flex-1 flex-col overflow-hidden bg-background text-foreground",
    header:
      "flex h-16 items-center justify-between border-border border-b px-6",
    body: "flex-1 overflow-y-auto px-6 py-6",
    footer:
      "flex h-16 items-center justify-end gap-3 border-border border-t bg-background px-6",
  },
})

export function Panel({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { root } = panel()
  return <div className={root({ className })} {...props} />
}

export function PanelHeader({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { header } = panel()
  return <div className={header({ className })} {...props} />
}

export function PanelBody({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { body } = panel()
  return <div className={body({ className })} {...props} />
}

export function PanelFooter({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const { footer } = panel()
  return <div className={footer({ className })} {...props} />
}
