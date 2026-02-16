import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import {
  createRootRoute,
  Outlet,
  redirect,
  useMatchRoute,
  useNavigate,
} from "@tanstack/react-router"
import { TanStackRouterDevtools } from "@tanstack/react-router-devtools"
import {
  Monitor,
  Moon,
  MousePointer2,
  Search,
  SlidersHorizontal,
  Sun,
} from "lucide-react"
import { Toaster } from "sonner"
import { ThemeProvider, useTheme } from "@/components/theme-provider"
import { Select, SelectItem } from "@/components/ui/select"
import {
  Sidebar,
  SidebarBody,
  SidebarFooter,
  SidebarHeader,
  SidebarItem,
} from "@/components/ui/sidebar"
import { useConfigUpdatedListener } from "@/hooks/use-config"

const queryClient = new QueryClient()

function ConfigEventBridge() {
  useConfigUpdatedListener()
  return null
}

/**
 * Application-wide layout component
 * Provides sidebar navigation and main content area
 */
function AppLayout() {
  const navigate = useNavigate()
  const matchRoute = useMatchRoute()
  const { theme, setTheme } = useTheme()

  /**
   * Check if a route is currently active
   * @param path - Route path to check (e.g., "/general")
   * @returns true if the route is active, false otherwise
   */
  const isRouteActive = (path: string) => !!matchRoute({ to: path })

  return (
    <div className="flex h-screen w-full overflow-hidden">
      {/* Sidebar Navigation */}
      <Sidebar>
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

              <SidebarItem
                active={isRouteActive("/general")}
                onClick={() => navigate({ to: "/general" })}
              >
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
                  onClick={() => {
                    const themes = ["light", "dark", "system"] as const
                    const currentIndex = themes.indexOf(theme)
                    const nextTheme = themes[(currentIndex + 1) % themes.length]
                    setTheme(nextTheme)
                  }}
                >
                  <Monitor className="size-4 text-foreground-muted" />
                </button>
              ) : (
                <>
                  <div className="font-semibold text-[11px] text-foreground-subtle tracking-wider">
                    THEME
                  </div>
                  <Select
                    value={theme}
                    onChange={(key) =>
                      setTheme(key as "light" | "dark" | "system")
                    }
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
                </>
              )}
            </SidebarFooter>
          </>
        )}
      </Sidebar>

      {/* Main Content Area - Route content is rendered here */}
      <Outlet />
    </div>
  )
}

const RootLayout = () => (
  <QueryClientProvider client={queryClient}>
    <ThemeProvider defaultTheme="system" storageKey="zero-gesture-theme">
      <ConfigEventBridge />
      <AppLayout />
      <TanStackRouterDevtools position="top-right" />
      <Toaster />
    </ThemeProvider>
  </QueryClientProvider>
)

export const Route = createRootRoute({
  component: RootLayout,
  beforeLoad: ({ location }) => {
    if (location.pathname === "/") {
      throw redirect({ to: "/general" })
    }
  },
})
