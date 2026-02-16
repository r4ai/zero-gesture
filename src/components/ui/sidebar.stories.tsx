import type { Meta, StoryObj } from "@storybook/react"
import {
  Home,
  LayoutGrid,
  Monitor,
  MousePointer2,
  Paintbrush,
  Search,
  Settings,
  Settings2,
  SlidersHorizontal,
  User,
} from "lucide-react"
import { expect, fireEvent, waitFor, within } from "storybook/test"
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

async function dragRail(
  rail: HTMLElement,
  startX: number,
  endX: number,
  pointerId = 1,
) {
  fireEvent.pointerDown(rail, { button: 0, clientX: startX, pointerId })
  fireEvent.pointerMove(window, { clientX: endX, pointerId })
  fireEvent.pointerUp(window, { clientX: endX, pointerId })
  await waitFor(() => {})
}

async function dragRailBy(rail: HTMLElement, deltaX: number, pointerId = 1) {
  const { left, width } = rail.getBoundingClientRect()
  const startX = left + width / 2
  await dragRail(rail, startX, startX + deltaX, pointerId)
}

export const Default: Story = {
  render: () => (
    <div className="flex h-[820px] border border-border bg-background">
      <Sidebar data-testid="sidebar-root">
        {({ compact }) => (
          <>
            <SidebarHeader>
              {compact ? (
                <div className="flex size-8 items-center justify-center rounded-lg bg-background-subtle">
                  <MousePointer2 className="size-4 text-foreground" />
                </div>
              ) : (
                <>
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
                </>
              )}
            </SidebarHeader>

            <SidebarBody>
              {!compact ? (
                <div className="mb-1 px-3 font-semibold text-[11px] text-foreground-subtle tracking-wider">
                  PAGES
                </div>
              ) : null}

              <SidebarItem active>
                <LayoutGrid className="size-4" />
                {!compact ? <span>Applications</span> : null}
              </SidebarItem>
              <SidebarItem>
                <SlidersHorizontal className="size-4" />
                {!compact ? <span>General</span> : null}
              </SidebarItem>
              <SidebarItem>
                <Paintbrush className="size-4" />
                {!compact ? <span>Style</span> : null}
              </SidebarItem>
              <SidebarItem>
                <Settings2 className="size-4" />
                {!compact ? <span>Advanced</span> : null}
              </SidebarItem>
              <div className="min-h-0 flex-1" />
            </SidebarBody>

            <SidebarFooter>
              {compact ? (
                <button
                  type="button"
                  className="flex h-10 w-10 items-center justify-center rounded-lg border border-border bg-background-card"
                >
                  <Monitor className="size-4 text-foreground-muted" />
                </button>
              ) : (
                <>
                  <div className="font-semibold text-[11px] text-foreground-subtle tracking-wider">
                    THEME
                  </div>
                  <div className="flex h-[34px] items-center justify-between rounded-lg border border-border bg-background-card px-2.5">
                    <div className="flex items-center gap-2">
                      <Monitor className="size-3.5 text-foreground-muted" />
                      <span className="font-medium text-[13px] text-foreground">
                        System
                      </span>
                    </div>
                    <span className="text-foreground-subtle text-xs">▼</span>
                  </div>
                </>
              )}
            </SidebarFooter>
          </>
        )}
      </Sidebar>
      <div className="flex-1 bg-background" />
    </div>
  ),
}

