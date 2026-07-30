/** Versioned configuration document shared directly with the Rust backend. */
export interface AppConfig {
  schema_version: 2
  shared: SharedSettings
  applications: ApplicationRecord[]
  bindings: BindingRecord[]
  platforms: {
    windows: PlatformOverride
    macos: PlatformOverride
  }
}

export interface SharedSettings {
  enabled: boolean
  recognition: RecognitionSettings
  appearance: AppearanceSettings
}

export interface RecognitionSettings {
  safety_timeout_ms: number
  min_segment_px: number
  direction_switch_confirm_px: number
  axis_ambiguity_deadzone_px: number
  replay_distance_threshold_px: number
  max_gesture_steps: number
}

export interface AppearanceSettings {
  trail_color: string
  trail_thickness: number
  label_font_family: string
  label_font_size: number
  label_font_weight: number
  label_padding: number
}

export interface PlatformOverride {
  appearance?: AppearanceSettings
}

export const MATCH_TARGETS = [
  "process_name",
  "window_class",
  "title",
  "bundle_identifier",
] as const
export type MatchTarget = (typeof MATCH_TARGETS)[number]

export const MATCH_METHODS = ["exact", "contains", "regex"] as const
export type MatchMethod = (typeof MATCH_METHODS)[number]

export interface AppMatcher {
  target: MatchTarget
  method: MatchMethod
  value: string
}

export interface AppDefinition {
  id: string
  label?: string
  matchers: AppMatcher[]
}

export type WindowsMatchTarget = Exclude<MatchTarget, "bundle_identifier">
export interface WindowsAppDefinition extends Omit<AppDefinition, "matchers"> {
  matchers: Array<Omit<AppMatcher, "target"> & { target: WindowsMatchTarget }>
}

export type ApplicationRecord =
  | { platform: "shared"; application: AppDefinition }
  | { platform: "windows"; application: AppDefinition }
  | { platform: "macos"; application: AppDefinition }

export const TRIGGER_BUTTONS = [
  "left_click",
  "right_click",
  "middle_click",
] as const
export type TriggerButton = (typeof TRIGGER_BUTTONS)[number]

export const GESTURE_STEPS = [
  "up",
  "down",
  "left",
  "right",
  "wheel_up",
  "wheel_down",
  "left_click",
  "right_click",
  "middle_click",
] as const
export type GestureStep = (typeof GESTURE_STEPS)[number]

export const HOLD_STEPS = ["wheel_up", "wheel_down"] as const
export type HoldStep = (typeof HOLD_STEPS)[number]

export const GESTURE_MODES = ["release", "hold"] as const
export type GestureMode = (typeof GESTURE_MODES)[number]

interface GesturePatternBase {
  trigger: TriggerButton
  sequence: GestureStep[]
}

interface ReleaseGesturePattern extends GesturePatternBase {
  mode: "release"
  step?: never
}

interface HoldGesturePattern extends GesturePatternBase {
  mode: "hold"
  step: HoldStep
}

export type GesturePattern = ReleaseGesturePattern | HoldGesturePattern

export type Key =
  | "primary"
  | "secondary"
  | "shift"
  | "ctrl"
  | "alt"
  | "win"
  | "command"
  | "option"
  | "left"
  | "right"
  | "up"
  | "down"
  | "tab"
  | "enter"
  | "escape"
  | "backspace"
  | "delete"
  | "home"
  | "end"
  | "pageup"
  | "pagedown"
  | "space"
  | "a"
  | "b"
  | "c"
  | "d"
  | "e"
  | "f"
  | "g"
  | "h"
  | "i"
  | "j"
  | "k"
  | "l"
  | "m"
  | "n"
  | "o"
  | "p"
  | "q"
  | "r"
  | "s"
  | "t"
  | "u"
  | "v"
  | "w"
  | "x"
  | "y"
  | "z"
  | "0"
  | "1"
  | "2"
  | "3"
  | "4"
  | "5"
  | "6"
  | "7"
  | "8"
  | "9"
  | `f${1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24}`

export type Action = { type: "keyboard"; keys: Key[] }

export interface GestureBinding {
  id: string
  label?: string
  application_id?: string
  gesture: GesturePattern
  action: Action
}

export type BindingRecord =
  | { platform: "shared"; binding: GestureBinding }
  | { platform: "windows"; binding: GestureBinding }
  | { platform: "macos"; binding: GestureBinding }

export const MAX_GESTURE_STEPS = 8

const DEFAULT_BINDINGS: GestureBinding[] = [
  releaseBinding("back", "Back", ["left"], ["alt", "left"]),
  releaseBinding("forward", "Forward", ["right"], ["alt", "right"]),
  releaseBinding("scroll-up", "Scroll Up", ["up"], ["pageup"]),
  releaseBinding("scroll-down", "Scroll Down", ["down"], ["pagedown"]),
  releaseBinding(
    "top-of-page",
    "Top of Page",
    ["down", "up"],
    ["ctrl", "home"],
  ),
  releaseBinding(
    "bottom-of-page",
    "Bottom of Page",
    ["up", "down"],
    ["ctrl", "end"],
  ),
  releaseBinding("next-tab", "Next Tab", ["up", "right"], ["ctrl", "tab"]),
  releaseBinding(
    "previous-tab",
    "Previous Tab",
    ["up", "left"],
    ["ctrl", "shift", "tab"],
  ),
  releaseBinding("reload", "Reload", ["right", "down"], ["ctrl", "r"]),
  releaseBinding("close-tab", "Close Tab", ["down", "right"], ["ctrl", "w"]),
]

function releaseBinding(
  id: string,
  label: string,
  sequence: GestureStep[],
  keys: Key[],
): GestureBinding {
  return {
    id,
    label,
    gesture: { trigger: "right_click", mode: "release", sequence },
    action: { type: "keyboard", keys },
  }
}

