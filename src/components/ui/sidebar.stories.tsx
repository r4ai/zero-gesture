import type { Meta, StoryObj } from "@storybook/react"
import { Home, Settings, User } from "lucide-react"
import {
  Sidebar,
  SidebarBody,
  SidebarFooter,
  SidebarHeader,
  SidebarItem,
} from "./sidebar"

const meta: Meta<typeof Sidebar> = {
  title: "UI/Sidebar",
  component: Sidebar,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
}

export default meta
type Story = StoryObj<typeof Sidebar>

export const Default: Story = {
  render: () => (
    <div className="h-[600px] border">
      <Sidebar>
        <SidebarHeader>
          <span className="font-semibold">MyApp</span>
        </SidebarHeader>
        <SidebarBody>
          <div className="space-y-1 px-2">
            <SidebarItem active>
              <Home className="h-4 w-4" />
              <span>Home</span>
            </SidebarItem>
            <SidebarItem>
              <User className="h-4 w-4" />
              <span>Profile</span>
            </SidebarItem>
            <SidebarItem>
              <Settings className="h-4 w-4" />
              <span>Settings</span>
            </SidebarItem>
          </div>
        </SidebarBody>
        <SidebarFooter>
          <div className="flex items-center gap-2 text-foreground-muted text-sm">
            <div className="h-8 w-8 rounded-full bg-background-muted" />
            <span>User Name</span>
          </div>
        </SidebarFooter>
      </Sidebar>
    </div>
  ),
}

export const Collapsed: Story = {
  render: () => (
    <div className="h-[600px] border">
      <Sidebar collapsed>
        <SidebarHeader className="justify-center px-0">
          <span className="font-semibold">MA</span>
        </SidebarHeader>
        <SidebarBody>
          <div className="space-y-1 px-2">
            <SidebarItem active className="justify-center px-0">
              <Home className="h-4 w-4" />
            </SidebarItem>
            <SidebarItem className="justify-center px-0">
              <User className="h-4 w-4" />
            </SidebarItem>
            <SidebarItem className="justify-center px-0">
              <Settings className="h-4 w-4" />
            </SidebarItem>
          </div>
        </SidebarBody>
        <SidebarFooter className="justify-center px-0">
          <div className="h-8 w-8 rounded-full bg-background-muted" />
        </SidebarFooter>
      </Sidebar>
    </div>
  ),
}
