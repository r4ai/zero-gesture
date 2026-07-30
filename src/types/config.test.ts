import { describe, expect, it } from "vitest"
import {
  type AppConfig,
  addWindowsBinding,
  getWindowsApplication,
  getWindowsApplications,
  getWindowsBindings,
  removeWindowsApplication,
  replaceWindowsApplication,
  replaceWindowsBinding,
} from "./config"

function document(): AppConfig {
  return {
    schema_version: 2,
    shared: {
      enabled: true,
      recognition: {
        safety_timeout_ms: 2000,
        min_segment_px: 12,
        direction_switch_confirm_px: 8,
        axis_ambiguity_deadzone_px: 2,
        replay_distance_threshold_px: 12,
        max_gesture_steps: 8,
      },
      appearance: {
        trail_color: "#fff",
        trail_thickness: 3,
        label_font_family: "Shared",
        label_font_size: 36,
        label_font_weight: 400,
        label_padding: 24,
      },
    },
    applications: [
      {
        platform: "shared",
        application: {
          id: "browser",
          matchers: [{ target: "title", method: "exact", value: "Browser" }],
        },
      },
      {
        platform: "macos",
        application: {
          id: "mac",
          matchers: [
            {
              target: "bundle_identifier",
              method: "exact",
              value: "dev.example.mac",
            },
          ],
        },
      },
    ],
    bindings: [
      {
        platform: "shared",
        binding: {
          id: "first",
          application_id: "browser",
          gesture: {
            trigger: "right_click",
            mode: "release",
            sequence: ["left"],
          },
          action: { type: "keyboard", keys: ["primary", "left"] },
        },
      },
      {
        platform: "macos",
        binding: {
          id: "mac-only",
          application_id: "mac",
          gesture: {
            trigger: "right_click",
            mode: "release",
            sequence: ["right"],
          },
          action: { type: "keyboard", keys: ["command", "r"] },
        },
      },
    ],
    platforms: {
      windows: {},
      macos: {
        appearance: {
          trail_color: "#000",
          trail_thickness: 4,
          label_font_family: "Mac",
          label_font_size: 24,
          label_font_weight: 500,
          label_padding: 12,
        },
      },
    },
  }
}

describe("Windows config editing", () => {
  it("preserves IDs, order, references, and untouched macOS records", () => {
    const original = document()
    const macApplication = original.applications[1]
    const macBinding = original.bindings[1]
    const macOverride = original.platforms.macos

    const added = addWindowsBinding(original, {
      id: "second",
      application_id: "browser",
      gesture: {
        trigger: "right_click",
        mode: "release",
        sequence: ["right"],
      },
      action: { type: "keyboard", keys: ["primary", "r"] },
    })
    const edited = replaceWindowsBinding(added, {
      ...getWindowsBindings(added, "browser")[0],
      label: "Edited",
    })

    expect(getWindowsBindings(edited, "browser").map(({ id }) => id)).toEqual([
      "first",
      "second",
    ])
    expect(getWindowsBindings(edited, "browser")[0].application_id).toBe(
      "browser",
    )
    expect(edited.applications[1]).toBe(macApplication)
    expect(edited.bindings[1]).toBe(macBinding)
    expect(edited.platforms.macos).toBe(macOverride)
    expect(getWindowsApplication(edited, "mac")).toBeUndefined()
    expect(
      getWindowsApplications(edited).flatMap((application) =>
        application.matchers.map((matcher) => matcher.target),
      ),
    ).not.toContain("bundle_identifier")
  })

  it("reclassifies a whole record when a Windows matcher or key is introduced", () => {
    const original = document()
    const application = original.applications[0].application
    const withWindowsMatcher = replaceWindowsApplication(original, {
      ...application,
      matchers: [
        { target: "window_class", method: "exact", value: "BrowserClass" },
      ],
    })
    expect(withWindowsMatcher.applications[0].platform).toBe("windows")
    expect(withWindowsMatcher.bindings[0].platform).toBe("windows")

    const withPhysicalKey = replaceWindowsBinding(original, {
      ...original.bindings[0].binding,
      action: { type: "keyboard", keys: ["ctrl", "r"] },
    })
    expect(withPhysicalKey.bindings[0].platform).toBe("windows")
  })

  it("deletes only the selected Windows application and its bindings", () => {
    const original = document()
    const edited = removeWindowsApplication(original, "browser")
    expect(edited.applications.map((record) => record.application.id)).toEqual([
      "mac",
    ])
    expect(edited.bindings.map((record) => record.binding.id)).toEqual([
      "mac-only",
    ])
  })
})
