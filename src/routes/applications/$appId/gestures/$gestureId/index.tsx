import { createFileRoute, useNavigate, useParams } from "@tanstack/react-router"
import { Keyboard, Plus, Trash2, X } from "lucide-react"
import { SettingsFormActions } from "@/components/settings-form-actions"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { KeyInput } from "@/components/ui/key-input"
import { Select, SelectItem } from "@/components/ui/select"
import { TabItem, TabList, Tabs } from "@/components/ui/tabs"
import { TextField } from "@/components/ui/textfield"
import { useConfigDraft } from "@/contexts/config-draft"
import {
  GESTURE_MODES,
  GESTURE_STEPS,
  type GestureStep,
  HOLD_STEPS,
  type HoldStep,
  TRIGGER_BUTTONS,
  type TriggerButton,
} from "@/types/config"
import { GestureNotFound } from "../-components/gesture-not-found"
import {
  ManualKeyInputDialog,
  WaitKeyInputDialog,
} from "../-components/keyboard-input"

export const Route = createFileRoute(
  "/applications/$appId/gestures/$gestureId/",
)({
  validateSearch: (
    search: Record<string, unknown>,
  ): { shortcut?: string; tab?: ActionEditTab; mode?: "wait" | "manual" } => {
    const shortcut =
      typeof search.shortcut === "string" && search.shortcut.length > 0
        ? search.shortcut
        : undefined
    const tab =
      search.tab === "gesture" || search.tab === "action"
        ? search.tab
        : undefined
    const mode =
      search.mode === "wait" || search.mode === "manual"
        ? search.mode
        : undefined

    return { shortcut, tab, mode }
  },
  component: ActionEditPage,
})

type ActionEditTab = "gesture" | "action"
type GestureMode = "hold" | "release"

function useGesture() {
  const { appId, gestureId } = useParams({
    from: "/applications/$appId/gestures/$gestureId/",
  })
  const { draft, setDraft } = useConfigDraft()

  const gesture = draft.bindings[appId]?.find((item) => item.id === gestureId)

  const setGesture = (updatedGesture: NonNullable<typeof gesture>) => {
    setDraft({
      ...draft,
      bindings: {
        ...draft.bindings,
        [appId]: draft.bindings[appId].map((item) =>
          item.id === gestureId ? updatedGesture : item,
        ),
      },
    })
  }

  const removeGesture = () => {
    setDraft({
      ...draft,
      bindings: {
        ...draft.bindings,
        [appId]: draft.bindings[appId].filter((item) => item.id !== gestureId),
      },
    })
  }

  return { gesture, setGesture, removeGesture, appId, gestureId }
}

