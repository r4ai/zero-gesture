import {
  createFileRoute,
  Link,
  Outlet,
  useNavigate,
  useParams,
} from "@tanstack/react-router"
import { Plus, Terminal } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { DEFAULT_BINDINGS } from "@/types/config"

export const Route = createFileRoute("/applications/$appId/")({
  component: AppDetailLayout,
})

/**
 * Format gesture steps into display string
 * e.g., ["up", "right"] -> "Up → Right"
 */
function formatGestureSequence(steps: string[]): string {
  if (steps.length === 0) return ""
  return steps
    .map((step) =>
      step
        .split("_")
        .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
        .join(" "),
    )
    .join(" → ")
}

/**
 * Format keyboard keys for display
 * e.g., ["ctrl", "z"] -> "Ctrl+Z"
 */
function formatKeys(keys: string[]): string {
  return keys.map((key) => key.charAt(0).toUpperCase() + key.slice(1)).join("+")
}

// Mock data for apps
const apps = [
  { id: "default", name: "default", icon: "fallback" },
  { id: "chrome", name: "Google Chrome", icon: "chrome" },
  { id: "terminal", name: "Terminal", icon: "terminal" },
  { id: "vscode", name: "VS Code:", icon: "vscode" },
]

// Use default bindings as mock gesture data
const gestures = DEFAULT_BINDINGS

/**
 * App Detail Layout - Three-panel layout with App Panel | Gesture Panel | Outlet for Action Panel
 */
function AppDetailLayout() {
  const { appId } = useParams({ from: "/applications/$appId/" })
  const navigate = useNavigate()

  const currentApp = apps.find((app) => app.id === appId)

  if (!currentApp) {
    // If app not found, redirect to applications list
    navigate({ to: "/applications" })
    return null
  }

  return (
    <div className="flex h-full w-full overflow-hidden">
      {/* App Panel - 220px */}
      <div className="flex h-full w-[220px] flex-col border-border border-r bg-background">
        {/* Header */}
        <div className="flex flex-col gap-1 border-border border-b px-4 py-4 pb-3">
          <h3 className="font-semibold text-[13px] text-foreground">
            Applications
          </h3>
          <p className="text-[12px] text-foreground-subtle">
            Select app to configure
          </p>
        </div>

        {/* App List */}
        <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
          {apps.map((app) => (
            <Link
              key={app.id}
              to="/applications/$appId"
              params={{ appId: app.id }}
              className={`flex h-[40px] items-center gap-3 rounded-lg px-3 transition-colors ${
                appId === app.id
                  ? "bg-background-card ring-1 ring-border-bright"
                  : "hover:bg-background-card"
              }`}
            >
              <div className="flex h-6 w-6 items-center justify-center rounded-md bg-background-subtle">
                {app.icon === "terminal" ? (
                  <Terminal className="h-3.5 w-3.5 text-foreground" />
                ) : (
                  <div className="h-3.5 w-3.5 rounded-sm bg-foreground-subtle" />
                )}
              </div>
              <span className="flex-1 truncate text-left text-[13px] text-foreground">
                {app.name}
              </span>
              {app.icon === "fallback" && (
                <Badge variant="fallback">fallback</Badge>
              )}
            </Link>
          ))}
        </div>

        {/* Footer */}
        <div className="flex h-12 items-center justify-center border-border border-t px-2">
          <Button
            variant="outline"
            className="h-8 w-full gap-2 rounded-lg border-border-muted bg-transparent text-[12px]"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>Add Application</span>
          </Button>
        </div>
      </div>

      {/* Gesture Panel - 260px */}
      <div className="flex h-full w-[260px] flex-col border-border border-r bg-background">
        {/* Header */}
        <div className="flex flex-col gap-1 border-border border-b px-4 py-4 pb-3">
          <div className="flex items-center justify-between">
            <h3 className="font-semibold text-[13px] text-foreground">
              Gestures
            </h3>
            <Link
              to="/applications/$appId/edit"
              params={{ appId }}
              className="text-[11px] text-foreground-subtle transition-colors hover:text-foreground"
            >
              Edit App
            </Link>
          </div>
          <p className="text-[12px] text-foreground-subtle">
            Assign action to each gesture
          </p>
        </div>

        {/* Gesture List */}
        <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
          {gestures.map((gesture) => (
            <Link
              key={gesture.label || gesture.gesture.sequence.join("-")}
              to="/applications/$appId/gestures/$gestureId"
              params={{
                appId,
                gestureId: gesture.label || gesture.gesture.sequence.join("-"),
              }}
              className="flex h-[38px] items-center justify-between rounded-lg px-3 transition-colors hover:bg-background-card"
            >
              <span className="text-[13px] text-foreground">
                {formatGestureSequence(gesture.gesture.sequence)}
              </span>
              {gesture.action.keys.length > 0 ? (
                <Badge variant="default">
                  {formatKeys(gesture.action.keys)}
                </Badge>
              ) : (
                <Badge variant="outline">—</Badge>
              )}
            </Link>
          ))}
        </div>

        {/* Footer */}
        <div className="flex h-12 items-center justify-center border-border border-t px-2">
          <Button
            variant="outline"
            className="h-8 w-full gap-2 rounded-lg border-border-muted bg-transparent text-[12px]"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>Add Gesture</span>
          </Button>
        </div>
      </div>

      {/* Action Panel - Fill remaining */}
      <div className="flex h-full flex-1 flex-col bg-background">
        <Outlet />
      </div>
    </div>
  )
}
