import { createFileRoute } from "@tanstack/react-router"
import {
  LayoutGrid,
  Monitor,
  Moon,
  MousePointer2,
  Paintbrush,
  Search,
  Settings2,
  SlidersHorizontal,
  Sun,
} from "lucide-react"
import { useState } from "react"
import { useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import {
  Panel,
  PanelBody,
  PanelFooter,
  PanelHeader,
} from "@/components/ui/panel"
import { Select, SelectItem } from "@/components/ui/select"
import {
  Sidebar,
  SidebarBody,
  SidebarFooter,
  SidebarHeader,
  SidebarItem,
} from "@/components/ui/sidebar"
import { Switch } from "@/components/ui/switch"

export const Route = createFileRoute("/general/")({
  component: GeneralSettings,
})

function GeneralSettings() {
  const [activeSection, setActiveSection] = useState("general")
  const [enableZeroGesture, setEnableZeroGesture] = useState(true)
  const { theme, setTheme } = useTheme()

  return (
    <div className="flex h-screen w-full overflow-hidden">
      {/* Sidebar Navigation */}
      <Sidebar>
        <SidebarHeader>
          <div className="flex items-center gap-2.5">
            <div className="flex size-7 items-center justify-center rounded-lg bg-background-subtle">
              <MousePointer2 className="size-3.5 text-foreground" />
            </div>
            <span className="font-bold text-[15px]">Zero Gesture</span>
          </div>
          <div className="flex h-[34px] items-center gap-2 rounded-lg border border-border bg-background-card px-2.5">
            <Search className="size-3.5 text-foreground-subtle" />
            <span className="text-[13px] text-foreground-subtle">
              Search...
            </span>
          </div>
        </SidebarHeader>
        <SidebarBody>
          <div className="mb-1 px-3 font-semibold text-[11px] text-foreground-subtle tracking-wider">
            PAGES
          </div>
          <SidebarItem
            active={activeSection === "general"}
            onClick={() => setActiveSection("general")}
          >
            <SlidersHorizontal className="size-4" />
            <span>General</span>
          </SidebarItem>
          <SidebarItem
            active={activeSection === "applications"}
            onClick={() => setActiveSection("applications")}
          >
            <LayoutGrid className="size-4" />
            <span>Applications</span>
          </SidebarItem>
          <SidebarItem
            active={activeSection === "style"}
            onClick={() => setActiveSection("style")}
          >
            <Paintbrush className="size-4" />
            <span>Style</span>
          </SidebarItem>
          <SidebarItem
            active={activeSection === "advanced"}
            onClick={() => setActiveSection("advanced")}
          >
            <Settings2 className="size-4" />
            <span>Advanced</span>
          </SidebarItem>
        </SidebarBody>
        <SidebarFooter>
          <div className="font-semibold text-[11px] text-foreground-subtle tracking-wider">
            THEME
          </div>
          <Select
            value={theme}
            onChange={(key) => setTheme(key as "light" | "dark" | "system")}
            aria-label="Select theme"
            className="*:bg-background-elevated!"
          >
            <SelectItem id="light" textValue="Light">
              <div className="flex items-center gap-2">
                <Sun className="size-3.5" />
                <span className="font-medium text-[13px]">Light</span>
              </div>
            </SelectItem>
            <SelectItem id="dark" textValue="Dark">
              <div className="flex items-center gap-2">
                <Moon className="size-3.5" />
                <span className="font-medium text-[13px]">Dark</span>
              </div>
            </SelectItem>
            <SelectItem id="system" textValue="System">
              <div className="flex items-center gap-2">
                <Monitor className="size-3.5" />
                <span className="font-medium text-[13px]">System</span>
              </div>
            </SelectItem>
          </Select>
        </SidebarFooter>
      </Sidebar>

      {/* Main Content Panel */}
      <Panel>
        <PanelHeader>
          <div className="flex flex-col gap-0.5">
            <h2 className="font-semibold text-[18px]">General</h2>
            <p className="text-[12px] text-foreground-subtle">
              General preferences for everyday use.
            </p>
          </div>
        </PanelHeader>
        <PanelBody>
          <div className="rounded-[10px] border border-border bg-background-elevated">
            <div className="flex h-[72px] items-center justify-between px-5">
              <div className="flex flex-col gap-1">
                <span className="font-medium text-[14px]">
                  Enable Zero Gesture
                </span>
                <span className="text-[12px] text-foreground-subtle">
                  Run gesture control on all of the other apps
                </span>
              </div>
              <Switch
                isSelected={enableZeroGesture}
                onChange={setEnableZeroGesture}
              />
            </div>
          </div>
        </PanelBody>
        <PanelFooter>
          <Button variant="outline">Cancel</Button>
          <Button>
            <span className="font-semibold text-[13px]">Save Changes</span>
          </Button>
        </PanelFooter>
      </Panel>
    </div>
  )
}
