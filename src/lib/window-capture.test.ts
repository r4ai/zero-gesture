import { describe, expect, it } from "vitest"
import {
  acceptedCurrentWindowCapture,
  acceptedWindowCapture,
} from "@/lib/window-capture"

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

  it("rejects an old captured poll that resolves after cleanup or replacement", () => {
    const requested = { capture_id: 5, epoch: 9 }
    const captured = {
      ...requested,
      state: "captured" as const,
      info,
    }

    expect(
      acceptedCurrentWindowCapture(false, requested, requested, captured),
    ).toBeNull()
    expect(
      acceptedCurrentWindowCapture(
        true,
        { capture_id: 6, epoch: 10 },
        requested,
        captured,
      ),
    ).toBeNull()
  })
})
