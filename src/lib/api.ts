import { invoke } from "@tauri-apps/api/core"
import type { AppConfig } from "@/types/config"

export const getConfig = (): Promise<AppConfig> => invoke("get_config")

export const updateConfig = (newConfig: AppConfig): Promise<void> =>
  invoke("update_config", { newConfig })
