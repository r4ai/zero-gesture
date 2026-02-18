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

export function ConfigDraftContextProvider({
  children,
  value,
}: {
  children: ReactNode
  value: ConfigDraftContext
}) {
  return (
    <ConfigDraftContext.Provider value={value}>
      {children}
    </ConfigDraftContext.Provider>
  )
}

/** Must be placed inside Suspense because it uses useConfig() */
export function ConfigDraftProvider({ children }: { children: ReactNode }) {
  const { data: config } = useConfig()
  const { mutate: updateConfig, isPending } = useUpdateConfig()

  const [draft, setDraft] = useState<AppConfig>(config)

  // Sync draft when server-side config changes due to config-updated events, etc.
  useEffect(() => {
    // TODO: check if draft is dirty and ask user if they want to discard changes
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
