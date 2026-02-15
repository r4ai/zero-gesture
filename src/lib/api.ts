import { invoke } from "@tauri-apps/api/core"
import type { AppConfig } from "@/types/config"

export const getConfig = (): Promise<AppConfig> => invoke("get_config")

export const updateConfig = (newConfig: AppConfig): Promise<void> =>
  invoke("update_config", { newConfig })

export const importConfig = (filePath: string): Promise<void> =>
  invoke("import_config", { filePath })

export const exportConfig = (filePath: string): Promise<void> =>
  invoke("export_config", { filePath })

export const openConfigDir = (): Promise<void> => invoke("open_config_dir")