export const DEFAULTS: AppConfig = {
  schema_version: 2,
  shared: {
    enabled: true,
    recognition: {
      safety_timeout_ms: 2000,
      min_segment_px: 12,
      direction_switch_confirm_px: 8,
      axis_ambiguity_deadzone_px: 2,
      replay_distance_threshold_px: 12,
      max_gesture_steps: 8,
    },
    appearance: {
      trail_color: "#00BFFF",
      trail_thickness: 3,
      label_font_family: "Yu Gothic UI Semibold",
      label_font_size: 36,
      label_font_weight: 400,
      label_padding: 24,
    },
  },
  applications: [],
  bindings: DEFAULT_BINDINGS.map((binding) => ({
    platform: hasWindowsKey(binding) ? "windows" : "shared",
    binding,
  })),
  platforms: { windows: {}, macos: {} },
}

function isWindowsRecord(record: ApplicationRecord | BindingRecord): boolean {
  return record.platform === "shared" || record.platform === "windows"
}

type WindowsApplicationRecord = (
  | Extract<ApplicationRecord, { platform: "shared" }>
  | Extract<ApplicationRecord, { platform: "windows" }>
) & { application: WindowsAppDefinition }

function isWindowsApplicationRecord(
  record: ApplicationRecord,
): record is WindowsApplicationRecord {
  return (
    isWindowsRecord(record) &&
    record.application.matchers.every(
      (matcher) => matcher.target !== "bundle_identifier",
    )
  )
}

function hasWindowsKey(binding: GestureBinding): boolean {
  return binding.action.keys.some((key) =>
    (["ctrl", "alt", "win"] as Key[]).includes(key),
  )
}

function applicationPlatform(
  config: AppConfig,
  applicationId: string | undefined,
): "shared" | "windows" {
  if (!applicationId) return "shared"
  const record = config.applications.find(
    (candidate) =>
      isWindowsRecord(candidate) && candidate.application.id === applicationId,
  )
  return record?.platform === "windows" ? "windows" : "shared"
}

function classifyApplication(
  application: WindowsAppDefinition,
): "shared" | "windows" {
  return application.matchers.some(
    (matcher) => matcher.target === "window_class",
  )
    ? "windows"
    : "shared"
}

function classifyBinding(
  config: AppConfig,
  binding: GestureBinding,
): "shared" | "windows" {
  return hasWindowsKey(binding) ||
    applicationPlatform(config, binding.application_id) === "windows"
    ? "windows"
    : "shared"
}

export function getWindowsApplications(
  config: AppConfig,
): WindowsAppDefinition[] {
  return config.applications
    .filter(isWindowsApplicationRecord)
    .map((record) => record.application)
}

export function getWindowsApplication(
  config: AppConfig,
  id: string,
): WindowsAppDefinition | undefined {
  return getWindowsApplications(config).find(
    (application) => application.id === id,
  )
}

export function getWindowsApplicationIds(config: AppConfig): string[] {
  return ["default", ...getWindowsApplications(config).map(({ id }) => id)]
}

export function getWindowsBindings(
  config: AppConfig,
  applicationId: string,
): GestureBinding[] {
  return config.bindings
    .filter(isWindowsRecord)
    .map((record) => record.binding)
    .filter(
      (binding) => (binding.application_id ?? "default") === applicationId,
    )
}

export function addWindowsApplication(
  config: AppConfig,
  application: WindowsAppDefinition,
): AppConfig {
  const platform = classifyApplication(application)
  return {
    ...config,
    applications: [...config.applications, { platform, application }],
  }
}

export function replaceWindowsApplication(
  config: AppConfig,
  application: WindowsAppDefinition,
): AppConfig {
  const platform = classifyApplication(application)
  const next: AppConfig = {
    ...config,
    applications: config.applications.map((record) =>
      isWindowsRecord(record) && record.application.id === application.id
        ? { platform, application }
        : record,
    ),
  }
  return {
    ...next,
    bindings: next.bindings.map((record) => {
      if (
        !isWindowsRecord(record) ||
        record.binding.application_id !== application.id
      ) {
        return record
      }
      return {
        platform: classifyBinding(next, record.binding),
        binding: record.binding,
      }
    }),
  }
}

export function removeWindowsApplication(
  config: AppConfig,
  applicationId: string,
): AppConfig {
  return {
    ...config,
    applications: config.applications.filter(
      (record) =>
        !isWindowsRecord(record) || record.application.id !== applicationId,
    ),
    bindings: config.bindings.filter(
      (record) =>
        !isWindowsRecord(record) ||
        record.binding.application_id !== applicationId,
    ),
  }
}

export function addWindowsBinding(
  config: AppConfig,
  binding: GestureBinding,
): AppConfig {
  return {
    ...config,
    bindings: [
      ...config.bindings,
      { platform: classifyBinding(config, binding), binding },
    ],
  }
}

export function replaceWindowsBinding(
  config: AppConfig,
  binding: GestureBinding,
): AppConfig {
  return {
    ...config,
    bindings: config.bindings.map((record) =>
      isWindowsRecord(record) &&
      record.binding.application_id === binding.application_id &&
      record.binding.id === binding.id
        ? { platform: classifyBinding(config, binding), binding }
        : record,
    ),
  }
}

export function removeWindowsBinding(
  config: AppConfig,
  applicationId: string,
  bindingId: string,
): AppConfig {
  const reference = applicationId === "default" ? undefined : applicationId
  return {
    ...config,
    bindings: config.bindings.filter(
      (record) =>
        !isWindowsRecord(record) ||
        record.binding.application_id !== reference ||
        record.binding.id !== bindingId,
    ),
  }
}
