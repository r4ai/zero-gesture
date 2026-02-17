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
import { TextField } from "@/components/ui/textfield"

export const Route = createFileRoute(
  "/applications/$appId/gestures/$gestureId/",
)({
  validateSearch: (
    search: Record<string, unknown>,
  ): { shortcut?: string; tab?: ActionEditTab } => {
    const shortcut =
      typeof search.shortcut === "string" && search.shortcut.length > 0
        ? search.shortcut
        : undefined
    const tab =
      search.tab === "gesture" || search.tab === "action"
        ? search.tab
        : undefined

    return { shortcut, tab }
  },
  component: ActionEditPage,
})

type ActionEditTab = "gesture" | "action"
type GestureMode = "hold" | "release"

type SequenceRow = {
  id: number
  type: string
  step: string
}

const INITIAL_SEQUENCE_ROWS: SequenceRow[] = [
  { id: 1, type: "mouse-move", step: "left" },
  { id: 2, type: "mouse-move", step: "right" },
]

function ActionEditHeader({
  title,
  onTitleChange,
  onRemoveAction,
}: {
  title: string
  onTitleChange: (title: string) => void
  onRemoveAction: () => void
}) {
  return (
    <div className="flex h-16 items-center justify-between border-border border-b px-6">
      <TextField
        variant="transparent"
        value={title}
        onChange={onTitleChange}
        aria-label="Action title"
        className="w-full max-w-[420px]"
        inputClassName="h-auto font-semibold text-foreground text-lg py-0 px-2"
      />
      <Button
        variant="outline"
        className="h-8 gap-2 rounded-md border-destructive-subtle bg-destructive-subtle text-[12px] text-destructive hover:bg-destructive/20 hover:text-destructive"
        onPress={onRemoveAction}
      >
        <Trash2 className="h-3.5 w-3.5" />
        <span>Remove Action</span>
      </Button>
    </div>
  )
}

function ActionEditTabs({
  selectedTab,
  onSelectionChange,
}: {
  selectedTab: ActionEditTab
  onSelectionChange: (tab: ActionEditTab) => void
}) {
  return (
    <div className="flex h-14 items-center border-border px-6 py-2">
      <Tabs
        selectedKey={selectedTab}
        onSelectionChange={(key: string | number) =>
          onSelectionChange(String(key) as ActionEditTab)
        }
        className="w-full"
      >
        <TabList className="flex h-10 items-center gap-1 rounded-xl border border-border bg-background-card p-1">
          <TabItem id="gesture">Gesture</TabItem>
          <TabItem id="action">Action</TabItem>
        </TabList>
      </Tabs>
    </div>
  )
}

function ActionTabContent({
  editedKeys,
  onKeysChange,
  onWaitMode,
  onManualMode,
}: {
  editedKeys: string[]
  onKeysChange: (keys: string[]) => void
  onWaitMode: () => void
  onManualMode: () => void
}) {
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <span className="font-medium text-foreground text-sm">Action Type</span>
        <Select value="keyboard" onChange={() => {}} className="w-full">
          <SelectItem id="keyboard" textValue="Keyboard Shortcut">
            <div className="flex items-center gap-2">
              <div className="flex h-4 w-4 items-center justify-center">
                <Keyboard className="h-3.5 w-3.5" />
              </div>
              <span className="font-medium text-[13px]">Keyboard Shortcut</span>
            </div>
          </SelectItem>
        </Select>
      </div>

      <div className="h-px bg-border" />

      <KeyInput
        keys={editedKeys}
        onChange={onKeysChange}
        onPress={onWaitMode}
        onKeyboardPress={onManualMode}
      />
    </div>
  )
}

