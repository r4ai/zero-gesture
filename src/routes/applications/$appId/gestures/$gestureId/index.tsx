import { createFileRoute, useNavigate, useParams } from "@tanstack/react-router"
import { Check, Keyboard, Plus, Trash2, X } from "lucide-react"
import { useCallback, useEffect, useMemo, useState } from "react"
import {
  AppSettingsLayout,
  getGestureId,
} from "@/components/applications/app-settings-layout"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { KeyInput } from "@/components/ui/key-input"
import { Select, SelectItem } from "@/components/ui/select"
import { TabItem, TabList, Tabs } from "@/components/ui/tabs"
import { TextField } from "@/components/ui/textfield"
import { useConfigDraft } from "@/contexts/config-draft"
import type {
  GestureBinding,
  GestureMode,
  GestureStep,
  TriggerButton,
} from "@/types/config"

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

type SequenceRow = {
  id: number
  step: GestureStep
}

function toUiButton(trigger: TriggerButton) {
  if (trigger === "left_click") return "left-click"
  if (trigger === "middle_click") return "middle-click"
  return "right-click"
}

function fromUiButton(trigger: string): TriggerButton {
  if (trigger === "left-click") return "left_click"
  if (trigger === "middle-click") return "middle_click"
  return "right_click"
}

function toUiStep(step: GestureStep) {
  return step.split("_").join("-")
}

function fromUiStep(step: string): GestureStep {
  return step.split("-").join("_") as GestureStep
}

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
        inputClassName="h-auto px-2 py-0 font-semibold text-foreground text-lg"
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
  onSequenceRowChange: (rowId: number, step: string) => void
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
        </div>
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
      </div>

      <div className="flex flex-col gap-3">
        <span className="font-medium text-foreground text-sm">Sequence</span>
        <div className="flex flex-col gap-2">
          {sequenceRows.map((row, index) => (
            <div key={row.id} className="flex items-center gap-2">
              <span className="w-3 text-center text-[12px] text-foreground-muted">
                {index + 1}
              </span>
              <Select
                value={toUiStep(row.step)}
                onChange={(key) => onSequenceRowChange(row.id, String(key))}
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
        {gestureMode === "hold" && (
          <div className="flex flex-col gap-2">
            <span className="font-medium text-foreground text-sm">Step</span>
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
            </Select>
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * Action Edit Page - Edit gesture action.
 */
function ActionEditPage() {
  const { appId, gestureId } = useParams({
    from: "/applications/$appId/gestures/$gestureId/",
  })
  const search = Route.useSearch()
  const navigate = useNavigate()
  const { draft, setDraft, reset, save, isDirty, isSaving } = useConfigDraft()
  const bindings = draft.bindings[appId] ?? []
  const selectedTab = search.tab ?? "gesture"
  const gesture = useMemo(
    () => bindings.find((item) => getGestureId(item) === gestureId),
    [bindings, gestureId],
  )
  const [isDirtyLocal, setIsDirtyLocal] = useState(false)

  const editedTitle = gesture?.label ?? "Untitled"
  const editedKeys = gesture?.action.keys ?? []
  const gestureMode = gesture?.gesture.mode ?? "release"
  const triggerButton = toUiButton(gesture?.gesture.trigger ?? "right_click")
  const sequenceRows: SequenceRow[] = (gesture?.gesture.sequence ?? []).map(
    (step, index) => ({ id: index + 1, step }),
  )
  const finalStep =
    gesture?.gesture.mode === "hold"
      ? toUiStep(gesture.gesture.step)
      : "wheel-up"

  const patchGesture = useCallback(
    (patch: Partial<GestureBinding>) => {
      if (!gesture) return
      setDraft({
        ...draft,
        bindings: {
          ...draft.bindings,
          [appId]: bindings.map((item) =>
            getGestureId(item) === gestureId ? { ...item, ...patch } : item,
          ),
        },
      })
      setIsDirtyLocal(true)
    },
    [appId, bindings, draft, gesture, gestureId, setDraft],
  )

  const patchGestureShape = (next: GestureBinding["gesture"]) => {
    if (!gesture) return
    patchGesture({ gesture: next })
  }

  const onSave = () => {
    save()
    setIsDirtyLocal(false)
  }

  const onCancel = () => {
    reset()
    setIsDirtyLocal(false)
  }

  const handleRemoveAction = () => {
    if (!gesture) return
    patchGesture({ action: { type: "keyboard", keys: [] } })
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

  useEffect(() => {
    const shortcut =
      typeof search.shortcut === "string" ? search.shortcut : undefined
    if (!shortcut || !gesture) return

    const next = shortcut
      .split(",")
      .map((part: string) => part.trim())
      .filter((part: string) => part.length > 0)

    patchGesture({ action: { type: "keyboard", keys: next } })
    navigate({
      to: "/applications/$appId/gestures/$gestureId",
      params: { appId, gestureId },
      search: search.tab ? { tab: search.tab } : {},
      replace: true,
    })
  }, [
    appId,
    gestureId,
    gesture,
    navigate,
    patchGesture,
    search.shortcut,
    search.tab,
  ])

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
        onTitleChange={(title) => patchGesture({ label: title })}
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
            onKeysChange={(keys) =>
              patchGesture({ action: { type: "keyboard", keys } })
            }
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
              patchGestureShape({
                ...gesture.gesture,
                trigger: fromUiButton(key),
              })
            }}
            onGestureModeChange={(mode) => {
              if (mode === "hold") {
                patchGestureShape({
                  mode: "hold",
                  trigger: gesture.gesture.trigger,
                  sequence: gesture.gesture.sequence,
                  step: "wheel_up",
                })
                return
              }
              patchGestureShape({
                mode: "release",
                trigger: gesture.gesture.trigger,
                sequence: gesture.gesture.sequence,
              })
            }}
            onSequenceRowChange={(rowId, step) => {
              const next = sequenceRows.map((row) =>
                row.id === rowId ? { ...row, step: fromUiStep(step) } : row,
              )
              patchGestureShape({
                ...gesture.gesture,
                sequence: next.map((row) => row.step),
              })
            }}
            onAddSequenceRow={() => {
              patchGestureShape({
                ...gesture.gesture,
                sequence: [...gesture.gesture.sequence, "right"],
              })
            }}
            onRemoveSequenceRow={(rowId) => {
              const next = sequenceRows
                .filter((row) => row.id !== rowId)
                .map((row) => row.step)
              if (next.length === 0) return
              patchGestureShape({
                ...gesture.gesture,
                sequence: next,
              })
            }}
            onFinalStepChange={(key) => {
              if (gesture.gesture.mode !== "hold") return
              patchGestureShape({
                ...gesture.gesture,
                step: fromUiStep(key) as "wheel_up" | "wheel_down",
              })
            }}
          />
        )}
      </div>
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
          isDisabled={!isDirty || isSaving || !isDirtyLocal}
        >
          <Check className="h-4 w-4" />
          <span>{isSaving ? "Saving..." : "Save Changes"}</span>
        </Button>
      </div>
    </AppSettingsLayout>
  )
}