function ActionEditHeader() {
  const { gesture, setGesture, removeGesture } = useGesture()
  if (!gesture) return null

  return (
    <div className="flex h-16 items-center justify-between border-border border-b px-6">
      <TextField
        variant="transparent"
        value={gesture.label}
        onChange={(value) => setGesture({ ...gesture, label: value })}
        aria-label="Action title"
        className="w-full max-w-[420px]"
        inputClassName="h-auto font-semibold text-foreground text-lg py-0 px-2"
      />
      <Button
        variant="outline"
        className="h-8 gap-2 rounded-md border-destructive-subtle bg-destructive-subtle text-[12px] text-destructive hover:bg-destructive/20 hover:text-destructive"
        onPress={removeGesture}
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

function ActionTabContent() {
  const { gesture, setGesture, appId, gestureId } = useGesture()
  const navigate = useNavigate()
  const search = Route.useSearch()

  if (!gesture) return null

  const selectedTab = search.tab === "action" ? "action" : "gesture"

  const actionType = gesture.action.type
  const onActionTypeChange = (type?: string) => {
    if (type !== "keyboard") {
      // TODO: handle unsupported action type
      console.error("Unsupported action type:", type)
      return
    }
    setGesture({
      ...gesture,
      action: {
        ...gesture.action,
        type,
      },
    })
  }

  const keys = gesture.action.keys ?? []
  const onKeysChange = (updatedKeys: string[]) => {
    setGesture({
      ...gesture,
      action: { ...gesture.action, keys: updatedKeys },
    })
  }

  const openKeyboardInput = (mode: "wait" | "manual") => {
    navigate({
      to: "/applications/$appId/gestures/$gestureId",
      params: { appId, gestureId },
      search: { tab: selectedTab, mode },
      replace: true,
    })
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-2">
        <span className="font-medium text-foreground text-sm">Action Type</span>
        <Select
          value={actionType}
          onChange={(key) => onActionTypeChange(key?.toString())}
          className="w-full"
        >
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
        keys={keys}
        onChange={onKeysChange}
        onPress={() => openKeyboardInput("wait")}
        onKeyboardPress={() => openKeyboardInput("manual")}
      />
    </div>
  )
}
function isValidTriggerButton(key?: string): key is TriggerButton {
  return (TRIGGER_BUTTONS as readonly string[]).includes(key ?? "")
}

function isValidGestureMode(key?: string): key is GestureMode {
  return (GESTURE_MODES as readonly string[]).includes(key ?? "")
}

function isValidGestureStep(key?: string): key is GestureStep {
  return (GESTURE_STEPS as readonly string[]).includes(key ?? "")
}

function isValidHoldStep(key?: string): key is HoldStep {
  return (HOLD_STEPS as readonly string[]).includes(key ?? "")
}

function GestureTabContent() {
  const { gesture, setGesture } = useGesture()
  if (!gesture) return <GestureNotFound />

  const triggerButton = gesture.gesture.trigger
  const onTriggerButtonChange = (key?: string) => {
    if (!isValidTriggerButton(key)) {
      // TODO: handle invalid value
      console.error("Invalid trigger button:", key)
      return
    }
    setGesture({ ...gesture, gesture: { ...gesture.gesture, trigger: key } })
  }

  const gestureMode = gesture.gesture.mode
  const onGestureModeChange = (mode?: string) => {
    if (!isValidGestureMode(mode)) {
      // TODO: handle invalid value
      console.error("Invalid gesture mode:", mode)
      return
    }
    setGesture({
      ...gesture,
      gesture: { step: "wheel_up", ...gesture.gesture, mode },
    })
  }

  const sequence = gesture.gesture.sequence
  const onStepChange = (index: number, step?: string) => {
    if (!isValidGestureStep(step)) {
      // TODO: handle invalid value
      console.error("Invalid sequence step:", step)
      return
    }
    const nextSequence = [...sequence]
    nextSequence[index] = step
    setGesture({
      ...gesture,
      gesture: { ...gesture.gesture, sequence: nextSequence },
    })
  }
  const onAddStep = () => {
    if (sequence.length >= 8) return
    const nextSequence: GestureStep[] = [...sequence, "left"]
    setGesture({
      ...gesture,
      gesture: { ...gesture.gesture, sequence: nextSequence },
    })
  }
  const onRemoveStep = (index: number) => {
    const nextSequence = sequence.filter((_, i) => i !== index)
    setGesture({
      ...gesture,
      gesture: { ...gesture.gesture, sequence: nextSequence },
    })
  }

  const holdStep =
    gesture.gesture.mode === "hold" ? gesture.gesture.step : "wheel-up"
  const onHoldStepChange = (step?: string) => {
    if (!isValidHoldStep(step)) {
      // TODO: handle invalid value
      console.error("Invalid hold step:", step)
      return
    }
    if (gesture.gesture.mode !== "hold") {
      // TODO: handle invalid state
      console.error("Hold step can only be set in hold mode")
      return
    }
    setGesture({
      ...gesture,
      gesture: { ...gesture.gesture, step },
    })
  }

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2">
        <span className="font-medium text-foreground text-sm">
          Trigger Button
        </span>
        <div className="flex h-10 items-center gap-2">
          <Select
            value={triggerButton}
            onChange={(key) => onTriggerButtonChange(key?.toString())}
            className="flex-1"
          >
            <SelectItem id="right_click" textValue="Right Click">
              Right Click
            </SelectItem>
            <SelectItem id="left_click" textValue="Left Click">
              Left Click
            </SelectItem>
            <SelectItem id="middle_click" textValue="Middle Click">
              Middle Click
            </SelectItem>
          </Select>

          {/* TODO: make this button works */}
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
          onSelectionChange={(key) => onGestureModeChange(key.toString())}
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
          {sequence.map((step, index) => (
            <Sequence
              // biome-ignore lint/suspicious/noArrayIndexKey: Using index as key is acceptable here since the order is fixed and won't change
              key={index}
              step={step}
              index={index}
              onStepChange={onStepChange}
              onRemoveStep={onRemoveStep}
            />
          ))}
        </div>
        <div className="flex items-start justify-between">
          <Badge
            variant="outline"
            className="py-3 font-medium text-foreground-muted text-sm"
          >
            {sequence.length} / 8 used
          </Badge>
          <Button
            variant="outline"
            className="text-sm"
            onPress={onAddStep}
            isDisabled={sequence.length >= 8}
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
              value={holdStep}
              onChange={(key) => onHoldStepChange(key?.toString())}
              className="w-full"
            >
              <SelectItem id="wheel_up" textValue="Wheel Up">
                Wheel Up
              </SelectItem>
              <SelectItem id="wheel_down" textValue="Wheel Down">
                Wheel Down
              </SelectItem>
            </Select>
          </div>
        )}
      </div>
    </div>
  )
}

