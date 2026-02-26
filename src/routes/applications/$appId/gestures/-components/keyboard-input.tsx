import {
  ArrowDown,
  ArrowRight,
  Check,
  Keyboard,
  Loader2,
  X,
} from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Button } from "@/components/ui/button"
import { ComboBox, ComboBoxItem } from "@/components/ui/combobox"
import { Dialog, DialogContent } from "@/components/ui/dialog"
import type { KeyboardSequence } from "@/types/config"

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/**
 * Modifier keys supported by the backend (executor.rs).
 * These match the key names used in config files.
 */
export const MODIFIER_KEYS = ["ctrl", "alt", "shift", "win"] as const
export type ModifierKey = (typeof MODIFIER_KEYS)[number]

/**
 * All supported keys that can be used in keyboard shortcuts.
 * Matches the keys supported by executor.rs parse_key function.
 *
 * Categories:
 * - Letters: a-z
 * - Numbers: 0-9
 * - Function keys: f1-f24
 * - Navigation: left, right, up, down, tab, enter, escape, backspace, delete, home, end, pageup, pagedown
 * - Space
 */
export const SHORTCUT_KEYS = [
  // Letters a-z
  "a",
  "b",
  "c",
  "d",
  "e",
  "f",
  "g",
  "h",
  "i",
  "j",
  "k",
  "l",
  "m",
  "n",
  "o",
  "p",
  "q",
  "r",
  "s",
  "t",
  "u",
  "v",
  "w",
  "x",
  "y",
  "z",
  // Numbers 0-9
  "0",
  "1",
  "2",
  "3",
  "4",
  "5",
  "6",
  "7",
  "8",
  "9",
  // Function keys f1-f24
  "f1",
  "f2",
  "f3",
  "f4",
  "f5",
  "f6",
  "f7",
  "f8",
  "f9",
  "f10",
  "f11",
  "f12",
  "f13",
  "f14",
  "f15",
  "f16",
  "f17",
  "f18",
  "f19",
  "f20",
  "f21",
  "f22",
  "f23",
  "f24",
  // Navigation keys
  "left",
  "right",
  "up",
  "down",
  "tab",
  "enter",
  "escape",
  "backspace",
  "delete",
  "home",
  "end",
  "pageup",
  "pagedown",
  // Space
  "space",
] as const

export type KeyboardInputMode = "wait" | "manual"

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/**
 * Parse a key-combo string into an array of normalized key names.
 *
 * Supported separators:
 * - `+` (e.g. `Ctrl+Shift+Z`)
 * - `,` (legacy format)
 * - whitespace (e.g. `Ctrl Shift Z`)
 *
 * Normalized key names match the backend (executor.rs) format.
 *
 * @example
 * parseKeys("ctrl+alt+a") // => ["ctrl", "alt", "a"]
 * parseKeys("Ctrl + Alt + A") // => ["ctrl", "alt", "a"]
 * parseKeys("pgup pgdn")  // => ["pageup", "pagedown"]
 */
export function parseKeys(raw?: string): string[] {
  if (!raw) return []

  const normalize = (value: string): string | null => {
    const key = value.trim()
    if (key.length === 0) return null

    const lower = key.toLowerCase()

    // Modifiers
    if (lower === "ctrl" || lower === "control") return "ctrl"
    if (lower === "alt" || lower === "menu" || lower === "option") return "alt"
    if (lower === "shift") return "shift"
    if (
      lower === "meta" ||
      lower === "command" ||
      lower === "cmd" ||
      lower === "win" ||
      lower === "windows" ||
      lower === "lwin" ||
      lower === "super"
    ) {
      return "win"
    }

    // Navigation / editing aliases
    if (lower === "return") return "enter"
    if (lower === "esc") return "escape"
    if (lower === "del") return "delete"
    if (lower === "pgup") return "pageup"
    if (lower === "pgdn") return "pagedown"

    // Single characters (letters and numbers)
    if (key.length === 1) {
      const ch = key.charCodeAt(0)
      // a-z
      if (ch >= 65 && ch <= 90) return String.fromCharCode(ch + 32) // A-Z -> a-z
      if (ch >= 97 && ch <= 122) return key // already lowercase
      // 0-9
      if (ch >= 48 && ch <= 57) return key
      return null
    }

    // Check if it's a valid function key (f1-f24)
    if (lower.startsWith("f")) {
      const num = lower.slice(1)
      if (/^\d+$/.test(num)) {
        const n = Number.parseInt(num, 10)
        if (n >= 1 && n <= 24) return lower
      }
    }

    // Other supported keys (must match SHORTCUT_KEYS)
    const supportedKeys = SHORTCUT_KEYS as unknown as string[]
    if (supportedKeys.includes(lower)) return lower

    // Unknown key - return null to filter it out
    return null
  }

  return raw
    .split(/[,+]/)
    .flatMap((part) => part.trim().split(/\s+/))
    .map((part) => normalize(part))
    .filter((part): part is string => part !== null && part.length > 0)
}

