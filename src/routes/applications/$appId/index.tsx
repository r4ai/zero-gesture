import {
  createFileRoute,
  redirect,
  useNavigate,
  useParams,
} from "@tanstack/react-router"
import { Check, Keyboard, Loader2, X } from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { getGestureId } from "@/components/applications/app-settings-layout"
import { Button } from "@/components/ui/button"
import { ComboBox, ComboBoxItem } from "@/components/ui/combobox"
import { Dialog, DialogContent } from "@/components/ui/dialog"
import { DEFAULT_BINDINGS } from "@/types/config"

type KeyboardInputMode = "wait" | "manual"

interface KeyboardInputSearch {
  mode?: KeyboardInputMode
  gestureId?: string
  keys?: string
  tab?: "gesture" | "action"
}

const MODIFIER_KEYS = ["Ctrl", "Alt", "Shift", "Win"] as const
const SHORTCUT_KEYS = [
  "A",
  "B",
  "C",
  "D",
  "E",
  "F",
  "G",
  "H",
  "I",
  "J",
  "K",
  "L",
  "M",
  "N",
  "O",
  "P",
  "Q",
  "R",
  "S",
  "T",
  "U",
  "V",
  "W",
  "X",
  "Y",
  "Z",
  "F1",
  "F2",
  "F3",
  "F4",
  "F5",
  "F6",
  "F7",
  "F8",
  "F9",
  "F10",
  "F11",
  "F12",
  "Space",
  "Enter",
  "Tab",
  "Backspace",
] as const

export const Route = createFileRoute("/applications/$appId/")({
  validateSearch: (search: Record<string, unknown>): KeyboardInputSearch => {
    const mode =
      search.mode === "wait" || search.mode === "manual"
        ? search.mode
        : undefined
    const gestureId =
      typeof search.gestureId === "string" && search.gestureId.length > 0
        ? search.gestureId
        : undefined
    const keys =
      typeof search.keys === "string" && search.keys.length > 0
        ? search.keys
        : undefined
    const tab =
      search.tab === "gesture" || search.tab === "action"
        ? search.tab
        : undefined

    return { mode, gestureId, keys, tab }
  },
  beforeLoad: ({ params, search }) => {
    const targetGestureId =
      search.gestureId || getGestureId(DEFAULT_BINDINGS[0])

    if (!search.mode) {
      throw redirect({
        to: "/applications/$appId/gestures/$gestureId",
        params: {
          appId: params.appId,
          gestureId: targetGestureId,
        },
      })
    }
  },
  component: KeyboardInputPage,
})

function parseKeys(raw?: string): string[] {
  if (!raw) return []

  const normalize = (value: string): string => {
    const key = value.trim()
    const lower = key.toLowerCase()

    if (lower === "ctrl" || lower === "control") return "Ctrl"
    if (lower === "alt" || lower === "option") return "Alt"
    if (lower === "shift") return "Shift"
    if (
      lower === "meta" ||
      lower === "command" ||
      lower === "cmd" ||
      lower === "win" ||
      lower === "windows"
    ) {
      return "Win"
    }
    if (key.length === 1) return key.toUpperCase()
    return key
  }

  return raw
    .split(",")
    .map((part) => normalize(part))
    .filter((part) => part.length > 0)
}

function normalizePressedKey(key: string): string | null {
  const normalizedKey = key.toLowerCase()

  if (
    normalizedKey === "meta" ||
    normalizedKey === "control" ||
    normalizedKey === "alt" ||
    normalizedKey === "shift"
  ) {
    return null
  }

  if (normalizedKey === " ") return "Space"
  if (normalizedKey === "arrowup") return "ArrowUp"
  if (normalizedKey === "arrowdown") return "ArrowDown"
  if (normalizedKey === "arrowleft") return "ArrowLeft"
  if (normalizedKey === "arrowright") return "ArrowRight"

  if (key.length === 1) {
    return key.toUpperCase()
  }

  return key
}

function modifierSymbol(key: string): string {
  if (key === "Ctrl") return "Ctrl"
  if (key === "Alt") return "Alt"
  if (key === "Shift") return "Shift"
  if (key === "Win") return "Win"
  return key
}

