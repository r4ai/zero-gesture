import { Link, useNavigate } from "@tanstack/react-router"
import { Plus, Terminal } from "lucide-react"
import type { ReactNode } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { DEFAULT_BINDINGS, type GestureBinding } from "@/types/config"

interface AppSettingsLayoutProps {
  appId: string
  selectedGestureId: string
  children: ReactNode
}

interface AppItem {
  id: string
  name: string
  icon: "fallback" | "chrome" | "terminal" | "vscode"
}

export const APPS: AppItem[] = [
  { id: "default", name: "default", icon: "fallback" },
  { id: "chrome", name: "Google Chrome", icon: "chrome" },
  { id: "terminal", name: "Terminal", icon: "terminal" },
  { id: "vscode", name: "VS Code:", icon: "vscode" },
]

export const GESTURES = DEFAULT_BINDINGS

/**
 * Convert a gesture sequence into a stable route id.
 * e.g., ["up", "right"] -> "up-right"
 */
export function getGestureId(binding: GestureBinding): string {
  return binding.gesture.sequence.join("-")
}

/**
 * Format gesture steps into display string.
 * e.g., ["up", "right"] -> "Up → Right"
 */
export function formatGestureSequence(steps: string[]): string {
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
 * Format keyboard keys for display.
 * e.g., ["ctrl", "z"] -> "Ctrl+Z"
 */
export function formatKeys(keys: string[]): string {
  return keys.map((key) => key.charAt(0).toUpperCase() + key.slice(1)).join("+")
}

/**
 * Applications settings layout (App Panel | Gesture Panel | Action Panel).
 */
export function AppSettingsLayout({
  appId,
  selectedGestureId,
  children,
}: AppSettingsLayoutProps) {
  return (
    <div className="flex h-full w-full overflow-hidden">
      <GesturePanel appId={appId} selectedGestureId={selectedGestureId} />
      <div className="flex h-full flex-1 flex-col bg-background">
        {children}
      </div>
    </div>
  )
}

interface AppPanelLayoutProps {
  appId: string
  children: ReactNode
}

export function AppPanelLayout({ appId, children }: AppPanelLayoutProps) {
  const navigate = useNavigate()
  const currentApp = APPS.find((app) => app.id === appId)

  if (!currentApp) {
    navigate({ to: "/applications" })
    return null
  }

  return (
    <div className="flex h-full w-full overflow-hidden">
      <AppPanel appId={appId} />
      <div className="flex h-full flex-1 flex-col bg-background">
        {children}
      </div>
    </div>
  )
}

function AppPanel({ appId }: { appId: string }) {
  return (
    <div className="flex h-full w-[220px] flex-col border-border border-r bg-background">
      <div className="flex flex-col gap-1 border-border border-b px-4 py-4 pb-3">
        <h3 className="font-semibold text-[13px] text-foreground">
          Applications
        </h3>
        <p className="text-[12px] text-foreground-subtle">
          Select app to configure
        </p>
      </div>

      <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
        {APPS.map((app) => (
          <Link
            key={app.id}
            to="/applications/$appId/gestures/$gestureId"
            params={{ appId: app.id, gestureId: getGestureId(GESTURES[0]) }}
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
  )
}

function GesturePanel({
  appId,
  selectedGestureId,
}: {
  appId: string
  selectedGestureId: string
}) {
  const navigate = useNavigate()
  const currentApp = APPS.find((app) => app.id === appId)

  if (!currentApp) {
    navigate({ to: "/applications" })
    return null
  }

  return (
    <div className="flex h-full w-[260px] flex-col border-border border-r bg-background">
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

      <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
        {GESTURES.map((gesture) => {
          const gestureId = getGestureId(gesture)
          const isActive = gestureId === selectedGestureId

          return (
            <Link
              key={gestureId}
              to="/applications/$appId/gestures/$gestureId"
              params={{ appId, gestureId }}
              className={`flex h-[38px] items-center justify-between rounded-lg px-3 transition-colors ${
                isActive
                  ? "bg-background-card ring-1 ring-border-bright"
                  : "hover:bg-background-card"
              }`}
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
          )
        })}
      </div>

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
  )
}
