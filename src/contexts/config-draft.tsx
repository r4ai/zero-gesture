import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useState,
} from "react"
import { configMutation, useConfig, useUpdateConfig } from "@/hooks/use-config"
import type * as api from "@/lib/api"
import { settingsErrorMessage } from "@/lib/settings-error"
import { type AppConfig, DEFAULTS } from "@/types/config"

interface ConfigDraftContext {
  draft: AppConfig
  setDraft: (config: AppConfig) => void
  isDirty: boolean
  reset: () => void
  save: () => void
  adoptApplied: (observed: api.ConfigObservation) => void
  isSaving: boolean
  saveError: string | null
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
  const { data: observed } = useConfig()
  const {
    mutate: updateConfig,
    isPending,
    error,
    reset: resetMutation,
  } = useUpdateConfig()
  const config = observed.config ?? DEFAULTS

  const [state, setState] = useState<DraftState>({
    base: config,
    draft: config,
    revision: observed.revision,
  })

  useEffect(() => {
    setState((current) =>
      advanceDraftObservation(current, observed, "conflict"),
    )
  }, [observed])

  const isDirty = observed.config === null || state.draft !== state.base

  const setDraft = (draft: AppConfig) => {
    resetMutation()
    setState((current) => ({ ...current, draft }))
  }
  const reset = () => {
    resetMutation()
    setState((current) => ({ ...current, draft: current.base }))
  }
  const save = () =>
    updateConfig(
      configMutation({ ...observed, revision: state.revision }, state.draft),
      {
        onSuccess: (result) => {
          setState((current) =>
            advanceDraftObservation(current, result.current, "applied"),
          )
        },
      },
    )
  const adoptApplied = (current: api.ConfigObservation) =>
    setState((state) => advanceDraftObservation(state, current, "applied"))

  return (
    <ConfigDraftContext.Provider
      value={{
        draft: state.draft,
        setDraft,
        isDirty,
        reset,
        save,
        adoptApplied,
        isSaving: isPending,
        saveError: error ? settingsErrorMessage(error, "Save") : null,
      }}
    >
      {children}
    </ConfigDraftContext.Provider>
  )
}

type DraftState = {
  base: AppConfig
  draft: AppConfig
  revision: number
}

export function advanceDraftObservation(
  state: DraftState,
  observed: api.ConfigObservation,
  cause: "conflict" | "applied" = "conflict",
): DraftState {
  if (state.revision === observed.revision) return state
  const base = observed.config ?? DEFAULTS
  return {
    base,
    draft:
      cause === "applied" || state.draft === state.base ? base : state.draft,
    revision: observed.revision,
  }
}

export function useConfigDraft() {
  const ctx = useContext(ConfigDraftContext)
  if (!ctx)
    throw new Error("useConfigDraft must be used within ConfigDraftProvider")
  return ctx
}
