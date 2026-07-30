import {
  type QueryClient,
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { isTauri } from "@tauri-apps/api/core"
import { toast } from "sonner"
import * as api from "@/lib/api"
import { type AppConfig, DEFAULTS } from "@/types/config"

export const CONFIG_QUERY_KEY = ["config"] as const
const DURABILITY_WARNING =
  "Config was applied, but Windows could not confirm directory metadata durability."

export function cacheAppliedConfig(
  queryClient: QueryClient,
  result: api.ConfigApplyResult,
) {
  queryClient.setQueryData(CONFIG_QUERY_KEY, result.current)
}

export function configMutation(
  observed: api.ConfigObservation,
  config: AppConfig,
) {
  return { config, expectedRevision: observed.revision }
}

export function durabilityWarningMessage(result: api.ConfigApplyResult) {
  return result.durability_warning ? DURABILITY_WARNING : null
}

const getConfig = () => {
  if (isTauri()) return api.getConfig()
  if (import.meta.env.DEV)
    return Promise.resolve({
      revision: 1,
      generation: 1,
      config: DEFAULTS,
    })
  throw new Error("getConfig is only available in Tauri environment")
}

type ConfigMutation = {
  config: AppConfig
  expectedRevision: number
}

const updateConfig = ({ config, expectedRevision }: ConfigMutation) => {
  if (isTauri()) return api.updateConfig(config, expectedRevision)
  if (import.meta.env.DEV)
    return Promise.resolve({
      current: {
        revision: expectedRevision + 1,
        generation: expectedRevision + 1,
        config,
      },
      durability_warning: false,
    })
  throw new Error("updateConfig is only available in Tauri environment")
}

type ImportMutation = {
  filePath: string
  expectedRevision: number
}

const importConfig = ({ filePath, expectedRevision }: ImportMutation) => {
  if (isTauri()) return api.importConfig(filePath, expectedRevision)
  throw new Error("importConfig is only available in Tauri environment")
}

/** Get the current config (supports Suspense) */
export function useConfig() {
  return useSuspenseQuery({
    queryKey: CONFIG_QUERY_KEY,
    queryFn: getConfig,
    staleTime: Number.POSITIVE_INFINITY,
  })
}

function appliedCallbacks(queryClient: QueryClient) {
  return {
    onSuccess: (result: api.ConfigApplyResult) => {
      cacheAppliedConfig(queryClient, result)
      const warning = durabilityWarningMessage(result)
      if (warning) {
        toast.warning(warning)
      }
    },
    onError: () => {
      queryClient.invalidateQueries({ queryKey: CONFIG_QUERY_KEY })
    },
  }
}

export function useUpdateConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: updateConfig,
    ...appliedCallbacks(queryClient),
  })
}

export function useImportConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: importConfig,
    ...appliedCallbacks(queryClient),
  })
}
