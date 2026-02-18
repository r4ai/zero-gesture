import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { isTauri } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { useEffect } from "react"
import * as api from "@/lib/api"
import { type AppConfig, DEFAULTS } from "@/types/config"

export const CONFIG_QUERY_KEY = ["config"] as const

/** Hook that subscribes to the "config-updated" event and automatically updates the cache */
export function useConfigUpdatedListener() {
  const queryClient = useQueryClient()
  useEffect(() => {
    if (isTauri()) {
      const unlisten = listen<AppConfig>("config-updated", (event) => {
        queryClient.setQueryData(CONFIG_QUERY_KEY, event.payload)
      })
      return () => {
        unlisten.then((fn) => fn())
      }
    }
    if (import.meta.env.DEV) return
    console.error(
      "useConfigUpdatedListener is only available in Tauri environment",
    )
  }, [queryClient])
}

const getConfig = () => {
  if (isTauri()) return api.getConfig()
  if (import.meta.env.DEV) return DEFAULTS // mock config for development
  throw new Error("getConfig is only available in Tauri environment")
}

const updateConfig = (newConfig: AppConfig) => {
  if (isTauri()) return api.updateConfig(newConfig)
  if (import.meta.env.DEV) return Promise.resolve() // mock update for development
  throw new Error("updateConfig is only available in Tauri environment")
}

/** Get the current config (supports Suspense) */
export function useConfig() {
  return useSuspenseQuery({
    queryKey: CONFIG_QUERY_KEY,
    queryFn: getConfig,
    staleTime: Number.POSITIVE_INFINITY, // Do not refetch automatically; updates are event-driven
  })
}

/** Update config */
export function useUpdateConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: updateConfig,
    // On success, a "config-updated" event will be sent from the server so optimistic updates are unnecessary
    // On error, keep the existing cache unchanged
    onError: () => {
      queryClient.invalidateQueries({ queryKey: CONFIG_QUERY_KEY })
    },
  })
}