/**
 * Parse a key sequence where combos are comma-separated.
 *
 * @example
 * parseKeySequence("f21+a, ctrl+x, shift+z")
 * // => [["f21", "a"], ["ctrl", "x"], ["shift", "z"]]
 */
export function parseKeySequence(raw?: string): KeyboardSequence {
  if (!raw) return []

  return raw
    .split(",")
    .map((combo) => parseKeys(combo))
    .filter((combo) => combo.length > 0)
}

/**
 * Normalize a raw `KeyboardEvent.key` value to a backend-compatible key name.
 * Returns `null` for bare modifier keys (they are tracked via `event.ctrlKey` etc.).
 *
 * This function mirrors the parsing logic in executor.rs parse_key function.
 *
 * @example
 * normalizePressedKey("a")          // => "a"
 * normalizePressedKey(" ")          // => "space"
 * normalizePressedKey("Control")    // => null
 * normalizePressedKey("ArrowUp")    // => "up"
 * normalizePressedKey("F1")         // => "f1"
 */
export function normalizePressedKey(key: string): string | null {
  const lower = key.toLowerCase()

  // Bare modifier keys - return null as they are tracked via event.*Key
  if (
    lower === "meta" ||
    lower === "control" ||
    lower === "alt" ||
    lower === "shift"
  ) {
    return null
  }

  // Space
  if (lower === " ") return "space"

  // Arrow keys -> navigation names
  if (lower === "arrowup") return "up"
  if (lower === "arrowdown") return "down"
  if (lower === "arrowleft") return "left"
  if (lower === "arrowright") return "right"

  // Navigation / editing key aliases
  if (lower === "return") return "enter"
  if (lower === "esc") return "escape"
  if (lower === "del") return "delete"

  // Single character keys (a-z, 0-9)
  if (key.length === 1) {
    const ch = key.charCodeAt(0)
    // a-z
    if (ch >= 65 && ch <= 90) return String.fromCharCode(ch + 32) // A-Z -> a-z
    if (ch >= 97 && ch <= 122) return key // already lowercase
    // 0-9
    if (ch >= 48 && ch <= 57) return key
    return null
  }

  // Function keys - normalize to lowercase f1-f24
  const fnMatch = lower.match(/^f(\d+)$/)
  if (fnMatch) {
    const fnNumber = Number(fnMatch[1])
    if (fnNumber >= 1 && fnNumber <= 24) {
      return lower
    }
  }

  // Other keys - check if supported
  const supportedKeys = SHORTCUT_KEYS as unknown as string[]
  if (supportedKeys.includes(lower)) return lower

  return null
}

/**
 * Return a human-readable label for a key.
 * Converts backend key names to display format.
 *
 * @example
 * keyLabel("ctrl") // => "Ctrl"
 * keyLabel("f1") // => "F1"
 * keyLabel("pageup") // => "PageUp"
 */
export function keyLabel(key: string): string {
  if (!key) return ""

  const lower = key.toLowerCase()

  // Modifiers - capitalize first letter
  if (lower === "ctrl") return "Ctrl"
  if (lower === "alt") return "Alt"
  if (lower === "shift") return "Shift"
  if (lower === "win") return "Win"

  // Function keys - uppercase F
  if (lower.match(/^f\d+$/)) {
    return key.toUpperCase()
  }

  // Navigation keys with special casing
  if (lower === "pageup") return "PageUp"
  if (lower === "pagedown") return "PageDown"

  // Single letters - uppercase
  if (key.length === 1 && lower >= "a" && lower <= "z") {
    return key.toUpperCase()
  }

  // Default - capitalize first letter
  return key.charAt(0).toUpperCase() + key.slice(1)
}

/**
 * @deprecated Use keyLabel instead
 */
export function modifierLabel(key: string): string {
  return keyLabel(key)
}

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

/**
 * Hook for capturing modifier state from a `KeyboardEvent`.
 *
 * @example
 * const modifiers = buildModifiersFromEvent(event) // => ["ctrl", "alt"]
 */
