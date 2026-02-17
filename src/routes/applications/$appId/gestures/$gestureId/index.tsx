import { createFileRoute, useParams } from "@tanstack/react-router"
import { Check, Keyboard, Trash2 } from "lucide-react"
import { useState } from "react"
import { Button } from "@/components/ui/button"
import { KeyInput } from "@/components/ui/key-input"
import { Select, SelectItem } from "@/components/ui/select"
import { TabItem, TabList, Tabs } from "@/components/ui/tabs"

export const Route = createFileRoute(
  "/applications/$appId/gestures/$gestureId/",
)({
  component: ActionEditPage,
})

/**
 * Format gesture steps into display string
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

// Mock gesture data
const mockGestures: Record<
  string,
  { sequence: string[]; label?: string; keys: string[] }
> = {
  left: { sequence: ["left"], label: "Left", keys: ["ctrl", "z"] },
  right: { sequence: ["right"], label: "Right", keys: ["ctrl", "y"] },
  up: { sequence: ["up"], label: "Up", keys: [] },
  down: { sequence: ["down"], label: "Down", keys: ["win", "down"] },
  "up-right": { sequence: ["up", "right"], label: "Up-Right", keys: [] },
}

/**
 * Action Edit Page - Edit gesture action (right panel)
 * Based on Pencil: "Applications Settings - Action Edit"
 */
function ActionEditPage() {
  const { gestureId } = useParams({
    from: "/applications/$appId/gestures/$gestureId/",
  })
  const [selectedTab, setSelectedTab] = useState<string>("action")
  const [editedKeys, setEditedKeys] = useState<string[]>(["ctrl", "z"])
  const [isDirty, setIsDirty] = useState(false)

  const gesture = mockGestures[gestureId]

  if (!gesture) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-[14px] text-foreground-subtle">Gesture not found</p>
      </div>
    )
  }

  const handleSave = () => {
    // TODO: Save changes to backend
    setIsDirty(false)
  }

  const handleCancel = () => {
    // Reset to original values
    setEditedKeys(gesture.keys)
    setIsDirty(false)
  }

  const handleRemoveAction = () => {
    // TODO: Remove action from gesture
  }

  return (
    <>
      {/* Header */}
      <div className="flex h-16 items-center justify-between border-border border-b px-6">
        <div className="flex flex-col">
          <h2 className="font-semibold text-[16px] text-foreground">
            {formatGestureSequence(gesture.sequence)}
          </h2>
          <span className="text-[12px] text-foreground-subtle">
            {gesture.label || "Custom Gesture"}
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
          <TabList className="flex h-10 items-center gap-1 rounded-xl border border-border bg-background-card p-1">
            <TabItem id="gesture">Gesture</TabItem>
            <TabItem id="action">Action</TabItem>
          </TabList>
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
              <Select value="keyboard" onChange={() => {}} className="w-full">
                <SelectItem id="keyboard" textValue="Keyboard Shortcut">
                  <div className="flex items-center gap-2">
                    <div className="flex h-4 w-4 items-center justify-center">
                      <Keyboard className="h-3.5 w-3.5" />
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
          <Check className="h-4 w-4" />
          <span>Save Changes</span>
        </Button>
      </div>
    </>
  )
}
