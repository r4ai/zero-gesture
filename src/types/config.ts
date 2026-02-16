/** What property of the foreground window to inspect. */
export type MatchTarget = "process_name" | "window_class" | "title"

/** How to compare the target value against the pattern. */
export type MatchMethod = "exact" | "contains" | "regex"

/** A single matching rule for identifying an application. */
export interface AppMatcher {
  /** What property of the foreground window to inspect. */
  target: MatchTarget
  /** How to compare the target value against the pattern. */
  method: MatchMethod
  /** The pattern to match against. */
  value: string
}

/** Definition of an application for per-app gesture bindings. */
export interface AppDefinition {
  /** Matching rules (OR logic — any match counts). */
  matchers: AppMatcher[]
}

/** Mouse button that starts a gesture session. */
export type TriggerButton = "left_click" | "right_click" | "middle_click"

/** One element inside a gesture sequence. */
export type GestureStep =
  | "up"
  | "down"
  | "left"
  | "right"
  | "wheel_up"
  | "wheel_down"
  | "left_click"
  | "right_click"
  | "middle_click"

/** Timing mode for a gesture binding. */
export type GestureMode = "release" | "hold"

/** Gesture pattern definition. */
export interface GesturePattern {
  /** Button that starts this gesture. */
  trigger: TriggerButton
  /** Whether this gesture runs on trigger release or while holding trigger. */
  mode?: GestureMode
  /**
   * Ordered sequence of movement/input steps.
   * - `release` mode: the full sequence to match on trigger release.
   * - `hold` mode: current recognized sequence required before `step` fires.
   */
  sequence?: GestureStep[]
  /** Single non-movement input step for `hold` mode. */
  step?: GestureStep
}

/** An action that can be triggered by a gesture. */
export type Action = { type: "keyboard"; keys: string[] }

/** A single gesture binding. */
export interface GestureBinding {
  label?: string
  /** Gesture pattern to match. */
  gesture: GesturePattern
  /** Action to execute when the gesture matches. */
  action: Action
}

/** Application-wide configuration persisted as JSON. */
export interface AppConfig {
  /** Whether gesture recognition is enabled. */
  enabled: boolean

  /** CSS colour string used to draw the gesture trail (e.g. `"#00BFFF"`). */
  trail_color: string

  /** Thickness in logical pixels for the gesture trail line. */
  trail_thickness: number

  /** Timeout in milliseconds used for stuck-state recovery. */
  safety_timeout_ms: number

  /** Minimum movement distance (in pixels) required to confirm a gesture direction segment. */
  min_segment_px: number

  /** Minimum movement distance (in pixels) required to switch to a new direction candidate. */
  direction_switch_confirm_px: number

  /** Deadzone (in pixels) used to ignore tiny ambiguous diagonal movement. */
  axis_ambiguity_deadzone_px: number

  /**
   * Maximum cursor travel distance (in pixels) to replay the original
   * trigger-button click when no gesture binding matches.
   * If movement exceeds this threshold, replay is skipped.
   */
  replay_distance_threshold_px: number

  /** Font family name for the gesture label overlay. */
  label_font_family: string

  /** Font size in pixels for the gesture label overlay. */
  label_font_size: number

  /** Font weight for the gesture label overlay (Win32 range: 0..=1000). */
  label_font_weight: number

  /** Padding in pixels around the gesture label text. */
  label_padding: number

  /** Named app definitions for per-app gesture bindings. */
  apps: Record<string, AppDefinition>

  /**
   * Gesture bindings grouped by app ID.
   * - `"default"` is the global fallback set.
   * - other keys reference entries in `apps`.
   */
  bindings: Record<string, GestureBinding[]>
}

/** Hard maximum number of steps inside one gesture sequence. */
export const MAX_GESTURE_STEPS = 8

/** Default gesture bindings matching the Rust backend. */
export const DEFAULT_BINDINGS: GestureBinding[] = [
  {
    label: "Back",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["left"],
    },
    action: { type: "keyboard", keys: ["alt", "left"] },
  },
  {
    label: "Forward",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["right"],
    },
    action: { type: "keyboard", keys: ["alt", "right"] },
  },
  {
    label: "Scroll Up",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["up"],
    },
    action: { type: "keyboard", keys: ["pageup"] },
  },
  {
    label: "Scroll Down",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["down"],
    },
    action: { type: "keyboard", keys: ["pagedown"] },
  },
  {
    label: "Top of Page",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["down", "up"],
    },
    action: { type: "keyboard", keys: ["ctrl", "home"] },
  },
  {
    label: "Bottom of Page",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["up", "down"],
    },
    action: { type: "keyboard", keys: ["ctrl", "end"] },
  },
  {
    label: "Next Tab",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["up", "right"],
    },
    action: { type: "keyboard", keys: ["ctrl", "tab"] },
  },
  {
    label: "Previous Tab",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["up", "left"],
    },
    action: { type: "keyboard", keys: ["ctrl", "shift", "tab"] },
  },
  {
    label: "Reload",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["right", "down"],
    },
    action: { type: "keyboard", keys: ["ctrl", "r"] },
  },
  {
    label: "Close Tab",
    gesture: {
      trigger: "right_click",
      mode: "release",
      sequence: ["down", "right"],
    },
    action: { type: "keyboard", keys: ["ctrl", "w"] },
  },
]

/** Default configuration values matching the Rust backend. */
export const DEFAULTS = {
  enabled: true,
  trail_color: "#00BFFF",
  trail_thickness: 3.0,
  safety_timeout_ms: 2000,
  min_segment_px: 12,
  direction_switch_confirm_px: 8,
  axis_ambiguity_deadzone_px: 2,
  replay_distance_threshold_px: 12,
  label_font_family: "Yu Gothic UI Semibold",
  label_font_size: 36.0,
  label_font_weight: 400,
  label_padding: 24.0,
  apps: {},
  bindings: {
    default: DEFAULT_BINDINGS,
  },
} as const