function KeyboardInputPage() {
  const { appId } = useParams({ from: "/applications/$appId/" })
  const search = Route.useSearch()
  const navigate = useNavigate()

  const targetGestureId = search.gestureId || getGestureId(DEFAULT_BINDINGS[0])
  const initialKeys = useMemo(() => parseKeys(search.keys), [search.keys])
  const [waitPreviewKeys, setWaitPreviewKeys] = useState<string[]>(initialKeys)
  const [selectedModifiers, setSelectedModifiers] = useState<Set<string>>(
    () =>
      new Set(
        initialKeys.filter((key) =>
          MODIFIER_KEYS.includes(key as (typeof MODIFIER_KEYS)[number]),
        ),
      ),
  )
  const [selectedKey, setSelectedKey] = useState<string>(
    initialKeys.find(
      (key) => !MODIFIER_KEYS.includes(key as (typeof MODIFIER_KEYS)[number]),
    ) || "",
  )
  const captureFinishedRef = useRef(false)

  const closeAndReturn = useCallback(
    (keys?: string[]) => {
      navigate({
        to: "/applications/$appId/gestures/$gestureId",
        params: { appId, gestureId: targetGestureId },
        search:
          keys && keys.length > 0
            ? { shortcut: keys.join(","), tab: search.tab }
            : search.tab
              ? { tab: search.tab }
              : {},
        replace: true,
      })
    },
    [appId, navigate, search.tab, targetGestureId],
  )

  useEffect(() => {
    if (search.mode !== "wait") return

    captureFinishedRef.current = false

    const buildComboFromEvent = (event: KeyboardEvent): string[] => {
      const next: string[] = []
      if (event.ctrlKey) next.push("Ctrl")
      if (event.altKey) next.push("Alt")
      if (event.shiftKey) next.push("Shift")
      if (event.metaKey) next.push("Win")

      const main = normalizePressedKey(event.key)
      if (main) next.push(main)
      return next
    }

    const buildModifiersFromEvent = (event: KeyboardEvent): string[] => {
      const next: string[] = []
      if (event.ctrlKey) next.push("Ctrl")
      if (event.altKey) next.push("Alt")
      if (event.shiftKey) next.push("Shift")
      if (event.metaKey) next.push("Win")
      return next
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        closeAndReturn()
        return
      }

      const next = buildComboFromEvent(event)
      if (next.length === 0) return

      event.preventDefault()
      setWaitPreviewKeys(next)
    }

    const onKeyUp = (event: KeyboardEvent) => {
      if (captureFinishedRef.current) return

      if (event.key === "Escape") {
        event.preventDefault()
        closeAndReturn()
        return
      }

      const releasedKey = normalizePressedKey(event.key)
      const comboAfterKeyUp = buildModifiersFromEvent(event)
      setWaitPreviewKeys(comboAfterKeyUp)

      if (!releasedKey) return

      const finalized = [...comboAfterKeyUp, releasedKey]
      captureFinishedRef.current = true
      closeAndReturn(finalized)
    }

    window.addEventListener("keydown", onKeyDown)
    window.addEventListener("keyup", onKeyUp)
    return () => {
      window.removeEventListener("keydown", onKeyDown)
      window.removeEventListener("keyup", onKeyUp)
    }
  }, [search.mode, closeAndReturn])

  const manualPreview = [
    ...MODIFIER_KEYS.filter((key) => selectedModifiers.has(key)),
    selectedKey,
  ].filter((part) => part.length > 0)

  const toggleModifier = (modifier: string) => {
    setSelectedModifiers((previous) => {
      const next = new Set(previous)
      if (next.has(modifier)) {
        next.delete(modifier)
      } else {
        next.add(modifier)
      }
      return next
    })
  }

  if (search.mode === "manual") {
    return (
      <Dialog isOpen onOpenChange={(isOpen) => !isOpen && closeAndReturn()}>
        <span className="hidden" />
        <DialogContent isDismissable modalClassName="max-w-[560px]">
          <div className="flex items-center justify-between p-4 pb-3">
            <div className="flex items-center gap-2">
              <Keyboard className="h-[18px] w-[18px] text-foreground-muted" />
              <h2 className="font-semibold text-[16px] text-foreground">
                Create Key Combination
              </h2>
            </div>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 bg-background-glass hover:bg-background-glass-medium"
              onPress={() => closeAndReturn()}
            >
              <X className="h-3.5 w-3.5 text-foreground-subtle" />
            </Button>
          </div>

          <div className="h-px bg-border" />

          <div className="flex flex-col gap-5 p-7 pt-4">
            <div className="flex flex-col gap-2.5">
              <span className="text-[12px] text-foreground-subtle">
                Modifier Keys
              </span>
              <div className="flex flex-wrap gap-2">
                {MODIFIER_KEYS.map((modifier) => {
                  const isSelected = selectedModifiers.has(modifier)
                  return (
                    <Button
                      key={modifier}
                      variant="ghost"
                      className={`h-9 rounded-[8px] border px-3 text-[13px] ${
                        isSelected
                          ? "border-border-bright bg-background-card text-foreground"
                          : "bg-background-glass text-foreground-muted"
                      }`}
                      onPress={() => toggleModifier(modifier)}
                    >
                      {modifierSymbol(modifier)}
                    </Button>
                  )
                })}
              </div>
            </div>

            <div className="flex flex-col gap-2.5">
              <span className="text-[12px] text-foreground-subtle">Key</span>
              <ComboBox
                placeholder="Select a key..."
                selectedKey={selectedKey || null}
                onSelectionChange={(key) => setSelectedKey(String(key || ""))}
              >
                {SHORTCUT_KEYS.map((key) => (
                  <ComboBoxItem key={key} id={key} textValue={key}>
                    {key}
                  </ComboBoxItem>
                ))}
              </ComboBox>
            </div>

            <div className="h-px bg-border" />

            <div className="flex flex-col gap-2.5">
              <span className="text-[12px] text-foreground-subtle">
                Preview
              </span>
              <div className="flex h-14 items-center justify-center gap-1.5 rounded-[10px] border border-border-muted bg-background-glass">
                {manualPreview.length > 0 ? (
                  manualPreview.map((key, index) => (
                    <div
                      // biome-ignore lint/suspicious/noArrayIndexKey: Preview order is fixed and display-only.
                      key={`${key}-${index}`}
                      className="flex items-center gap-1.5"
                    >
                      <span className="inline-flex h-8 min-w-[34px] items-center justify-center rounded-md border border-border-muted bg-background-card px-3 font-semibold text-[14px] text-foreground">
                        {modifierSymbol(key)}
                      </span>
                      {index < manualPreview.length - 1 && (
                        <span className="text-[14px] text-foreground-muted">
                          +
                        </span>
                      )}
                    </div>
                  ))
                ) : (
                  <span className="text-[13px] text-foreground-faint">
                    No key selected
                  </span>
                )}
              </div>
            </div>

            <div className="flex justify-end gap-2.5">
              <Button
                variant="outline"
                className="h-9 w-[100px] border-border-muted text-[12px]"
                onPress={() => closeAndReturn()}
              >
                Cancel
              </Button>
              <Button
                className="h-9 w-[120px] text-[13px]"
                onPress={() => closeAndReturn(manualPreview)}
                isDisabled={manualPreview.length === 0}
              >
                <Check className="h-3.5 w-3.5" />
                <span>Assign</span>
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    )
  }

  return (
    <Dialog isOpen onOpenChange={(isOpen) => !isOpen && closeAndReturn()}>
      <span className="hidden" />
      <DialogContent isDismissable modalClassName="max-w-[520px]">
        <div className="flex justify-end px-2.5 pt-2.5">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 bg-background-glass hover:bg-background-glass-medium"
            onPress={() => closeAndReturn()}
          >
            <X className="h-3.5 w-3.5 text-foreground-subtle" />
          </Button>
        </div>

        <div className="flex flex-col items-center gap-5 px-8 pt-5 pb-9">
          <div className="flex h-[72px] w-[72px] items-center justify-center rounded-full bg-background-glass-medium">
            <Loader2 className="h-8 w-8 animate-spin text-foreground" />
          </div>

          <div className="flex flex-col items-center gap-2">
            <h2 className="font-semibold text-[20px] text-foreground">
              Press a Key Combination
            </h2>
            <p className="max-w-[420px] text-center text-[14px] text-foreground-muted">
              Press any key combination to assign it as a shortcut.
              <br />
              Hold modifier keys (Ctrl, Alt, Shift, Win) together with a key.
            </p>
          </div>

          <div className="flex h-16 w-full items-center justify-center gap-2 rounded-[10px] border border-border-muted bg-background-glass">
            {waitPreviewKeys.length > 0 ? (
              waitPreviewKeys.map((key, index) => (
                <div
                  // biome-ignore lint/suspicious/noArrayIndexKey: Preview order is fixed and display-only.
                  key={`${key}-${index}`}
                  className="flex items-center gap-2"
                >
                  <span className="inline-flex h-8 min-w-[34px] items-center justify-center rounded-md border border-border-muted bg-background-card px-3 font-semibold text-[14px] text-foreground">
                    {modifierSymbol(key)}
                  </span>
                  {index < waitPreviewKeys.length - 1 && (
                    <span className="text-[14px] text-foreground-muted">+</span>
                  )}
                </div>
              ))
            ) : (
              <>
                <span className="h-1.5 w-1.5 rounded-full bg-foreground-faint" />
                <span className="h-1.5 w-1.5 rounded-full bg-foreground-faint" />
                <span className="h-1.5 w-1.5 rounded-full bg-foreground-faint" />
              </>
            )}
          </div>

          <div className="flex h-8 items-center gap-2 rounded-md border border-border-muted bg-background-card px-3">
            <span className="inline-flex h-5 min-w-[28px] items-center justify-center rounded bg-background-subtle px-1.5 font-semibold text-[10px] text-foreground-subtle">
              Esc
            </span>
            <span className="text-[13px] text-foreground-subtle">
              Press Esc to cancel
            </span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
