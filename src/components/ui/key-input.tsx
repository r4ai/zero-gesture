import { Keyboard, X } from "lucide-react"
import { Badge } from "./badge"
import { Button } from "./button"

export interface KeyInputProps {
  /** Array of keys currently in the shortcut */
  keys: string[]
  /** Callback when keys change */
  onChange: (keys: string[]) => void
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
      <div className="flex h-[40px] items-center gap-2 rounded-[8px] border border-border-bright bg-background-card px-3">
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
                  <span className="text-[12px] text-foreground-subtle">+</span>
                )}
              </div>
            ))}
          </div>
        ) : (
          <span className="flex-1 text-[13px] text-foreground-faint">
            {placeholder}
          </span>
        )}

        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            isDisabled={isDisabled}
          >
            <Keyboard className="h-3.5 w-3.5 text-foreground-muted" />
          </Button>
          {keys.length > 0 && (
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
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
