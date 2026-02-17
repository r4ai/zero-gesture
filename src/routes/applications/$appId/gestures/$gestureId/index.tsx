import { createFileRoute, useNavigate, useParams } from "@tanstack/react-router"
import { Check, Keyboard, Trash2 } from "lucide-react"
import { useEffect, useState } from "react"
import {
  AppSettingsLayout,
  formatGestureSequence,
  GESTURES,
  getGestureId,
} from "@/components/applications/app-settings-layout"
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
  const [selectedTab, setSelectedTab] = useState<string>("action")
  const [editedKeys, setEditedKeys] = useState<string[]>(originalKeys)
  const [isDirty, setIsDirty] = useState(false)

  const handleSave = () => {
    // TODO: Save changes to backend
    setIsDirty(false)
  }

  const handleCancel = () => {
    // Reset to original values
    setEditedKeys(originalKeys)
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
          <h2 className="font-semibold text-[16px] text-foreground">
            {formatGestureSequence(gesture.gesture.sequence)}
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
              onPress={() => openKeyboardInput("wait")}
              onKeyboardPress={() => openKeyboardInput("manual")}
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
    </AppSettingsLayout>
  )
}
