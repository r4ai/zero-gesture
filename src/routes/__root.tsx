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
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
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
  SidebarTrigger,
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

const SettingsSidebar = () => {
  return (
    <Sidebar variant="inset" collapsible="icon" className="w-64 border-r">
      <SidebarContent>
        <div className="p-4">
          <div className="relative">
            <Search className="absolute top-2.5 left-2.5 h-4 w-4 text-muted-foreground" />
            <Input type="search" placeholder="Search..." className="pl-9" />
          </div>
        </div>
        <Separator />
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu>
              {sections.map((section) => (
                <SidebarMenuItem key={section.title}>
                  <SidebarMenuButton asChild>
                    <Link
                      to={section.url}
                      className="[&.active]:bg-accent [&.active]:text-accent-foreground"
                      activeProps={{
                        className: "bg-accent text-accent-foreground",
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
      <SidebarFooter className="p-4">
        <SidebarTrigger />
      </SidebarFooter>
    </Sidebar>
  )
}

const RootLayout = () => (
  <TooltipProvider>
    <SidebarProvider defaultOpen={true}>
      <div className="flex h-screen w-full overflow-hidden">
        <SettingsSidebar />
        <div className="flex flex-1 flex-col">
          <main className="flex-1 overflow-auto p-6">
            <Outlet />
          </main>
          <footer className="border-t p-4">
            <div className="flex justify-end gap-2">
              <Button variant="outline">Cancel</Button>
              <Button>Save & Apply</Button>
            </div>
          </footer>
        </div>
      </div>
      <TanStackRouterDevtools />
    </SidebarProvider>
  </TooltipProvider>
)

export const Route = createRootRoute({
  component: RootLayout,
  beforeLoad: ({ location }) => {
    if (location.pathname === "/") {
      throw redirect({ to: "/general" })
    }
  },
})
