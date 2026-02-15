import { createRootRoute, Link, Outlet, redirect } from "@tanstack/react-router"
import { TanStackRouterDevtools } from "@tanstack/react-router-devtools"
import {
  AppWindow,
  Keyboard,
  Palette,
  Search,
  Settings,
  Wrench,
} from "lucide-react"
import { ThemeProvider, useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"

const sections = [
  {
    title: "General",
    url: "/general",
    icon: Settings,
  },
  {
    title: "Applications",
    url: "/applications",
    icon: AppWindow,
  },
  {
    title: "Bindings",
    url: "/bindings",
    icon: Keyboard,
  },
  {
    title: "Style",
    url: "/style",
    icon: Palette,
  },
  {
    title: "Advanced",
    url: "/advanced",
    icon: Wrench,
  },
]

const SettingsSidebarContent = () => {
  const { state } = useSidebar()
  const { theme, setTheme } = useTheme()
  const isCollapsed = state === "collapsed"

  return (
    <>
      <SidebarContent>
        <div className="p-2">
          {isCollapsed ? (
            <div className="flex h-9 items-center justify-center">
              <Search className="h-4 w-4 text-muted-foreground" />
            </div>
          ) : (
            <div className="relative">
              <Search className="absolute top-2.5 left-2.5 h-4 w-4 text-muted-foreground" />
              <Input type="search" placeholder="Search..." className="pl-9" />
            </div>
          )}
        </div>
        <Separator />
        <SidebarGroup className="p-2">
          <SidebarGroupContent>
            <SidebarMenu>
              {sections.map((section) => (
                <SidebarMenuItem key={section.title}>
                  <SidebarMenuButton asChild tooltip={section.title}>
                    <Link
                      to={section.url}
                      className="[&.active]:bg-accent [&.active]:font-bold [&.active]:text-accent-foreground"
                      activeProps={{
                        className: "bg-accent font-bold text-accent-foreground",
                      }}
                    >
                      <section.icon className="h-4 w-4" />
                      <span>{section.title}</span>
                    </Link>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter className="p-2">
        <div className="flex items-center justify-between gap-2">
          {!isCollapsed && (
            <Select
              value={theme}
              onValueChange={(value) =>
                setTheme(value as "light" | "dark" | "system")
              }
            >
              <SelectTrigger className="h-9 w-auto">
                <SelectValue placeholder="Theme" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="light">Light</SelectItem>
                <SelectItem value="dark">Dark</SelectItem>
                <SelectItem value="system">System</SelectItem>
              </SelectContent>
            </Select>
          )}
          <div className="ml-auto">
            <SidebarTrigger className="size-8" />
          </div>
        </div>
      </SidebarFooter>
    </>
  )
}

const SettingsSidebar = () => {
  return (
    <Sidebar variant="inset" collapsible="icon">
      <SettingsSidebarContent />
      <SidebarRail />
    </Sidebar>
  )
}

const RootLayout = () => (
  <ThemeProvider defaultTheme="system" storageKey="zero-gesture-theme">
    <TooltipProvider>
      <SidebarProvider defaultOpen={true}>
        <div className="flex h-screen w-full overflow-hidden">
          <SettingsSidebar />
          <div className="relative flex flex-1 flex-col">
            <main className="flex-1 overflow-auto p-6">
              <Outlet />
            </main>
            <Button variant="outline" className="fixed right-[156px] bottom-6">
              Cancel
            </Button>
            <Button className="fixed right-6 bottom-6">Save & Apply</Button>
          </div>
        </div>
        <TanStackRouterDevtools position="top-right" />
      </SidebarProvider>
    </TooltipProvider>
  </ThemeProvider>
)

export const Route = createRootRoute({
  component: RootLayout,
  beforeLoad: ({ location }) => {
    if (location.pathname === "/") {
      throw redirect({ to: "/general" })
    }
  },
})
