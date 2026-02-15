import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useState,
} from "react"
import { useConfig, useUpdateConfig } from "@/hooks/use-config"
import type { AppConfig } from "@/types/config"

interface ConfigDraftContext {
  draft: AppConfig
  setDraft: (config: AppConfig) => void
  isDirty: boolean
  reset: () => void
  save: () => void
  isSaving: boolean
}

const ConfigDraftContext = createContext<ConfigDraftContext | null>(null)

/** useConfig() を使うため Suspense の内側に置くこと */
export function ConfigDraftProvider({ children }: { children: ReactNode }) {
  const { data: config } = useConfig()
  const { mutate: updateConfig, isPending } = useUpdateConfig()

  const [draft, setDraft] = useState<AppConfig>(config)

  // config-updated イベント等でサーバー側の config が変わったら draft を同期する
  useEffect(() => {
    setDraft(config)
  }, [config])

  const isDirty = draft !== config

  const reset = () => setDraft(config)
  const save = () => updateConfig(draft)

  return (
    <ConfigDraftContext.Provider
      value={{ draft, setDraft, isDirty, reset, save, isSaving: isPending }}
    >
      {children}
    </ConfigDraftContext.Provider>
  )
}

export function useConfigDraft() {
  const ctx = useContext(ConfigDraftContext)
  if (!ctx)
    throw new Error("useConfigDraft must be used within ConfigDraftProvider")
  return ctx
}
