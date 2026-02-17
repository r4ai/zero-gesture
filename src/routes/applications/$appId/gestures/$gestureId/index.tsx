import { createFileRoute, useNavigate, useParams } from "@tanstack/react-router"
import { Check, Keyboard, Plus, Trash2, X } from "lucide-react"
import { useEffect, useState } from "react"
import {
  AppSettingsLayout,
  GESTURES,
  getGestureId,
} from "@/components/applications/app-settings-layout"
import { Badge } from "@/components/ui/badge"
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
 * Action Edit Page - Edit gesture action (right panel)
 * Based on Pencil: "Applications Settings - Gesture Edit"
 */
function ActionEditPage() {
  const { appId, gestureId } = useParams({
    from: "/applications/$appId/gestures/$gestureId/",
  })
  const search = Route.useSearch() as { shortcut?: string }
  const navigate = useNavigate()
  const gesture = GESTURES.find((item) => getGestureId(item) === gestureId)
  const originalKeys = gesture?.action.keys ?? []
  const [selectedTab, setSelectedTab] = useState<string>("gesture")
  const [gestureMode, setGestureMode] = useState<"hold" | "release">("hold")
  const [triggerButton, setTriggerButton] = useState<string>("right-click")
  const [sequenceRows, setSequenceRows] = useState<
    { id: number; type: string; step: string }[]
  >([
    { id: 1, type: "mouse-move", step: "left" },
    { id: 2, type: "mouse-move", step: "right" },
  ])
  const [finalStep, setFinalStep] = useState<string>("wheel-up")
  const [editedKeys, setEditedKeys] = useState<string[]>(originalKeys)
  const [isDirty, setIsDirty] = useState(false)

  const handleSave = () => {
    // TODO: Save changes to backend
    setIsDirty(false)
  }

  const handleCancel = () => {
    // Reset to original values
    setEditedKeys(originalKeys)
    setGestureMode("hold")
    setTriggerButton("right-click")
    setSequenceRows([
      { id: 1, type: "mouse-move", step: "left" },
      { id: 2, type: "mouse-move", step: "right" },
    ])
    setFinalStep("wheel-up")
    setIsDirty(false)
  }

  const handleRemoveAction = () => {
    // TODO: Remove action from gesture
  }

  const openKeyboardInput = (mode: "wait" | "manual") => {
    navigate({
      to: "/applications/$appId",
      params: { appId },
      search: {
        mode,
        gestureId,
        keys: editedKeys.length > 0 ? editedKeys.join(",") : undefined,
      },
    })
  }

  const updateSequenceRow = (
    rowId: number,
    patch: Partial<{ type: string; step: string }>,
  ) => {
    setSequenceRows((prev) =>
      prev.map((row) => (row.id === rowId ? { ...row, ...patch } : row)),
    )
    setIsDirty(true)
  }

  const addSequenceRow = () => {
    setSequenceRows((prev) => [
      ...prev,
      {
        id: Math.max(0, ...prev.map((row) => row.id)) + 1,
        type: "mouse-input",
        step: "wheel-up",
      },
    ])
    setIsDirty(true)
  }

  const removeSequenceRow = (rowId: number) => {
    setSequenceRows((prev) => prev.filter((row) => row.id !== rowId))
    setIsDirty(true)
  }

  useEffect(() => {
    const shortcut =
      typeof search.shortcut === "string" ? search.shortcut : undefined
    if (!shortcut) return

    const next = shortcut
      .split(",")
      .map((part: string) => part.trim())
      .filter((part: string) => part.length > 0)

    setEditedKeys(next)
    setIsDirty(true)
    navigate({
      to: "/applications/$appId/gestures/$gestureId",
      params: { appId, gestureId },
      search: {},
      replace: true,
    })
  }, [appId, gestureId, navigate, search.shortcut])

  if (!gesture) {
    return (
      <AppSettingsLayout appId={appId} selectedGestureId={gestureId}>
        <div className="flex h-full items-center justify-center">
          <p className="text-[14px] text-foreground-subtle">
            Gesture not found
          </p>
        </div>
      </AppSettingsLayout>
    )
  }

  return (
    <AppSettingsLayout appId={appId} selectedGestureId={gestureId}>
      {/* Header */}
      <div className="flex h-16 items-center justify-between border-border border-b px-6">
        <div className="flex flex-col">
          <h2 className="font-semibold text-foreground text-lg">
            {gesture.label ?? "Untitled"}
          </h2>
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
      <div className="flex h-14 items-center border-border px-6 py-2">
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
              <span className="font-medium text-foreground text-sm">
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
              onPress={() => openKeyboardInput("wait")}
              onKeyboardPress={() => openKeyboardInput("manual")}
            />
          </div>
        )}

        {selectedTab === "gesture" && (
          <div className="flex flex-col gap-5">
            <div className="flex flex-col gap-2">
              <span className="font-medium text-foreground text-sm">
                Trigger Button
              </span>
              <div className="flex h-10 items-center gap-2">
                <Select
                  value={triggerButton}
                  onChange={(key) => {
                    setTriggerButton(String(key))
                    setIsDirty(true)
                  }}
                  className="flex-1"
                >
                  <SelectItem id="right-click" textValue="Right Click">
                    Right Click
                  </SelectItem>
                  <SelectItem id="left-click" textValue="Left Click">
                    Left Click
                  </SelectItem>
                  <SelectItem id="middle-click" textValue="Middle Click">
                    Middle Click
                  </SelectItem>
                </Select>
                <Button
                  variant="outline"
                  size="icon"
                  className="h-10 w-10 rounded-[8px] border-border bg-background-card hover:bg-background-subtle"
                >
                  <Keyboard className="h-3.5 w-3.5 text-foreground" />
                </Button>
              </div>
              <p className="text-foreground-muted text-xs">
                Use the keyboard icon to capture from live input.
              </p>
            </div>

            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <span className="font-medium text-foreground text-sm">
                  Gesture Mode
                </span>
              </div>
              <Tabs
                selectedKey={gestureMode}
                onSelectionChange={(key: string | number) => {
                  setGestureMode(String(key) as "hold" | "release")
                  setIsDirty(true)
                }}
                className="w-full"
              >
                <TabList className="flex h-10 items-center gap-1 rounded-xl border border-border bg-background-card p-1">
                  <TabItem id="hold">Hold</TabItem>
                  <TabItem id="release">Release</TabItem>
                </TabList>
              </Tabs>
              <p className="text-foreground-muted text-xs">
                Hold: fires while the trigger is held. Release: fires on trigger
                release.
              </p>
            </div>

            <div className="flex flex-col gap-3">
              <span className="font-medium text-foreground text-sm">
                Sequence
              </span>
              <p className="text-foreground-muted text-xs">
                Ordered steps recognized before the final step fires. Up to 8
                steps.
              </p>
              <div className="flex flex-col gap-2">
                {sequenceRows.map((row, index) => (
                  <div key={row.id} className="flex items-center gap-2">
                    <span className="w-3 text-center text-[12px] text-foreground-muted">
                      {index + 1}
                    </span>
                    <Select
                      value={row.type}
                      onChange={(key) =>
                        updateSequenceRow(row.id, { type: String(key) })
                      }
                      className="w-[140px]"
                    >
                      <SelectItem id="mouse-move" textValue="Mouse Move">
                        Mouse Move
                      </SelectItem>
                      <SelectItem id="mouse-input" textValue="Mouse Input">
                        Mouse Input
                      </SelectItem>
                    </Select>
                    <Select
                      value={row.step}
                      onChange={(key) =>
                        updateSequenceRow(row.id, { step: String(key) })
                      }
                      className="flex-1"
                    >
                      <SelectItem id="left" textValue="Left">
                        Left
                      </SelectItem>
                      <SelectItem id="right" textValue="Right">
                        Right
                      </SelectItem>
                      <SelectItem id="up" textValue="Up">
                        Up
                      </SelectItem>
                      <SelectItem id="down" textValue="Down">
                        Down
                      </SelectItem>
                      <SelectItem id="wheel-up" textValue="Wheel Up">
                        Wheel Up
                      </SelectItem>
                      <SelectItem id="wheel-down" textValue="Wheel Down">
                        Wheel Down
                      </SelectItem>
                    </Select>
                    <Button
                      variant="outline"
                      size="icon"
                      className="h-10 w-10 rounded-[8px] border-destructive-subtle bg-destructive-subtle text-destructive hover:bg-destructive/20 hover:text-destructive"
                      onPress={() => removeSequenceRow(row.id)}
                      isDisabled={sequenceRows.length <= 1}
                    >
                      <X className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                ))}
              </div>
              <div className="flex items-start justify-between">
                <Badge
                  variant="outline"
                  className="py-3 font-medium text-foreground-muted text-sm"
                >
                  {sequenceRows.length} / 8 used
                </Badge>
                <Button
                  variant="outline"
                  className="text-sm"
                  onPress={addSequenceRow}
                  isDisabled={sequenceRows.length >= 8}
                >
                  <Plus className="h-3 w-3" />
                  <span>Add Step</span>
                </Button>
              </div>
              <p className="text-foreground-muted text-xs">
                Supported steps: Left / Right / Up / Down / Wheel Up / Wheel
                Down / Left Click / Right Click / Middle Click
              </p>
              {gestureMode === "hold" && (
                <div className="flex flex-col gap-2">
                  <span className="font-medium text-foreground text-sm">
                    Step
                  </span>
                  <p className="text-foreground-muted text-xs">
                    Single non-movement input that fires while the trigger is
                    held.
                  </p>
                  <Select
                    value={finalStep}
                    onChange={(key) => {
                      setFinalStep(String(key))
                      setIsDirty(true)
                    }}
                    className="w-full"
                  >
                    <SelectItem id="wheel-up" textValue="Wheel Up">
                      Wheel Up
                    </SelectItem>
                    <SelectItem id="wheel-down" textValue="Wheel Down">
                      Wheel Down
                    </SelectItem>
                    <SelectItem id="left-click" textValue="Left Click">
                      Left Click
                    </SelectItem>
                    <SelectItem id="right-click" textValue="Right Click">
                      Right Click
                    </SelectItem>
                    <SelectItem id="middle-click" textValue="Middle Click">
                      Middle Click
                    </SelectItem>
                  </Select>
                </div>
              )}
            </div>
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
    </AppSettingsLayout>
  )
}
