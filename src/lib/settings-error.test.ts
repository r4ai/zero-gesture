import { describe, expect, it } from "vitest"
import {
  configConflictObservation,
  settingsErrorMessage,
} from "@/lib/settings-error"
import { DEFAULTS } from "@/types/config"

describe("Settings command errors", () => {
  it("uses the typed revision conflict payload without parsing its message", () => {
    const current = {
      revision: 8,
      generation: 8,
      config: DEFAULTS,
    }
    const error = {
      code: "revision-conflict",
      message: "unstructured text that may change",
      retryable: true,
      current,
    }

    expect(configConflictObservation(error)).toBe(current)
    expect(settingsErrorMessage(error, "Save")).toContain(
      "newer settings at revision 8",
    )
  })

  it.each([
    ["engine-unavailable", "Start or restart Zero Gesture"],
    ["engine-disconnected", "Reconnect"],
    ["validation-failed", "Review the highlighted values"],
    ["request-rejected", "Review the input"],
    ["filesystem-failed", "Check the file path and permissions"],
    ["platform-failed", "Try again"],
    ["backend-failed", "Try again"],
  ] as const)("renders actionable %s guidance", (code, guidance) => {
    expect(
      settingsErrorMessage(
        { code, message: "backend detail", retryable: true },
        "Import",
      ),
    ).toContain(guidance)
  })

  it("rejects string and malformed objects instead of classifying their text", () => {
    expect(configConflictObservation("revision-conflict")).toBeNull()
    expect(settingsErrorMessage("Engine is unavailable", "Save")).toBe(
      "Save failed. Try again.",
    )
  })
})
