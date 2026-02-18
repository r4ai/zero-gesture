import {
  type Key,
  TabList as RATabList,
  Tabs as RATabs,
  Tab,
  TabPanel,
} from "react-aria-components"
import { tv } from "tailwind-variants"

const tabs = tv({
  slots: {
    root: "flex flex-col gap-2",
    tabList:
      "flex h-[40px] items-center gap-1 rounded-[12px] border border-border bg-background-card p-1",
    tab: "flex flex-1 cursor-pointer items-center justify-center rounded-[8px] px-3 py-1.5 font-medium text-[13px] text-foreground-muted outline-none transition-all focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2",
    tabPanel: "mt-2 outline-none",
  },
  variants: {
    isSelected: {
      true: {
        tab: "border border-border-bright bg-background text-foreground shadow dark:bg-background-subtle",
      },
      false: {
        tab: "hover:text-foreground",
      },
    },
  },
})

interface TabsBaseProps {
  className?: string
  children?: React.ReactNode
  selectedKey?: string
  onSelectionChange?: (key: Key) => void
}

export function Tabs({ className, children, ...props }: TabsBaseProps) {
  const { root } = tabs()
  return (
    <RATabs className={root({ className })} {...props}>
      {children}
    </RATabs>
  )
}

export function TabList({
  className,
  children,
}: React.HTMLAttributes<HTMLDivElement>) {
  const { tabList } = tabs()
  return <RATabList className={tabList({ className })}>{children}</RATabList>
}

interface TabItemProps {
  id: string
  className?: string
  children?: React.ReactNode
}

export function TabItem({ id, className, children }: TabItemProps) {
  const { tab } = tabs()
  return (
    <Tab
      id={id}
      className={(renderProps) =>
        tab({ isSelected: renderProps.isSelected, className })
      }
    >
      {children}
    </Tab>
  )
}

interface TabContentProps {
  id: string
  className?: string
  children?: React.ReactNode
}

export function TabContent({ id, className, children }: TabContentProps) {
  const { tabPanel } = tabs()
  return (
    <TabPanel id={id} className={tabPanel({ className })}>
      {children}
    </TabPanel>
  )
}
