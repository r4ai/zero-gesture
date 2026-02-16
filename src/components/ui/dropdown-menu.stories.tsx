import type { Meta, StoryObj } from "@storybook/react"
import { Monitor, Moon, Sun } from "lucide-react"
import { useState } from "react"
import { DropdownMenu, MenuItem } from "./dropdown-menu"

const meta: Meta<typeof DropdownMenu> = {
  title: "UI/DropdownMenu",
  component: DropdownMenu,
  parameters: {
    layout: "centered",
  },
  tags: ["autodocs"],
}

export default meta
type Story = StoryObj<typeof DropdownMenu>

export const Default: Story = {
  render: () => {
    const [theme, setTheme] = useState<"light" | "dark" | "system">("system")

    return (
      <DropdownMenu
        triggerContent={
          theme === "light" ? (
            <Sun className="size-4 text-foreground-muted" />
          ) : theme === "dark" ? (
            <Moon className="size-4 text-foreground-muted" />
          ) : (
            <Monitor className="size-4 text-foreground-muted" />
          )
        }
        triggerClassName="h-10 w-10"
      >
        <MenuItem
          id="system"
          checked={theme === "system"}
          onAction={() => setTheme("system")}
        >
          <div className="flex items-center gap-2">
            <Monitor className="size-3.5" />
            <span className="font-medium text-[13px]">System</span>
          </div>
        </MenuItem>
        <MenuItem
          id="light"
          checked={theme === "light"}
          onAction={() => setTheme("light")}
        >
          <div className="flex items-center gap-2">
            <Sun className="size-3.5" />
            <span className="font-medium text-[13px]">Light</span>
          </div>
        </MenuItem>
        <MenuItem
          id="dark"
          checked={theme === "dark"}
          onAction={() => setTheme("dark")}
        >
          <div className="flex items-center gap-2">
            <Moon className="size-3.5" />
            <span className="font-medium text-[13px]">Dark</span>
          </div>
        </MenuItem>
      </DropdownMenu>
    )
  },
}
