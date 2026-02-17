import { Keyboard, X } from "lucide-react"
import { Badge } from "./badge"
import { Button } from "./button"

export interface KeyInputProps {
  /** Array of keys currently in the shortcut */
  keys: string[]
  /** Callback when keys change */
  onChange: (keys: string[]) => void
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
 * Displays keys as badges with plus separators
 */
export function KeyInput({
  keys,
  onChange,
  onPress,
  onKeyboardPress,
  placeholder = "Enter shortcut...",
  hint = "Enter modifiers and a key (e.g. Ctrl+Shift+A)",
  isDisabled = false,
}: KeyInputProps) {
  const handleClear = () => {
    onChange([])
  }

  return (
    <div className="flex flex-col gap-2">
      <span className="font-medium text-[12px] text-foreground-subtle">
        Shortcut
      </span>
      <div className="flex h-[40px] items-center gap-1.5">
        <button
          type="button"
          className="flex h-[40px] flex-1 items-center gap-2 rounded-[8px] border bg-background-card px-3 text-left transition-colors hover:bg-background-subtle focus-visible:outline focus-visible:outline-2 focus-visible:outline-foreground focus-visible:outline-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
          onClick={onPress}
          disabled={isDisabled}
        >
          {keys.length > 0 ? (
            <div className="flex flex-1 items-center gap-1.5 overflow-hidden">
              {keys.map((key, index) => (
                <div
                  // biome-ignore lint/suspicious/noArrayIndexKey: Keys are display-only and order is fixed
                  key={`${key}-${index}`}
                  className="flex items-center gap-1.5"
                >
                  <Badge variant="key">{key}</Badge>
                  {index < keys.length - 1 && (
                    <span className="text-[12px] text-foreground-subtle">
                      +
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
            <Keyboard className="h-3.5 w-3.5 text-foreground-muted" />
          </Button>
          {keys.length > 0 && (
            <Button
              variant="outline"
              size="icon"
              className="h-10 w-10 rounded-[8px] border-border bg-background-card hover:bg-background-subtle"
              onPress={handleClear}
              isDisabled={isDisabled}
            >
              <X className="h-3.5 w-3.5 text-foreground-muted" />
            </Button>
          )}
        </div>
      </div>
      {hint && <p className="text-[11px] text-foreground-faint">{hint}</p>}
    </div>
  )
}
