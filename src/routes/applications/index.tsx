import { createFileRoute } from "@tanstack/react-router"
import { Plus, Terminal, Trash2 } from "lucide-react"
import { useState } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { KeyInput } from "@/components/ui/key-input"
import { Select, SelectItem } from "@/components/ui/select"
import { TabItem, Tabs } from "@/components/ui/tabs"
import {
  DEFAULT_BINDINGS,
  type GestureBinding,
  type GestureStep,
} from "@/types/config"

export const Route = createFileRoute("/applications/")({
  component: ApplicationsSettings,
})

/**
 * Format gesture steps into display string
 * e.g., ["up", "right"] -> "Up → Right"
 */
function formatGestureSequence(steps: GestureStep[]): string {
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

/**
 * Applications Settings page component
 * Three-panel layout: App Panel | Gesture Panel | Action Panel
 */
function ApplicationsSettings() {
  const [selectedApp, setSelectedApp] = useState<string>("default")
  const [selectedGesture, setSelectedGesture] = useState<GestureBinding | null>(
    DEFAULT_BINDINGS[0],
  )
  const [editedKeys, setEditedKeys] = useState<string[]>(["ctrl", "z"])
  const [selectedTab, setSelectedTab] = useState<string>("action")
  const [isDirty, setIsDirty] = useState(false)

  // Mock data for apps
  const apps = [
    { id: "default", name: "default", icon: "fallback" },
    { id: "chrome", name: "Google Chrome", icon: "chrome" },
    { id: "terminal", name: "Terminal", icon: "terminal" },
    { id: "vscode", name: "VS Code:", icon: "vscode" },
  ]

  // Use default bindings as mock gesture data
  const gestures = DEFAULT_BINDINGS

  const handleSave = () => {
    // TODO: Save changes to backend
    setIsDirty(false)
  }

  const handleCancel = () => {
    // Reset to original values
    setEditedKeys(selectedGesture?.action.keys || [])
    setIsDirty(false)
  }

  const handleRemoveAction = () => {
    // TODO: Remove action from gesture
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
            <button
              key={app.id}
              type="button"
              onClick={() => setSelectedApp(app.id)}
              className={`flex h-[40px] items-center gap-3 rounded-lg px-3 transition-colors ${
                selectedApp === app.id
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
            </button>
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
          <h3 className="font-semibold text-[13px] text-foreground">
            Gestures
          </h3>
          <p className="text-[12px] text-foreground-subtle">
            Assign action to each gesture
          </p>
        </div>

        {/* Gesture List */}
        <div className="flex flex-1 flex-col gap-1 overflow-y-auto p-2">
          {gestures.map((gesture) => (
            <button
              key={gesture.label || gesture.gesture.sequence.join("-")}
              type="button"
              onClick={() => setSelectedGesture(gesture)}
              className={`flex h-[38px] items-center justify-between rounded-lg px-3 transition-colors ${
                selectedGesture?.label === gesture.label
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
            </button>
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
        {selectedGesture ? (
          <>
            {/* Header */}
            <div className="flex h-16 items-center justify-between border-border border-b px-6">
              <div className="flex flex-col">
                <h2 className="font-semibold text-[16px] text-foreground">
                  {formatGestureSequence(selectedGesture.gesture.sequence)}
                </h2>
                <span className="text-[12px] text-foreground-subtle">
                  {selectedGesture.label || "Custom Gesture"}
                </span>
              </div>
              <Button
                variant="outline"
                className="h-8 gap-2 rounded-md border-destructive-subtle bg-destructive-subtle text-[12px] text-destructive hover:bg-destructive/20"
                onPress={handleRemoveAction}
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>Remove Action</span>
              </Button>
            </div>

            {/* Tab Bar */}
            <div className="flex h-14 items-center border-border border-b px-6 py-2">
              <Tabs
                selectedKey={selectedTab}
                onSelectionChange={(key: string | number) =>
                  setSelectedTab(String(key))
                }
                className="w-full"
              >
                <div className="flex h-10 items-center gap-1 rounded-xl border border-border bg-background-card p-1">
                  <TabItem id="gesture">Gesture</TabItem>
                  <TabItem id="action">Action</TabItem>
                </div>
              </Tabs>
            </div>

            {/* Body */}
            <div className="flex-1 overflow-y-auto p-6">
              {selectedTab === "action" && (
                <div className="flex flex-col gap-6">
                  {/* Action Type */}
                  <div className="flex flex-col gap-2">
                    <span className="font-medium text-[12px] text-foreground-subtle">
                      Action Type
                    </span>
                    <Select
                      value="keyboard"
                      onChange={() => {}}
                      className="w-full"
                    >
                      <SelectItem id="keyboard" textValue="Keyboard Shortcut">
                        <div className="flex items-center gap-2">
                          <div className="flex h-4 w-4 items-center justify-center">
                            <svg
                              className="h-3.5 w-3.5"
                              viewBox="0 0 24 24"
                              fill="none"
                              stroke="currentColor"
                              strokeWidth="2"
                              aria-label="Keyboard icon"
                            >
                              <title>Keyboard</title>
                              <rect x="2" y="4" width="20" height="16" rx="2" />
                              <path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M8 12h.01M12 12h.01M16 12h.01M7 16h10" />
                            </svg>
                          </div>
                          <span className="font-medium text-[13px]">
                            Keyboard Shortcut
                          </span>
                        </div>
                      </SelectItem>
                    </Select>
                  </div>

                  {/* Divider */}
                  <div className="h-px bg-border" />

                  {/* Keyboard Shortcut */}
                  <KeyInput
                    keys={editedKeys}
                    onChange={(keys) => {
                      setEditedKeys(keys)
                      setIsDirty(true)
                    }}
                  />
                </div>
              )}

              {selectedTab === "gesture" && (
                <div className="flex flex-col gap-4">
                  <p className="text-[13px] text-foreground-muted">
                    Gesture configuration will be implemented here.
                  </p>
                </div>
              )}
            </div>

            {/* Footer */}
            <div className="flex h-16 items-center justify-end gap-3 border-border border-t px-6">
              <Button
                variant="outline"
                className="h-9 px-4 text-[13px]"
                onPress={handleCancel}
                isDisabled={!isDirty}
              >
                Cancel
              </Button>
              <Button
                className="h-9 gap-2 px-4 text-[13px]"
                onPress={handleSave}
                isDisabled={!isDirty}
              >
                <svg
                  className="h-4 w-4"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  aria-label="Check icon"
                >
                  <title>Check</title>
                  <polyline points="20 6 9 17 4 12" />
                </svg>
                <span>Save Changes</span>
              </Button>
            </div>
          </>
        ) : (
          <div className="flex h-full items-center justify-center">
            <p className="text-[14px] text-foreground-subtle">
              Select a gesture to edit its action
            </p>
          </div>
        )}
      </div>
    </div>
  )
}
