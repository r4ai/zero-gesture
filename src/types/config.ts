export type MatchTarget = "process_name" | "window_class" | "title"
export type MatchMethod = "exact" | "contains" | "regex"

export interface AppMatcher {
  target: MatchTarget
  method: MatchMethod
  value: string
}

export interface AppDefinition {
  matchers: AppMatcher[]
}

export interface GestureBinding {
  label?: string
  // Action fields flattened (serde flatten)
  type: string
  keys?: string[]
  button?: string
  command?: string
}

export interface AppConfig {
  enabled: boolean
  gesture_trigger_button: string
  trail_color: string
  trail_thickness: number
  gesture_threshold: number
  safety_timeout_ms: number
  min_segment_px: number
  direction_switch_confirm_px: number
  axis_ambiguity_deadzone_px: number
  label_font_family: string
  label_font_size: number
  label_font_weight: number
  label_padding: number
  apps: Record<string, AppDefinition>
  bindings: Record<string, Record<string, GestureBinding>>
}
