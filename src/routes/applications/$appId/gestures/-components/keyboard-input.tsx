import { Check, Keyboard, Loader2, X } from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"
import { Button } from "@/components/ui/button"
import { ComboBox, ComboBoxItem } from "@/components/ui/combobox"
import { Dialog, DialogContent } from "@/components/ui/dialog"

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const MODIFIER_KEYS = ["Ctrl", "Alt", "Shift", "Win"] as const
export type ModifierKey = (typeof MODIFIER_KEYS)[number]

export const SHORTCUT_KEYS = [
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

export type KeyboardInputMode = "wait" | "manual"

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/**
 * Parse a comma-separated key string into an array of normalized key names.
 *
 * @example
 * parseKeys("Ctrl,Alt,A") // => ["Ctrl", "Alt", "A"]
 * parseKeys("ctrl,a")     // => ["Ctrl", "A"]
 */
export function parseKeys(raw?: string): string[] {
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

/**
 * Normalize a raw `KeyboardEvent.key` value to a display-friendly key name.
 * Returns `null` for bare modifier keys (they are tracked via `event.ctrlKey` etc.).
 *
 * @example
 * normalizePressedKey("a")          // => "A"
 * normalizePressedKey(" ")          // => "Space"
 * normalizePressedKey("Control")    // => null
 */
export function normalizePressedKey(key: string): string | null {
  const lower = key.toLowerCase()

  if (
    lower === "meta" ||
    lower === "control" ||
    lower === "alt" ||
    lower === "shift"
  ) {
    return null
  }

  if (lower === " ") return "Space"
  if (lower === "arrowup") return "ArrowUp"
  if (lower === "arrowdown") return "ArrowDown"
  if (lower === "arrowleft") return "ArrowLeft"
  if (lower === "arrowright") return "ArrowRight"

  if (key.length === 1) return key.toUpperCase()
  return key
}

/**
 * Return a human-readable label for a modifier key.
 *
 * @example
 * modifierLabel("Win") // => "Win"
 */
export function modifierLabel(key: string): string {
  return key
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

/**
 * Hook for capturing modifier state from a `KeyboardEvent`.
 *
 * @example
 * const modifiers = buildModifiersFromEvent(event) // => ["Ctrl", "Alt"]
 */
function buildModifiersFromEvent(event: KeyboardEvent): string[] {
  const next: string[] = []
  if (event.ctrlKey) next.push("Ctrl")
  if (event.altKey) next.push("Alt")
  if (event.shiftKey) next.push("Shift")
  if (event.metaKey) next.push("Win")
  return next
}

function buildComboFromEvent(event: KeyboardEvent): string[] {
  const next = buildModifiersFromEvent(event)
  const main = normalizePressedKey(event.key)
  if (main) next.push(main)
  return next
}

/**
 * Hook that listens to global keyboard events and calls `onConfirm` when the
 * user releases a non-modifier key.  Calls `onCancel` when Escape is pressed.
 *
 * Only active when `active` is `true`.
 *
 * @example
 * useKeyCapture({
 *   active: mode === "wait",
 *   onPreview: setPreviewKeys,
 *   onConfirm: (keys) => console.log(keys),
 *   onCancel: () => console.log("cancelled"),
 * })
 */
export function useKeyCapture({
  active,
  onPreview,
  onConfirm,
  onCancel,
}: {
  active: boolean
  onPreview: (keys: string[]) => void
  onConfirm: (keys: string[]) => void
  onCancel: () => void
}) {
  const captureFinishedRef = useRef(false)

  useEffect(() => {
    if (!active) return

    captureFinishedRef.current = false

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        onCancel()
        return
      }
      const next = buildComboFromEvent(event)
      if (next.length === 0) return
      event.preventDefault()
      onPreview(next)
    }

    const onKeyUp = (event: KeyboardEvent) => {
      if (captureFinishedRef.current) return
      if (event.key === "Escape") {
        event.preventDefault()
        onCancel()
        return
      }
      const releasedKey = normalizePressedKey(event.key)
      const modifiersAfterRelease = buildModifiersFromEvent(event)
      onPreview(modifiersAfterRelease)
      if (!releasedKey) return
      const finalized = [...modifiersAfterRelease, releasedKey]
      captureFinishedRef.current = true
      onConfirm(finalized)
    }

    window.addEventListener("keydown", onKeyDown)
    window.addEventListener("keyup", onKeyUp)
    return () => {
      window.removeEventListener("keydown", onKeyDown)
      window.removeEventListener("keyup", onKeyUp)
    }
  }, [active, onPreview, onConfirm, onCancel])
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

/**
 * Renders a row of key badges separated by "+" symbols.
 *
 * @example
 * <KeyComboPreview keys={["Ctrl", "A"]} />
 */
function KeyComboPreview({
  keys,
  emptyFallback,
}: {
  keys: string[]
  emptyFallback?: React.ReactNode
}) {
  if (keys.length === 0) {
    return <>{emptyFallback}</>
  }

  return (
    <>
      {keys.map((key, index) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: Preview order is fixed and display-only.
          key={`${key}-${index}`}
          className="flex items-center gap-1.5"
        >
          <span className="inline-flex h-8 min-w-[34px] items-center justify-center rounded-md border border-border-muted bg-background-card px-3 font-semibold text-[14px] text-foreground">
            {modifierLabel(key)}
          </span>
          {index < keys.length - 1 && (
            <span className="text-[14px] text-foreground-muted">+</span>
          )}
        </div>
      ))}
    </>
  )
}

// ---------------------------------------------------------------------------
// Public components
// ---------------------------------------------------------------------------

/**
 * Modal that listens for a key combination pressed by the user (wait mode).
 *
 * @example
 * <WaitKeyInputDialog
 *   isOpen={mode === "wait"}
 *   initialKeys={["Ctrl", "A"]}
 *   onConfirm={(keys) => setKeys(keys)}
 *   onClose={() => setMode(undefined)}
 * />
 */
export function WaitKeyInputDialog({
  isOpen,
  initialKeys = [],
  onConfirm,
  onClose,
}: {
  isOpen: boolean
  initialKeys?: string[]
  onConfirm: (keys: string[]) => void
  onClose: () => void
}) {
  const [previewKeys, setPreviewKeys] = useState<string[]>(initialKeys)

  // `initialKeys` を ref に保持することで、isOpen が true に変わった瞬間の
  // 値だけを使ってプレビューをリセットできる
  const initialKeysRef = useRef(initialKeys)
  initialKeysRef.current = initialKeys

  useEffect(() => {
    if (isOpen) setPreviewKeys(initialKeysRef.current)
  }, [isOpen])

  useKeyCapture({
    active: isOpen,
    onPreview: setPreviewKeys,
    onConfirm: (keys) => {
      onConfirm(keys)
      onClose()
    },
    onCancel: onClose,
  })

  return (
    <Dialog isOpen={isOpen} onOpenChange={(open) => !open && onClose()}>
      <span className="hidden" />
      <DialogContent isDismissable modalClassName="max-w-[520px]">
        <div className="flex justify-end px-2.5 pt-2.5">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 bg-background-glass hover:bg-background-glass-medium"
            onPress={onClose}
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
            <KeyComboPreview
              keys={previewKeys}
              emptyFallback={
                <>
                  <span className="h-1.5 w-1.5 rounded-full bg-foreground-faint" />
                  <span className="h-1.5 w-1.5 rounded-full bg-foreground-faint" />
                  <span className="h-1.5 w-1.5 rounded-full bg-foreground-faint" />
                </>
              }
            />
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

/**
 * Modal that lets users manually pick modifier keys and a main key (manual mode).
 *
 * @example
 * <ManualKeyInputDialog
 *   isOpen={mode === "manual"}
 *   initialKeys={["Ctrl", "A"]}
 *   onConfirm={(keys) => setKeys(keys)}
 *   onClose={() => setMode(undefined)}
 * />
 */
export function ManualKeyInputDialog({
  isOpen,
  initialKeys = [],
  onConfirm,
  onClose,
}: {
  isOpen: boolean
  initialKeys?: string[]
  onConfirm: (keys: string[]) => void
  onClose: () => void
}) {
  const [selectedModifiers, setSelectedModifiers] = useState<Set<string>>(
    () =>
      new Set(
        initialKeys.filter((key) => MODIFIER_KEYS.includes(key as ModifierKey)),
      ),
  )
  const [selectedKey, setSelectedKey] = useState<string>(
    initialKeys.find((key) => !MODIFIER_KEYS.includes(key as ModifierKey)) ??
      "",
  )

  // Reset state whenever the dialog opens with new initialKeys
  const prevIsOpen = useRef(false)
  useEffect(() => {
    if (isOpen && !prevIsOpen.current) {
      setSelectedModifiers(
        new Set(
          initialKeys.filter((key) =>
            MODIFIER_KEYS.includes(key as ModifierKey),
          ),
        ),
      )
      setSelectedKey(
        initialKeys.find(
          (key) => !MODIFIER_KEYS.includes(key as ModifierKey),
        ) ?? "",
      )
    }
    prevIsOpen.current = isOpen
  }, [isOpen, initialKeys])

  const preview = useMemo(
    () =>
      [
        ...MODIFIER_KEYS.filter((key) => selectedModifiers.has(key)),
        selectedKey,
      ].filter((part) => part.length > 0),
    [selectedModifiers, selectedKey],
  )

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

  const handleConfirm = () => {
    onConfirm(preview)
    onClose()
  }

  return (
    <Dialog isOpen={isOpen} onOpenChange={(open) => !open && onClose()}>
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
            onPress={onClose}
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
                    {modifierLabel(modifier)}
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
              onSelectionChange={(key) => setSelectedKey(String(key ?? ""))}
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
            <span className="text-[12px] text-foreground-subtle">Preview</span>
            <div className="flex h-14 items-center justify-center gap-1.5 rounded-[10px] border border-border-muted bg-background-glass">
              <KeyComboPreview
                keys={preview}
                emptyFallback={
                  <span className="text-[13px] text-foreground-faint">
                    No key selected
                  </span>
                }
              />
            </div>
          </div>

          <div className="flex justify-end gap-2.5">
            <Button
              variant="outline"
              className="h-9 w-[100px] border-border-muted text-[12px]"
              onPress={onClose}
            >
              Cancel
            </Button>
            <Button
              className="h-9 w-[120px] text-[13px]"
              onPress={handleConfirm}
              isDisabled={preview.length === 0}
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
