import { describe, expect, it } from "vitest"
import { acceptedWindowCapture } from "@/lib/window-capture"

describe("window capture UI identity", () => {
  const info = {
    process_name: "explorer.exe",
    window_class: "CabinetWClass",
    title: "Downloads",
  }

  it("accepts only the active capture id and epoch", () => {
    const active = { capture_id: 5, epoch: 9 }
    expect(
      acceptedWindowCapture(active, {
        capture_id: 5,
        epoch: 9,
        state: "captured",
        info,
      }),
    ).toEqual(info)
    expect(
      acceptedWindowCapture(active, {
        capture_id: 4,
        epoch: 8,
        state: "captured",
        info,
      }),
    ).toBeNull()
  })

  it("does not apply pending or cancelled results", () => {
    const active = { capture_id: 5, epoch: 9 }
    expect(
      acceptedWindowCapture(active, {
        capture_id: 5,
        epoch: 9,
        state: "pending",
      }),
    ).toBeNull()
  })
})