function buildModifiersFromEvent(event: KeyboardEvent): string[] {
  const next: string[] = []
  if (event.ctrlKey) next.push("ctrl")
  if (event.altKey) next.push("alt")
  if (event.shiftKey) next.push("shift")
  if (event.metaKey) next.push("win")
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
  const pressedNonModifiersRef = useRef<string[]>([])
  const comboCapturedRef = useRef(false)

  useEffect(() => {
    if (!active) return
    pressedNonModifiersRef.current = []
    comboCapturedRef.current = false

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        onCancel()
        return
      }

      const pressed = pressedNonModifiersRef.current
      const normalized = normalizePressedKey(event.key)
      if (normalized && !pressed.includes(normalized)) {
        pressed.push(normalized)
      }

      const next = [...buildModifiersFromEvent(event), ...pressed]
      if (next.length === 0) return
      event.preventDefault()
      onPreview(next)
    }

    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        onCancel()
        return
      }

      const releasedKey = normalizePressedKey(event.key)
      const pressed = pressedNonModifiersRef.current
      const modifiers = buildModifiersFromEvent(event)

      if (!releasedKey) {
        const next = [...modifiers, ...pressed]
        onPreview(next)
        if (comboCapturedRef.current && next.length === 0) {
          comboCapturedRef.current = false
        }
        return
      }

      event.preventDefault()
      const finalized = [...modifiers, ...pressed]

      const releaseIndex = pressed.indexOf(releasedKey)
      if (releaseIndex >= 0) {
        pressed.splice(releaseIndex, 1)
      }

      const previewAfterRelease = [...modifiers, ...pressed]
      onPreview(previewAfterRelease)

      if (!comboCapturedRef.current && finalized.length > 0) {
        onConfirm(finalized)
        comboCapturedRef.current = true
      }

      if (comboCapturedRef.current && previewAfterRelease.length === 0) {
        comboCapturedRef.current = false
      }
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
    <div className="flex flex-wrap items-center justify-center gap-1">
      {keys.map((key, index) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: Preview order is fixed and display-only.
          key={`${key}-${index}`}
          className="inline-flex items-center gap-1"
        >
          <span className="inline-flex h-8 min-w-[30px] items-center justify-center rounded-md border border-border-muted bg-background-card px-2.5 font-semibold text-[14px] text-foreground">
            {keyLabel(key)}
          </span>
          {index < keys.length - 1 && (
            <span className="text-[13px] text-foreground-muted">+</span>
          )}
        </span>
      ))}
    </div>
  )
}

