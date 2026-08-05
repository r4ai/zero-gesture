import { invoke } from "@tauri-apps/api/core"
import type { AppConfig } from "@/types/config"

/** Information about the current foreground window. */
export type ForegroundWindowInfo = {
  /** Executable file name (e.g., "chrome.exe"), lowercased. `null` if unavailable. */
  process_name: string | null
  /** Win32 window class name (e.g., "CabinetWClass"). `null` if unavailable. */
  window_class: string | null
  /** Window title text. `null` if unavailable. */
  title: string | null
}

export type ConfigObservation = {
  revision: number
  generation: number
  config: AppConfig | null
}

export type ConfigApplyResult = {
  current: ConfigObservation
  durability_warning: boolean
}

export type WindowCaptureToken = {
  capture_id: number
  epoch: number
}

type WindowCaptureBackendObservation =
  | { state: "pending" }
  | { state: "captured"; info: ForegroundWindowInfo }

export type WindowCapturePoll = WindowCaptureToken &
  WindowCaptureBackendObservation

let nextCaptureId = 1

export const getConfig = (): Promise<ConfigObservation> => invoke("get_config")

export const updateConfig = (
  newConfig: AppConfig,
  expectedRevision: number,
): Promise<ConfigApplyResult> =>
  invoke("update_config", { newConfig, expectedRevision })

export const setEnabled = (
  enabled: boolean,
  expectedRevision: number,
): Promise<ConfigApplyResult> =>
  invoke("set_enabled", { enabled, expectedRevision })

export const importConfig = (
  filePath: string,
  expectedRevision: number,
): Promise<ConfigApplyResult> =>
  invoke("import_config", { filePath, expectedRevision })

export const exportConfig = (filePath: string): Promise<void> =>
  invoke("export_config", { filePath })

export const openConfigDir = (): Promise<void> => invoke("open_config_dir")

/**
 * Retrieves information about the current foreground window.
 *
 * @returns Process name, window class, and title of the foreground window.
 *   Each field may be `null` if unavailable.
 */
export const getForegroundWindowInfo = (): Promise<ForegroundWindowInfo> =>
  invoke("get_foreground_window_info")

export const startWindowCapture = (): Promise<WindowCaptureToken> => {
  const captureId = nextCaptureId
  nextCaptureId =
    nextCaptureId === Number.MAX_SAFE_INTEGER ? 1 : nextCaptureId + 1
  return invoke("start_window_capture", { captureId })
}

export const pollWindowCapture = async (
  token: WindowCaptureToken,
): Promise<WindowCapturePoll> => {
  const observation = await invoke<WindowCaptureBackendObservation>(
    "poll_window_capture",
    {
      captureId: token.capture_id,
      epoch: token.epoch,
    },
  )
  return { ...token, ...observation }
}

export const stopWindowCapture = (token: WindowCaptureToken): Promise<void> =>
  invoke("stop_window_capture", {
    captureId: token.capture_id,
    epoch: token.epoch,
  })
