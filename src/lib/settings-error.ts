import type { ConfigObservation } from "@/lib/api"

export const SETTINGS_ERROR_CODES = [
  "revision-conflict",
  "engine-unavailable",
  "engine-disconnected",
  "validation-failed",
  "request-rejected",
  "filesystem-failed",
  "platform-failed",
  "backend-failed",
  "capture-stale",
  "capture-unavailable",
] as const

export type SettingsErrorCode = (typeof SETTINGS_ERROR_CODES)[number]

export type SettingsCommandError = {
  code: SettingsErrorCode
  message: string
  retryable: boolean
  current?: ConfigObservation
}

function settingsCommandError(value: unknown): SettingsCommandError | null {
  if (!value || typeof value !== "object") return null
  const candidate = value as Record<string, unknown>
  if (
    typeof candidate.code !== "string" ||
    !SETTINGS_ERROR_CODES.includes(candidate.code as SettingsErrorCode) ||
    typeof candidate.message !== "string" ||
    typeof candidate.retryable !== "boolean"
  ) {
    return null
  }
  return candidate as SettingsCommandError
}

export function configConflictObservation(
  value: unknown,
): ConfigObservation | null {
  const error = settingsCommandError(value)
  if (error?.code !== "revision-conflict" || !error.current) return null
  return error.current
}

export function settingsErrorMessage(value: unknown, action: string): string {
  const error = settingsCommandError(value)
  if (!error) return `${action} failed. Try again.`

  switch (error.code) {
    case "revision-conflict":
      return error.current
        ? `${action} was not applied because newer settings at revision ${error.current.revision} are available. Your draft was kept; review it and retry.`
        : `${action} was not applied because newer settings are available. Your draft was kept; reload and retry.`
    case "engine-unavailable":
      return `${action} failed because the Engine is unavailable. Start or restart Zero Gesture, then retry.`
    case "engine-disconnected":
      return `${action} failed because the Engine disconnected. Reconnect by reopening Settings, then retry.`
    case "validation-failed":
      return `${action} was rejected. Review the highlighted values and retry.`
    case "request-rejected":
      return `${action} was rejected by the Engine. Review the input and retry.`
    case "filesystem-failed":
      return `${action} could not access the file. Check the file path and permissions, then retry.`
    case "platform-failed":
      return `${action} could not be completed by Windows. Try again.`
    case "backend-failed":
      return `${action} failed inside the Engine. Try again; restart Zero Gesture if it continues.`
    case "capture-stale":
      return "That window capture is no longer active. Start a new capture."
    case "capture-unavailable":
      return "Window capture is unavailable. Try again; restart Zero Gesture if it continues."
  }
}