function Sequence({
  step,
  index,
  onStepChange,
  onRemoveStep,
}: {
  step: string
  index: number
  onStepChange: (index: number, step?: string) => void
  onRemoveStep: (index: number) => void
}) {
  return (
    <div key={index} className="flex items-center gap-2">
      <span className="w-3 text-center text-[12px] text-foreground-muted">
        {index + 1}
      </span>
      <Select
        value={step}
        onChange={(key) => onStepChange(index, key?.toString())}
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
        <SelectItem id="wheel_up" textValue="Wheel Up">
          Wheel Up
        </SelectItem>
        <SelectItem id="wheel_down" textValue="Wheel Down">
          Wheel Down
        </SelectItem>
      </Select>
      <Button
        variant="outline"
        size="icon"
        className="h-10 w-10 rounded-[8px] border-destructive-subtle bg-destructive-subtle text-destructive hover:bg-destructive/20 hover:text-destructive"
        onPress={() => onRemoveStep(index)}
      >
        <X className="h-3.5 w-3.5" />
      </Button>
    </div>
  )
}

/**
 * Action Edit Page - Edit gesture action (right panel)
 * Based on Pencil: "Applications Settings - Gesture Edit"
 */
function ActionEditPage() {
  const { gesture, setGesture, appId, gestureId } = useGesture()
  const search = Route.useSearch()
  const navigate = useNavigate()

  const selectedTab = search.tab ?? "gesture"
  const keyboardMode = search.mode

  const closeKeyboardInput = () => {
    navigate({
      to: "/applications/$appId/gestures/$gestureId",
      params: { appId, gestureId },
      search: { tab: selectedTab },
      replace: true,
    })
  }

  if (!gesture) {
    return <GestureNotFound />
  }

  const keys = gesture.action.keys ?? []

  const handleKeysConfirm = (updatedKeys: string[]) => {
    setGesture({
      ...gesture,
      action: { ...gesture.action, keys: updatedKeys },
    })
    closeKeyboardInput()
  }

  return (
    <>
      <ActionEditHeader />
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
        {selectedTab === "action" && <ActionTabContent />}
        {selectedTab === "gesture" && <GestureTabContent />}
      </div>

      <SettingsFormActions />

      <WaitKeyInputDialog
        isOpen={keyboardMode === "wait"}
        initialKeys={keys}
        onConfirm={handleKeysConfirm}
        onClose={closeKeyboardInput}
      />
      <ManualKeyInputDialog
        isOpen={keyboardMode === "manual"}
        initialKeys={keys}
        onConfirm={handleKeysConfirm}
        onClose={closeKeyboardInput}
      />
    </>
  )
}