function GestureTabContent({
  triggerButton,
  gestureMode,
  sequenceRows,
  finalStep,
  onTriggerButtonChange,
  onGestureModeChange,
  onSequenceRowChange,
  onAddSequenceRow,
  onRemoveSequenceRow,
  onFinalStepChange,
}: {
  triggerButton: string
  gestureMode: GestureMode
  sequenceRows: SequenceRow[]
  finalStep: string
  onTriggerButtonChange: (key: string) => void
  onGestureModeChange: (mode: GestureMode) => void
  onSequenceRowChange: (
    rowId: number,
    patch: Partial<Pick<SequenceRow, "type" | "step">>,
  ) => void
  onAddSequenceRow: () => void
  onRemoveSequenceRow: (rowId: number) => void
  onFinalStepChange: (key: string) => void
}) {
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2">
        <span className="font-medium text-foreground text-sm">
          Trigger Button
        </span>
        <div className="flex h-10 items-center gap-2">
          <Select
            value={triggerButton}
            onChange={(key) => onTriggerButtonChange(String(key))}
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
        <span className="font-medium text-foreground text-sm">
          Gesture Mode
        </span>
        <Tabs
          selectedKey={gestureMode}
          onSelectionChange={(key: string | number) =>
            onGestureModeChange(String(key) as GestureMode)
          }
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
        <span className="font-medium text-foreground text-sm">Sequence</span>
        <p className="text-foreground-muted text-xs">
          Ordered steps recognized before the final step fires. Up to 8 steps.
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
                  onSequenceRowChange(row.id, { type: String(key) })
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
                  onSequenceRowChange(row.id, { step: String(key) })
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
                onPress={() => onRemoveSequenceRow(row.id)}
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
            onPress={onAddSequenceRow}
            isDisabled={sequenceRows.length >= 8}
          >
            <Plus className="h-3 w-3" />
            <span>Add Step</span>
          </Button>
        </div>
        <p className="text-foreground-muted text-xs">
          Supported steps: Left / Right / Up / Down / Wheel Up / Wheel Down /
          Left Click / Right Click / Middle Click
        </p>
        {gestureMode === "hold" && (
          <div className="flex flex-col gap-2">
            <span className="font-medium text-foreground text-sm">Step</span>
            <p className="text-foreground-muted text-xs">
              Single non-movement input that fires while the trigger is held.
            </p>
            <Select
              value={finalStep}
              onChange={(key) => onFinalStepChange(String(key))}
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
  )
}

function ActionEditFooter({
  isDirty,
  onCancel,
  onSave,
}: {
  isDirty: boolean
  onCancel: () => void
  onSave: () => void
}) {
  return (
    <div className="flex h-16 items-center justify-end gap-3 border-border border-t px-6">
      <Button
        variant="outline"
        className="h-9 px-4 text-[13px]"
        onPress={onCancel}
        isDisabled={!isDirty}
      >
        Cancel
      </Button>
      <Button
        className="h-9 gap-2 px-4 text-[13px]"
        onPress={onSave}
        isDisabled={!isDirty}
      >
        <Check className="h-4 w-4" />
        <span>Save Changes</span>
      </Button>
    </div>
  )
}

/**
 * Action Edit Page - Edit gesture action (right panel)
 * Based on Pencil: "Applications Settings - Gesture Edit"
 */
function ActionEditPage() {
  const { appId, gestureId } = useParams({
    from: "/applications/$appId/gestures/$gestureId/",
  })
  const search = Route.useSearch()
  const navigate = useNavigate()
  const gesture = GESTURES.find((item) => getGestureId(item) === gestureId)
  const originalKeys = gesture?.action.keys ?? []
  const selectedTab = search.tab ?? "gesture"
  const [gestureMode, setGestureMode] = useState<GestureMode>("hold")
  const [triggerButton, setTriggerButton] = useState<string>("right-click")
  const [sequenceRows, setSequenceRows] = useState<SequenceRow[]>(
    INITIAL_SEQUENCE_ROWS,
  )
  const [finalStep, setFinalStep] = useState<string>("wheel-up")
  const [editedKeys, setEditedKeys] = useState<string[]>(originalKeys)
  const [editedTitle, setEditedTitle] = useState(gesture?.label ?? "Untitled")
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
    setSequenceRows(INITIAL_SEQUENCE_ROWS)
    setFinalStep("wheel-up")
    setEditedTitle(gesture?.label ?? "Untitled")
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
        tab: selectedTab,
      },
    })
  }

  const updateSequenceRow = (
    rowId: number,
    patch: Partial<Pick<SequenceRow, "type" | "step">>,
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
      search: search.tab ? { tab: search.tab } : {},
      replace: true,
    })
  }, [appId, gestureId, navigate, search.shortcut, search.tab])

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
      <ActionEditHeader
        title={editedTitle}
        onTitleChange={(title) => {
          setEditedTitle(title)
          setIsDirty(true)
        }}
        onRemoveAction={handleRemoveAction}
      />
      <ActionEditTabs
        selectedTab={selectedTab}
        onSelectionChange={(tab) => {
          navigate({
            to: "/applications/$appId/gestures/$gestureId",
            params: { appId, gestureId },
            search: { tab },
            replace: true,
          })
        }}
      />
      <div className="flex-1 overflow-y-auto p-6">
        {selectedTab === "action" && (
          <ActionTabContent
            editedKeys={editedKeys}
            onKeysChange={(keys) => {
              setEditedKeys(keys)
              setIsDirty(true)
            }}
            onWaitMode={() => openKeyboardInput("wait")}
            onManualMode={() => openKeyboardInput("manual")}
          />
        )}

        {selectedTab === "gesture" && (
          <GestureTabContent
            triggerButton={triggerButton}
            gestureMode={gestureMode}
            sequenceRows={sequenceRows}
            finalStep={finalStep}
            onTriggerButtonChange={(key) => {
              setTriggerButton(key)
              setIsDirty(true)
            }}
            onGestureModeChange={(mode) => {
              setGestureMode(mode)
              setIsDirty(true)
            }}
            onSequenceRowChange={updateSequenceRow}
            onAddSequenceRow={addSequenceRow}
            onRemoveSequenceRow={removeSequenceRow}
            onFinalStepChange={(key) => {
              setFinalStep(key)
              setIsDirty(true)
            }}
          />
        )}
      </div>

      <ActionEditFooter
        isDirty={isDirty}
        onCancel={handleCancel}
        onSave={handleSave}
      />
    </AppSettingsLayout>
  )
}
