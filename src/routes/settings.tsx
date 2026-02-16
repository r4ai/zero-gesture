import { createFileRoute } from "@tanstack/react-router"
import {
  Sidebar,
  SidebarBody,
  SidebarFooter,
  SidebarHeader,
  SidebarItem,
} from "../components/ui/sidebar"

export const Route = createFileRoute("/settings")({
  component: SettingsLayout,
})

function SettingsLayout() {
  return (
    <div className="flex h-screen w-full bg-background">
      <Sidebar>
        <SidebarHeader>
          <span className="font-semibold text-lg">Settings</span>
        </SidebarHeader>
        <SidebarBody>
          <nav className="space-y-1 px-2">
            <SidebarItem active>
              <span>General</span>
            </SidebarItem>
            {/* Add other sidebar items here */}
          </nav>
        </SidebarBody>
        <SidebarFooter>{/* Footer content if needed */}</SidebarFooter>
      </Sidebar>
      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
    </div>
  )
}
