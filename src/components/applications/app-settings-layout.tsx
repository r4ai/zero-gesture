import { Link, useNavigate } from "@tanstack/react-router"
import { Pencil, Plus } from "lucide-react"
import { nanoid } from "nanoid"
import type { ReactNode } from "react"
import { twMerge } from "tailwind-merge"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { useConfigDraft } from "@/contexts/config-draft"
import type { AppDefinition, GestureBinding } from "@/types/config"

interface AppSettingsLayoutProps {
  appId: string
  selectedGestureId: string | undefined
  children: ReactNode
}

interface AppItem {
  id: string
  name: string
  icon: "fallback" | "terminal" | "generic"
}

function toAppItems(
  appIds: string[],
  apps: Record<string, AppDefinition>,
): AppItem[] {
  return appIds.map((id) => {
    if (id === "default") return { id, name: "default", icon: "fallback" }
    const name = apps[id]?.label ?? ""
    if (id.includes("term")) return { id, name, icon: "terminal" }
    return { id, name, icon: "generic" }
  })
}

function getAppIdsFromBindings(bindings: Record<string, GestureBinding[]>) {
  const ids = Object.keys(bindings)
  const rest = ids.filter((id) => id !== "default").sort()
  return ["default", ...rest]
}

function createNextAppId(appIds: string[]): string {
  const base = "app"
  let index = 1
  while (appIds.includes(`${base}-${index}`)) index += 1
  return `${base}-${index}`
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
export function GesturePanelLayout({
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

function AppIcon() {
  return <div className="h-3.5 w-3.5 rounded-sm bg-foreground-subtle" />
}

interface AppPanelLayoutProps {
  appId: string
  children: ReactNode
}

export function AppPanelLayout({ appId, children }: AppPanelLayoutProps) {
  const navigate = useNavigate()
  const { draft } = useConfigDraft()
  const appIds = getAppIdsFromBindings(draft.bindings)

  if (!appIds.includes(appId)) {
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
  const navigate = useNavigate()
  const { draft, setDraft } = useConfigDraft()
  const appIds = getAppIdsFromBindings(draft.bindings)
  const apps = toAppItems(appIds, draft.apps)

  const addApp = () => {
    const nextId = createNextAppId(appIds)
    setDraft({
      ...draft,
      apps: { ...draft.apps, [nextId]: { label: nextId, matchers: [] } },
      bindings: {
        ...draft.bindings,
        [nextId]: [],
      },
    })
    navigate({
      to: "/applications/$appId/edit",
      params: { appId: nextId },
    })
  }

  return (
    <div className="flex h-full w-[220px] flex-col border-border border-r bg-background">
      <div className="flex flex-col gap-1 border-border border-b px-4 py-4 pb-3">
        <h3 className="font-semibold text-foreground">Applications</h3>
        <p className="text-foreground-muted text-xs">Select app to configure</p>
      </div>

      <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
        {apps.map((app) => {
          const isActive = appId === app.id
          const firstGesture = draft.bindings[app.id]?.[0]

          return (
            <div
              key={app.id}
              className={twMerge(
                "group flex h-[40px] items-center justify-between rounded-lg px-2.5 transition-colors",
                isActive
                  ? "bg-background-card ring-1 ring-border-bright"
                  : "hover:bg-background-card",
              )}
            >
              <Link
                to="/applications/$appId/gestures/$gestureId"
                params={{
                  appId: app.id,
                  gestureId: draft.bindings[app.id]?.[0]?.id ?? "",
                }}
                search={{ tab: "gesture" }}
                className="flex min-w-0 flex-1 items-center gap-2"
              >
                <div className="flex h-6 w-6 items-center justify-center rounded-md bg-background-subtle">
                  <AppIcon />
                </div>
                <span
                  className={twMerge(
                    "truncate text-left text-sm",
                    isActive
                      ? "font-semibold text-foreground"
                      : "font-medium text-foreground-muted",
                  )}
                >
                  {app.name}
                </span>
                {app.id === "default" && (
                  <Badge className="ml-1" variant="fallback">
                    fallback
                  </Badge>
                )}
              </Link>
              <Link
                to="/applications/$appId/edit"
                params={{ appId: app.id }}
                className={twMerge(
                  "flex h-7 w-7 items-center justify-center rounded-md text-foreground-subtle transition-colors hover:bg-background-subtle hover:text-foreground",
                  "opacity-0 group-hover:opacity-100",
                  isActive && "opacity-100",
                )}
                aria-label={`${app.name} settings`}
              >
                <Pencil className="h-3.5 w-3.5" />
              </Link>
            </div>
          )
        })}
      </div>

      <div className="flex h-12 items-center justify-center border-border border-t px-2">
        <Button
          variant="outline"
          className="h-8 w-full gap-2 rounded-lg border-border-muted bg-transparent text-[12px]"
          onPress={addApp}
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
  selectedGestureId: string | undefined
}) {
  const navigate = useNavigate()
  const { draft, setDraft } = useConfigDraft()
  const bindings = draft.bindings[appId] ?? []
  const appIds = getAppIdsFromBindings(draft.bindings)

  if (!appIds.includes(appId)) {
    navigate({ to: "/applications" })
    return null
  }

  const addGesture = () => {
    const nextBinding: GestureBinding = {
      id: nanoid(11),
      label: "New Gesture",
      gesture: {
        mode: "release",
        trigger: "right_click",
        sequence: ["right"],
      },
      action: { type: "keyboard", keys: [] },
    }

    setDraft({
      ...draft,
      bindings: {
        ...draft.bindings,
        [appId]: [...bindings, nextBinding],
      },
    })

    navigate({
      to: "/applications/$appId/gestures/$gestureId",
      params: { appId, gestureId: nextBinding.id },
    })
  }

  return (
    <div className="flex h-full w-[260px] flex-col border-border border-r bg-background">
      <div className="flex flex-col gap-1 border-border border-b px-4 py-4 pb-3">
        <h3 className="font-semibold text-foreground">Gestures</h3>
        <p className="text-foreground-muted text-xs">
          Assign action to each gesture
        </p>
      </div>

      <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
        {bindings.map((gesture) => {
          const isActive = gesture.id === selectedGestureId

          return (
            <Link
              key={gesture.id}
              to="/applications/$appId/gestures/$gestureId"
              params={{ appId, gestureId: gesture.id }}
              search={{ tab: "gesture" }}
              className={twMerge(
                "flex h-[38px] items-center justify-between rounded-lg px-3 transition-colors",
                isActive
                  ? "bg-background-card ring-1 ring-border-bright"
                  : "hover:bg-background-card",
              )}
            >
              <span className="text-foreground text-sm">{gesture.label}</span>
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
          onPress={addGesture}
        >
          <Plus className="h-3.5 w-3.5" />
          <span>Add Gesture</span>
        </Button>
      </div>
    </div>
  )
}
