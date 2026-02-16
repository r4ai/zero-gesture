import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { createRootRoute, Outlet, redirect } from "@tanstack/react-router"
import { TanStackRouterDevtools } from "@tanstack/react-router-devtools"
import { Toaster } from "sonner"
import { ThemeProvider } from "@/components/theme-provider"
import { useConfigUpdatedListener } from "@/hooks/use-config"

const queryClient = new QueryClient()

function ConfigEventBridge() {
  useConfigUpdatedListener()
  return null
}

const RootLayout = () => (
  <QueryClientProvider client={queryClient}>
    <ThemeProvider defaultTheme="system" storageKey="zero-gesture-theme">
      <ConfigEventBridge />
      <Outlet />
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