function KeySequencePreview({
  sequence,
  activeCombo = [],
  emptyFallback,
}: {
  sequence: KeyboardSequence
  activeCombo?: string[]
  emptyFallback?: React.ReactNode
}) {
  const displaySequence =
    activeCombo.length > 0 ? [...sequence, activeCombo] : sequence

  if (displaySequence.length === 0) {
    return <>{emptyFallback}</>
  }

  return (
    <div className="flex flex-wrap items-center justify-center gap-2">
      {displaySequence.map((combo, comboIndex) => (
        <div
          // biome-ignore lint/suspicious/noArrayIndexKey: Sequence order is user-defined and display-only.
          key={`sequence-${comboIndex}`}
          className="flex items-center gap-1.5"
        >
          <KeyComboPreview keys={combo} />
          {comboIndex < displaySequence.length - 1 && (
            <ArrowRight className="h-3.5 w-3.5 text-foreground-muted" />
          )}
        </div>
      ))}
    </div>
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
  initialSequence = [],
  onConfirm,
  onClose,
}: {
  isOpen: boolean
  initialSequence?: KeyboardSequence
  onConfirm: (sequence: KeyboardSequence) => void
  onClose: () => void
}) {
  const [capturedSequence, setCapturedSequence] =
    useState<KeyboardSequence>(initialSequence)
  const [activeCombo, setActiveCombo] = useState<string[]>([])

  // `initialSequence` を ref に保持することで、isOpen が true に変わった瞬間の
  // 値だけを使ってプレビューをリセットできる
  const initialSequenceRef = useRef(initialSequence)
  initialSequenceRef.current = initialSequence

  useEffect(() => {
    if (!isOpen) return
    setCapturedSequence(initialSequenceRef.current)
    setActiveCombo([])
  }, [isOpen])

  useKeyCapture({
    active: isOpen,
    onPreview: setActiveCombo,
    onConfirm: (keys) => {
      setCapturedSequence((previous) => [...previous, keys])
    },
    onCancel: onClose,
  })

  const handleConfirm = () => {
    onConfirm(capturedSequence)
    onClose()
  }

  const removeLastCombo = () => {
    setCapturedSequence((previous) => previous.slice(0, -1))
  }

  const clearAll = () => {
    setCapturedSequence([])
    setActiveCombo([])
  }

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
              Capture Key Sequence
            </h2>
            <p className="max-w-[420px] text-center text-[14px] text-foreground-muted">
              Press key combinations in order to build the action sequence.
              <br />A combo is added each time you release a non-modifier key.
            </p>
          </div>

          <div className="flex min-h-16 w-full items-center justify-center rounded-[10px] border border-border-muted bg-background-glass px-4 py-2">
            <KeySequencePreview
              sequence={capturedSequence}
              activeCombo={activeCombo}
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
              Press Esc to cancel capture
            </span>
          </div>

          <div className="flex w-full justify-end gap-2.5">
            <Button
              variant="outline"
              className="h-9 border-border-muted text-[12px]"
              onPress={clearAll}
              isDisabled={capturedSequence.length === 0}
            >
              Clear
            </Button>
            <Button
              variant="outline"
              className="h-9 border-border-muted text-[12px]"
              onPress={removeLastCombo}
              isDisabled={capturedSequence.length === 0}
            >
              Undo
            </Button>
            <Button
              className="h-9 min-w-[120px] text-[13px]"
              onPress={handleConfirm}
              isDisabled={capturedSequence.length === 0}
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
  initialSequence = [],
  onConfirm,
  onClose,
}: {
  isOpen: boolean
  initialSequence?: KeyboardSequence
  onConfirm: (sequence: KeyboardSequence) => void
  onClose: () => void
}) {
  const [sequence, setSequence] = useState<KeyboardSequence>(initialSequence)
  const [selectedModifiers, setSelectedModifiers] = useState<Set<string>>(
    () => new Set<string>(),
  )
  const [selectedKeys, setSelectedKeys] = useState<string[]>([])
  const [selectedKeyCandidate, setSelectedKeyCandidate] = useState<string>("")
  const resetCurrentCombo = useCallback(() => {
    setSelectedModifiers(new Set<string>())
    setSelectedKeys([])
    setSelectedKeyCandidate("")
  }, [])

  // Reset state whenever the dialog opens with new initialSequence
  const prevIsOpen = useRef(false)
  useEffect(() => {
    if (isOpen && !prevIsOpen.current) {
      setSequence(initialSequence)
      resetCurrentCombo()
    }
    prevIsOpen.current = isOpen
  }, [isOpen, initialSequence, resetCurrentCombo])

  const preview = useMemo(
    () =>
      [
        ...MODIFIER_KEYS.filter((key) => selectedModifiers.has(key)),
        ...selectedKeys,
      ].filter((part) => part.length > 0),
    [selectedModifiers, selectedKeys],
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

  const addCombo = () => {
    if (preview.length === 0) return
    setSequence((previous) => [...previous, preview])
    resetCurrentCombo()
  }

  const addSelectedKey = () => {
    if (!selectedKeyCandidate) return
    setSelectedKeys((previous) => {
      if (previous.includes(selectedKeyCandidate)) return previous
      return [...previous, selectedKeyCandidate]
    })
    setSelectedKeyCandidate("")
  }

  const removeSelectedKey = (keyToRemove: string, indexToRemove: number) => {
    setSelectedKeys((previous) =>
      previous.filter(
        (key, index) => !(key === keyToRemove && index === indexToRemove),
      ),
    )
  }

  const removeCombo = (index: number) => {
    setSequence((previous) =>
      previous.filter((_, current) => current !== index),
    )
  }

  const clearAllSequence = () => {
    setSequence([])
  }

  const handleConfirm = () => {
    onConfirm(sequence)
    onClose()
  }

  return (
    <Dialog isOpen={isOpen} onOpenChange={(open) => !open && onClose()}>
      <span className="hidden" />
      <DialogContent
        isDismissable
        modalClassName="max-w-[560px] max-h-[90vh] overflow-hidden flex flex-col"
        dialogClassName="flex flex-col min-h-0"
      >
        <div className="flex shrink-0 items-center justify-between p-4 pb-3">
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

        <div className="h-px shrink-0 bg-border" />

        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-7 pt-4">
          <div className="rounded-xl border border-border-muted bg-background-glass p-4">
            <div className="mb-3">
              <div className="flex flex-col gap-1">
                <span className="font-medium text-[13px] text-foreground">
                  Current Combo
                </span>
                <span className="text-[12px] text-foreground-subtle">
                  Build one combo, then add it to sequence.
                </span>
              </div>
            </div>

            <div className="flex flex-col gap-3">
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
                        {keyLabel(modifier)}
                      </Button>
                    )
                  })}
                </div>
              </div>

              <div className="flex flex-col gap-2.5">
                <span className="text-[12px] text-foreground-subtle">Keys</span>
                <div className="flex items-center gap-2">
                  <div className="flex-1">
                    <ComboBox
                      placeholder="Select a key..."
                      selectedKey={selectedKeyCandidate || null}
                      onSelectionChange={(key) =>
                        setSelectedKeyCandidate(String(key ?? ""))
                      }
                    >
                      {SHORTCUT_KEYS.map((key) => (
                        <ComboBoxItem
                          key={key}
                          id={key}
                          textValue={keyLabel(key)}
                        >
                          {keyLabel(key)}
                        </ComboBoxItem>
                      ))}
                    </ComboBox>
                  </div>
                  <Button
                    variant="outline"
                    className="h-9 border-border-muted px-3 text-[12px]"
                    onPress={addSelectedKey}
                    isDisabled={
                      !selectedKeyCandidate ||
                      selectedKeys.includes(selectedKeyCandidate)
                    }
                  >
                    Add Key
                  </Button>
                </div>
                {selectedKeys.length > 0 && (
                  <div className="flex flex-wrap gap-1.5">
                    {selectedKeys.map((key, index) => (
                      <button
                        // biome-ignore lint/suspicious/noArrayIndexKey: Selected key order is user-defined.
                        key={`selected-${key}-${index}`}
                        type="button"
                        className="inline-flex h-8 items-center gap-1 rounded-md border border-border-muted bg-background-card px-2.5 font-semibold text-[13px] text-foreground"
                        onClick={() => removeSelectedKey(key, index)}
                      >
                        <span>{keyLabel(key)}</span>
                        <X className="h-3.5 w-3.5 text-foreground-subtle hover:text-destructive" />
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <div className="flex items-center gap-2 rounded-lg bg-background-subtle px-3 py-2.5">
                <span className="shrink-0 text-[12px] text-foreground-subtle">
                  Preview:
                </span>
                <div className="flex flex-1 items-center">
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
            </div>
          </div>

          <div className="flex justify-center">
            <Button
              variant="outline"
              className="h-8 gap-1.5 border-border-muted px-4 text-[12px]"
              onPress={addCombo}
              isDisabled={preview.length === 0}
            >
              <ArrowDown className="h-3.5 w-3.5" />
              Add to Sequence
            </Button>
          </div>

          <div className="rounded-xl border border-border-muted bg-background-glass p-4">
            <div className="mb-3 flex items-start justify-between gap-2">
              <div className="flex flex-col gap-1">
                <span className="font-medium text-[13px] text-foreground">
                  Sequence
                </span>
                <span className="text-[12px] text-foreground-subtle">
                  Ordered combos executed from left to right.
                </span>
              </div>
              <Button
                variant="outline"
                className="h-8 border-destructive-subtle bg-destructive-subtle text-[12px] text-destructive hover:bg-destructive/20 hover:text-destructive"
                onPress={clearAllSequence}
                isDisabled={sequence.length === 0}
              >
                Clear All
              </Button>
            </div>

            <div className="flex min-h-10 flex-wrap items-center gap-2">
              {sequence.length > 0 ? (
                sequence.map((combo, index) => (
                  <div
                    // biome-ignore lint/suspicious/noArrayIndexKey: Sequence order is user-defined and stable in this view.
                    key={`manual-sequence-${index}`}
                    className="flex items-center gap-1.5"
                  >
                    <div className="flex items-center gap-1.5 rounded-md border border-border-muted bg-background-card px-2 py-1">
                      <KeyComboPreview keys={combo} />
                      <Button
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6 text-foreground-muted hover:bg-destructive-subtle hover:text-destructive"
                        onPress={() => removeCombo(index)}
                      >
                        <X className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                    {index < sequence.length - 1 && (
                      <ArrowRight className="h-3.5 w-3.5 text-foreground-muted" />
                    )}
                  </div>
                ))
              ) : (
                <span className="text-[13px] text-foreground-faint">
                  Add a combo from Current Combo to start sequence
                </span>
              )}
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
              isDisabled={sequence.length === 0}
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
