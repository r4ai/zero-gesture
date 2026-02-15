import {
  useMutation,
  useQueryClient,
  useSuspenseQuery,
} from "@tanstack/react-query"
import { listen } from "@tauri-apps/api/event"
import { useEffect } from "react"
import { getConfig, updateConfig } from "@/lib/api"
import type { AppConfig } from "@/types/config"

export const CONFIG_QUERY_KEY = ["config"] as const

/** config-updated イベントを購読し、キャッシュを自動更新するフック */
export function useConfigUpdatedListener() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const unlisten = listen<AppConfig>("config-updated", (event) => {
      queryClient.setQueryData(CONFIG_QUERY_KEY, event.payload)
    })
    return () => {
      unlisten.then((fn) => fn())
    }
  }, [queryClient])
}

/** 現在の config を取得する（Suspense 対応） */
export function useConfig() {
  return useSuspenseQuery({
    queryKey: CONFIG_QUERY_KEY,
    queryFn: getConfig,
    staleTime: Number.POSITIVE_INFINITY, // イベント駆動で更新するため自動再取得しない
  })
}

/** config を更新する */
export function useUpdateConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: updateConfig,
    // 成功時はサーバーから config-updated イベントが来るので楽観的更新は不要
    // エラー時も既存キャッシュはそのまま維持
    onError: () => {
      queryClient.invalidateQueries({ queryKey: CONFIG_QUERY_KEY })
    },
  })
}
