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

export const getConfig = (): Promise<AppConfig> => invoke("get_config")

export const updateConfig = (newConfig: AppConfig): Promise<void> =>
  invoke("update_config", { newConfig })

export const importConfig = (filePath: string): Promise<void> =>
  invoke("import_config", { filePath })

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

/**
 * Starts a one-shot window capture via a global mouse hook.
 *
 * The backend installs `WH_MOUSE_LL` and waits for the user to click.
 * When clicked, a `window-captured` Tauri event is emitted carrying
 * {@link ForegroundWindowInfo} for the window under the cursor.
 *
 * Use {@link stopWindowCapture} to cancel before the user clicks.
 */
export const startWindowCapture = (): Promise<void> =>
  invoke("start_window_capture")

/**
 * Cancels an in-progress window capture started by {@link startWindowCapture}.
 * No-op if no capture is active.
 */
export const stopWindowCapture = (): Promise<void> =>
  invoke("stop_window_capture")