export const StateTransitions: Story = {
  render: () => (
    <div className="flex h-[820px] border border-border bg-background">
      <Sidebar data-testid="sidebar-root">
        {({ compact }) => (
          <>
            <SidebarHeader>
              {compact ? (
                <div className="flex size-8 items-center justify-center rounded-lg bg-background-subtle">
                  <MousePointer2 className="size-4 text-foreground" />
                </div>
              ) : (
                <>
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
                </>
              )}
            </SidebarHeader>

            <SidebarBody>
              {!compact ? (
                <div className="mb-1 px-3 font-semibold text-[11px] text-foreground-subtle tracking-wider">
                  PAGES
                </div>
              ) : null}
              <SidebarItem active>
                <LayoutGrid className="size-4" />
                {!compact ? <span>Applications</span> : null}
              </SidebarItem>
              <SidebarItem>
                <SlidersHorizontal className="size-4" />
                {!compact ? <span>General</span> : null}
              </SidebarItem>
              <div className="min-h-0 flex-1" />
            </SidebarBody>

            <SidebarFooter>
              {compact ? (
                <button
                  type="button"
                  className="flex h-10 w-10 items-center justify-center rounded-lg border border-border bg-background-card"
                >
                  <Monitor className="size-4 text-foreground-muted" />
                </button>
              ) : (
                <>
                  <div className="font-semibold text-[11px] text-foreground-subtle tracking-wider">
                    THEME
                  </div>
                  <div className="flex h-[34px] items-center justify-between rounded-lg border border-border bg-background-card px-2.5">
                    <div className="flex items-center gap-2">
                      <Monitor className="size-3.5 text-foreground-muted" />
                      <span className="font-medium text-[13px] text-foreground">
                        System
                      </span>
                    </div>
                    <span className="text-foreground-subtle text-xs">▼</span>
                  </div>
                </>
              )}
            </SidebarFooter>
          </>
        )}
      </Sidebar>
      <div className="flex-1 bg-background" />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const sidebarRoot = canvas.getByTestId("sidebar-root")
    const rail = canvas.getByLabelText("Resize sidebar")
    const { left, width } = rail.getBoundingClientRect()
    const startX = left + width / 2

    expect(sidebarRoot.style.getPropertyValue("--sidebar-width")).toBe("200px")
    expect(canvas.getByText("PAGES")).toBeInTheDocument()

    await dragRailBy(rail, -140, 1)

    await waitFor(() => {
      expect(sidebarRoot.getAttribute("data-compact")).toBe("true")
    })
    expect(canvas.queryByText("PAGES")).not.toBeInTheDocument()
    expect(sidebarRoot.style.getPropertyValue("--sidebar-width")).toBe("72px")

    await dragRailBy(rail, -60, 6)
    await waitFor(() => {
      expect(sidebarRoot.getAttribute("data-compact")).toBe("true")
    })
    expect(sidebarRoot.style.getPropertyValue("--sidebar-width")).toBe("72px")

    fireEvent.pointerDown(rail, { button: 0, clientX: startX, pointerId: 7 })
    await waitFor(() => {
      expect(rail.getAttribute("data-dragging")).toBe("true")
    })
    expect(sidebarRoot.className).toContain("transition-none")
    fireEvent.pointerMove(window, { clientX: startX + 30, pointerId: 7 })
    fireEvent.pointerMove(window, { clientX: startX + 80, pointerId: 7 })
    fireEvent.pointerMove(window, { clientX: startX + 120, pointerId: 7 })
    fireEvent.pointerUp(window, { clientX: startX + 120, pointerId: 7 })

    await waitFor(() => {
      expect(rail.getAttribute("data-dragging")).toBe("false")
    })
    expect(sidebarRoot.className).toContain("transition-[width]")
    await waitFor(() => {
      expect(sidebarRoot.style.getPropertyValue("--sidebar-width")).toBe(
        "192px",
      )
    })

    await dragRailBy(rail, 180, 2)

    await waitFor(() => {
      expect(sidebarRoot.getAttribute("data-compact")).toBe("false")
    })
    expect(canvas.getByText("PAGES")).toBeInTheDocument()

    await dragRailBy(rail, 500, 3)
    await waitFor(() => {
      expect(sidebarRoot.style.getPropertyValue("--sidebar-width")).toBe(
        "360px",
      )
    })

    await dragRailBy(rail, -40, 4)
    await waitFor(() => {
      expect(sidebarRoot.getAttribute("data-compact")).toBe("false")
    })
    expect(sidebarRoot.style.getPropertyValue("--sidebar-width")).toBe("320px")

    await dragRailBy(rail, -500, 5)
    await waitFor(() => {
      expect(sidebarRoot.getAttribute("data-compact")).toBe("true")
    })
    expect(sidebarRoot.style.getPropertyValue("--sidebar-width")).toBe("72px")
  },
}

export const Collapsed: Story = {
  render: () => (
    <div className="h-[600px] border">
      <Sidebar defaultCollapsed resizable={false}>
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
