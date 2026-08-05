import { describe, expect, it } from "vitest"
import { advanceDraftObservation } from "@/contexts/config-draft"
import { DEFAULTS } from "@/types/config"

describe("config draft observation", () => {
  it("keeps a dirty draft while advancing the retry revision after conflict", () => {
    const base = DEFAULTS
    const draft = {
      ...base,
      shared: { ...base.shared, enabled: !base.shared.enabled },
    }
    const current = {
      revision: 4,
      generation: 4,
      config: {
        ...base,
        shared: { ...base.shared, locale: "ja-JP" },
      },
    }

    expect(
      advanceDraftObservation({ base, draft, revision: 3 }, current),
    ).toEqual({
      base: current.config,
      draft,
      revision: 4,
    })
  })

  it("adopts a new Engine observation when the draft is clean", () => {
    const current = {
      revision: 6,
      generation: 6,
      config: {
        ...DEFAULTS,
        shared: { ...DEFAULTS.shared, enabled: true },
      },
    }

    expect(
      advanceDraftObservation(
        { base: DEFAULTS, draft: DEFAULTS, revision: 5 },
        current,
      ),
    ).toEqual({
      base: current.config,
      draft: current.config,
      revision: 6,
    })
  })
})
