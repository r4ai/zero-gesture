import { QueryClient } from "@tanstack/react-query"
import { describe, expect, it } from "vitest"
import {
  CONFIG_QUERY_KEY,
  cacheAppliedConfig,
  cacheConfigConflict,
  configMutation,
  durabilityWarningMessage,
} from "@/hooks/use-config"
import { DEFAULTS } from "@/types/config"

describe("config cache", () => {
  it("replaces the observed config and revision after Applied", () => {
    const queryClient = new QueryClient()
    queryClient.setQueryData(CONFIG_QUERY_KEY, {
      revision: 7,
      generation: 7,
      config: DEFAULTS,
    })
    const changed = {
      ...DEFAULTS,
      shared: {
        ...DEFAULTS.shared,
        enabled: !DEFAULTS.shared.enabled,
      },
    }

    const result = {
      current: {
        revision: 8,
        generation: 8,
        config: changed,
      },
      durability_warning: true,
    }
    cacheAppliedConfig(queryClient, result)

    const current = queryClient.getQueryData(CONFIG_QUERY_KEY)
    expect(current).toEqual({
      revision: 8,
      generation: 8,
      config: changed,
    })
    expect(configMutation(current as typeof result.current, changed)).toEqual({
      config: changed,
      expectedRevision: 8,
    })
    expect(durabilityWarningMessage(result)).toContain(
      "could not confirm directory metadata durability",
    )
  })

  it("refreshes the observed revision from a typed conflict payload", () => {
    const queryClient = new QueryClient()
    const current = {
      revision: 12,
      generation: 12,
      config: DEFAULTS,
    }

    expect(
      cacheConfigConflict(queryClient, {
        code: "revision-conflict",
        message: "detail",
        retryable: true,
        current,
      }),
    ).toBe(true)
    expect(queryClient.getQueryData(CONFIG_QUERY_KEY)).toBe(current)
    expect(cacheConfigConflict(queryClient, "revision conflict")).toBe(false)
  })
})
