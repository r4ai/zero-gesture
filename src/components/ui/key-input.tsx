import { Keyboard, X } from "lucide-react"
import type { KeyboardSequence } from "@/types/config"
import { Badge } from "./badge"
import { Button } from "./button"

export interface KeyInputProps {
  /** Ordered key combos currently configured in the shortcut sequence */
  sequence: KeyboardSequence
  /** Callback when sequence changes */
  onChange: (sequence: KeyboardSequence) => void
  /** Callback when main input area is pressed */
  onPress?: () => void
  /** Callback when keyboard icon button is pressed */
  onKeyboardPress?: () => void
  /** Placeholder text when empty */
  placeholder?: string
  /** Hint text displayed below the input */
  hint?: string
  /** Whether the input is disabled */
  isDisabled?: boolean
}

/**
 * Key input component for keyboard shortcuts
 * Displays key combos as badges with plus separators and combo sequence arrows.
 */
export function KeyInput({
  sequence,
  onChange,
  onPress,
  onKeyboardPress,
  placeholder = "Enter shortcut...",
  hint = "Enter one or more combos (e.g. F21+A, Ctrl+X, Shift+Z)",
  isDisabled = false,
}: KeyInputProps) {
  const hasSequence = sequence.length > 0

  const handleClear = () => {
    onChange([])
  }

  const formatDisplayKey = (key: string): string => {
    if (!key) return ""
    const lower = key.toLowerCase()

    if (lower === "ctrl") return "Ctrl"
    if (lower === "alt") return "Alt"
    if (lower === "shift") return "Shift"
    if (lower === "win") return "Win"
    if (lower === "pageup") return "PageUp"
    if (lower === "pagedown") return "PageDown"
    if (/^f\d+$/.test(lower)) return lower.toUpperCase()
    if (lower.length === 1 && lower >= "a" && lower <= "z")
      return lower.toUpperCase()
    return lower.charAt(0).toUpperCase() + lower.slice(1)
  }

  return (
    <div className="flex flex-col gap-2">
      <span className="font-medium text-foreground text-sm">Shortcut</span>
      <div className="flex h-[40px] items-center gap-1.5">
        <button
          type="button"
          className="flex h-[40px] flex-1 items-center gap-2 rounded-[8px] border bg-background-card px-3 text-left transition-colors hover:bg-background-subtle focus-visible:outline focus-visible:outline-2 focus-visible:outline-foreground focus-visible:outline-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
          onClick={onPress}
          disabled={isDisabled}
        >
          {hasSequence ? (
            <div className="flex flex-1 flex-wrap items-center gap-1.5 overflow-hidden">
              {sequence.map((combo, comboIndex) => (
                <div
                  // biome-ignore lint/suspicious/noArrayIndexKey: Sequence entries are display-only and order is fixed.
                  key={`combo-${comboIndex}`}
                  className="flex items-center gap-1.5"
                >
                  {combo.map((key, keyIndex) => (
                    <div
                      // biome-ignore lint/suspicious/noArrayIndexKey: Keys are display-only and order is fixed.
                      key={`${comboIndex}-${key}-${keyIndex}`}
                      className="flex items-center gap-1.5"
                    >
                      <Badge className="text-foreground" variant="key">
                        {formatDisplayKey(key)}
                      </Badge>
                      {keyIndex < combo.length - 1 && (
                        <span className="text-[12px] text-foreground-muted">
                          +
                        </span>
                      )}
                    </div>
                  ))}
                  {comboIndex < sequence.length - 1 && (
                    <span className="px-0.5 text-[12px] text-foreground-muted">
                      →
                    </span>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <span className="flex-1 text-[13px] text-foreground-faint">
              {placeholder}
            </span>
          )}
        </button>
        <div className="flex items-center gap-1.5">
          <Button
            variant="outline"
            size="icon"
            className="h-10 w-10 rounded-[8px] border-border bg-background-card hover:bg-background-subtle"
            onPress={onKeyboardPress}
            isDisabled={isDisabled}
          >
            <Keyboard className="h-3.5 w-3.5 text-foreground" />
          </Button>
          {hasSequence && (
            <Button
              variant="outline"
              size="icon"
              className="size-10 rounded-md border-destructive-subtle bg-destructive-subtle text-[12px] text-destructive hover:bg-destructive/20 hover:text-destructive"
              onPress={handleClear}
              isDisabled={isDisabled}
            >
              <X className="size-3.5" />
            </Button>
          )}
        </div>
      </div>
      {hint && <p className="text-foreground-muted text-xs">{hint}</p>}
    </div>
  )
}
