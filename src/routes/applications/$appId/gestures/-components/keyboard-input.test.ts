import { describe, expect, it } from "vitest"
import {
  keyLabel,
  MODIFIER_KEYS,
  modifierLabel,
  normalizePressedKey,
  parseKeySequence,
  parseKeys,
  SHORTCUT_KEYS,
} from "./keyboard-input"

describe("keyboard-input", () => {
  describe("SHORTCUT_KEYS", () => {
    it("should contain all letters a-z", () => {
      const letters = SHORTCUT_KEYS.filter((k) => /^[a-z]$/.test(k))
      expect(letters).toHaveLength(26)
      expect(letters).toContain("a")
      expect(letters).toContain("z")
    })

    it("should contain all numbers 0-9", () => {
      const numbers = SHORTCUT_KEYS.filter((k) => /^\d$/.test(k))
      expect(numbers).toHaveLength(10)
      expect(numbers).toContain("0")
      expect(numbers).toContain("9")
    })

    it("should contain all function keys f1-f24", () => {
      const functionKeys = SHORTCUT_KEYS.filter((k) => /^f\d+$/.test(k))
      expect(functionKeys).toHaveLength(24)
      expect(functionKeys).toContain("f1")
      expect(functionKeys).toContain("f12")
      expect(functionKeys).toContain("f24")
    })

    it("should contain all navigation keys", () => {
      const navigationKeys = [
        "left",
        "right",
        "up",
        "down",
        "tab",
        "enter",
        "escape",
        "backspace",
        "delete",
        "home",
        "end",
        "pageup",
        "pagedown",
      ]
      for (const key of navigationKeys) {
        expect(SHORTCUT_KEYS).toContain(key)
      }
    })

    it("should contain space", () => {
      expect(SHORTCUT_KEYS).toContain("space")
    })

    it("should not contain uppercase letters", () => {
      const uppercaseKeys = SHORTCUT_KEYS.filter((k) => /^[A-Z]$/.test(k))
      expect(uppercaseKeys).toHaveLength(0)
    })
  })

  describe("MODIFIER_KEYS", () => {
    it("should contain all modifier keys in lowercase", () => {
      expect(MODIFIER_KEYS).toContain("ctrl")
      expect(MODIFIER_KEYS).toContain("alt")
      expect(MODIFIER_KEYS).toContain("shift")
      expect(MODIFIER_KEYS).toContain("win")
    })

    it("should have exactly 4 modifier keys", () => {
      expect(MODIFIER_KEYS).toHaveLength(4)
    })
  })

  describe("parseKeys", () => {
    it("should return empty array for undefined input", () => {
      expect(parseKeys(undefined)).toEqual([])
    })

    it("should return empty array for empty string", () => {
      expect(parseKeys("")).toEqual([])
    })

    it("should parse modifier keys with various aliases", () => {
      expect(parseKeys("ctrl")).toEqual(["ctrl"])
      expect(parseKeys("Ctrl")).toEqual(["ctrl"])
      expect(parseKeys("CTRL")).toEqual(["ctrl"])
      expect(parseKeys("control")).toEqual(["ctrl"])

      expect(parseKeys("alt")).toEqual(["alt"])
      expect(parseKeys("Alt")).toEqual(["alt"])
      expect(parseKeys("menu")).toEqual(["alt"])
      expect(parseKeys("option")).toEqual(["alt"])

      expect(parseKeys("shift")).toEqual(["shift"])
      expect(parseKeys("Shift")).toEqual(["shift"])

      expect(parseKeys("win")).toEqual(["win"])
      expect(parseKeys("Win")).toEqual(["win"])
      expect(parseKeys("meta")).toEqual(["win"])
      expect(parseKeys("command")).toEqual(["win"])
      expect(parseKeys("cmd")).toEqual(["win"])
      expect(parseKeys("windows")).toEqual(["win"])
      expect(parseKeys("lwin")).toEqual(["win"])
      expect(parseKeys("super")).toEqual(["win"])
    })

    it("should normalize letters to lowercase", () => {
      expect(parseKeys("a")).toEqual(["a"])
      expect(parseKeys("A")).toEqual(["a"])
      expect(parseKeys("z")).toEqual(["z"])
      expect(parseKeys("Z")).toEqual(["z"])
    })

    it("should keep numbers as-is", () => {
      expect(parseKeys("0")).toEqual(["0"])
      expect(parseKeys("9")).toEqual(["9"])
    })

    it("should normalize function keys to lowercase", () => {
      expect(parseKeys("f1")).toEqual(["f1"])
      expect(parseKeys("F1")).toEqual(["f1"])
      expect(parseKeys("f12")).toEqual(["f12"])
      expect(parseKeys("F24")).toEqual(["f24"])
    })

    it("should reject invalid function keys", () => {
      expect(parseKeys("f0")).toEqual([])
      expect(parseKeys("f25")).toEqual([])
      expect(parseKeys("f100")).toEqual([])
      expect(parseKeys("fx")).toEqual([])
    })

    it("should handle navigation key aliases", () => {
      expect(parseKeys("return")).toEqual(["enter"])
      expect(parseKeys("esc")).toEqual(["escape"])
      expect(parseKeys("del")).toEqual(["delete"])
      expect(parseKeys("pgup")).toEqual(["pageup"])
      expect(parseKeys("pgdn")).toEqual(["pagedown"])
    })

    it("should handle space", () => {
      expect(parseKeys("space")).toEqual(["space"])
    })

    it("should parse comma-separated keys", () => {
      expect(parseKeys("ctrl,a")).toEqual(["ctrl", "a"])
      expect(parseKeys("ctrl,alt,delete")).toEqual(["ctrl", "alt", "delete"])
    })

    it("should parse plus-separated keys", () => {
      expect(parseKeys("ctrl+shift+t")).toEqual(["ctrl", "shift", "t"])
      expect(parseKeys("Ctrl + Alt + F1")).toEqual(["ctrl", "alt", "f1"])
    })

    it("should parse whitespace-separated keys", () => {
      expect(parseKeys("ctrl shift t")).toEqual(["ctrl", "shift", "t"])
    })

    it("should handle spaces around commas", () => {
      expect(parseKeys("ctrl, a, b")).toEqual(["ctrl", "a", "b"])
      expect(parseKeys("  ctrl  ,  alt  ")).toEqual(["ctrl", "alt"])
    })

    it("should filter out unknown keys", () => {
      expect(parseKeys("unknown")).toEqual([])
      expect(parseKeys("ctrl,unknown,a")).toEqual(["ctrl", "a"])
    })

    it("should filter out invalid single characters", () => {
      expect(parseKeys("@")).toEqual([])
      expect(parseKeys("#")).toEqual([])
      expect(parseKeys("!")).toEqual([])
    })

    it("should handle complex combinations", () => {
      expect(parseKeys("Ctrl,Alt,F1")).toEqual(["ctrl", "alt", "f1"])
      expect(parseKeys("CTRL+SHIFT+T")).toEqual(["ctrl", "shift", "t"])
    })
  })

  describe("parseKeySequence", () => {
    it("should return empty array for undefined input", () => {
      expect(parseKeySequence(undefined)).toEqual([])
    })

    it("should parse comma-separated combos", () => {
      expect(parseKeySequence("f21+a, ctrl+x, shift+z")).toEqual([
        ["f21", "a"],
        ["ctrl", "x"],
        ["shift", "z"],
      ])
    })

    it("should ignore empty combos", () => {
      expect(parseKeySequence("ctrl+x, , shift+z")).toEqual([
        ["ctrl", "x"],
        ["shift", "z"],
      ])
    })
  })

  describe("normalizePressedKey", () => {
    it("should return null for bare modifier keys", () => {
      expect(normalizePressedKey("Control")).toBeNull()
      expect(normalizePressedKey("Alt")).toBeNull()
      expect(normalizePressedKey("Shift")).toBeNull()
      expect(normalizePressedKey("Meta")).toBeNull()
    })

    it("should normalize space", () => {
      expect(normalizePressedKey(" ")).toBe("space")
    })

    it("should normalize arrow keys to navigation names", () => {
      expect(normalizePressedKey("ArrowUp")).toBe("up")
      expect(normalizePressedKey("ArrowDown")).toBe("down")
      expect(normalizePressedKey("ArrowLeft")).toBe("left")
      expect(normalizePressedKey("ArrowRight")).toBe("right")
      expect(normalizePressedKey("arrowup")).toBe("up")
      expect(normalizePressedKey("ARROWUP")).toBe("up")
    })

    it("should normalize navigation key aliases", () => {
      expect(normalizePressedKey("return")).toBe("enter")
      expect(normalizePressedKey("esc")).toBe("escape")
      expect(normalizePressedKey("del")).toBe("delete")
    })

    it("should normalize letters to lowercase", () => {
      expect(normalizePressedKey("a")).toBe("a")
      expect(normalizePressedKey("A")).toBe("a")
      expect(normalizePressedKey("z")).toBe("z")
      expect(normalizePressedKey("Z")).toBe("z")
    })

    it("should keep numbers as-is", () => {
      expect(normalizePressedKey("0")).toBe("0")
      expect(normalizePressedKey("9")).toBe("9")
    })

    it("should normalize function keys to lowercase", () => {
      expect(normalizePressedKey("F1")).toBe("f1")
      expect(normalizePressedKey("f1")).toBe("f1")
      expect(normalizePressedKey("F12")).toBe("f12")
      expect(normalizePressedKey("F24")).toBe("f24")
    })

    it("should reject invalid function keys", () => {
      expect(normalizePressedKey("F0")).toBeNull()
      expect(normalizePressedKey("F25")).toBeNull()
    })

    it("should normalize supported navigation keys", () => {
      expect(normalizePressedKey("Tab")).toBe("tab")
      expect(normalizePressedKey("Enter")).toBe("enter")
      expect(normalizePressedKey("Escape")).toBe("escape")
      expect(normalizePressedKey("Backspace")).toBe("backspace")
      expect(normalizePressedKey("Delete")).toBe("delete")
      expect(normalizePressedKey("Home")).toBe("home")
      expect(normalizePressedKey("End")).toBe("end")
      expect(normalizePressedKey("PageUp")).toBe("pageup")
      expect(normalizePressedKey("PageDown")).toBe("pagedown")
    })

    it("should return null for unknown keys", () => {
      expect(normalizePressedKey("unknown")).toBeNull()
      expect(normalizePressedKey("@")).toBeNull()
    })
  })

  describe("keyLabel", () => {
    it("should format modifier keys with first letter capitalized", () => {
      expect(keyLabel("ctrl")).toBe("Ctrl")
      expect(keyLabel("alt")).toBe("Alt")
      expect(keyLabel("shift")).toBe("Shift")
      expect(keyLabel("win")).toBe("Win")
    })

    it("should format function keys with uppercase F", () => {
      expect(keyLabel("f1")).toBe("F1")
      expect(keyLabel("f12")).toBe("F12")
      expect(keyLabel("f24")).toBe("F24")
    })

    it("should format PageUp and PageDown specially", () => {
      expect(keyLabel("pageup")).toBe("PageUp")
      expect(keyLabel("pagedown")).toBe("PageDown")
    })

    it("should format single letters to uppercase", () => {
      expect(keyLabel("a")).toBe("A")
      expect(keyLabel("z")).toBe("Z")
    })

    it("should format other keys with first letter capitalized", () => {
      expect(keyLabel("space")).toBe("Space")
      expect(keyLabel("enter")).toBe("Enter")
      expect(keyLabel("tab")).toBe("Tab")
      expect(keyLabel("escape")).toBe("Escape")
      expect(keyLabel("left")).toBe("Left")
      expect(keyLabel("right")).toBe("Right")
    })

    it("should handle empty string", () => {
      expect(keyLabel("")).toBe("")
    })
  })

  describe("modifierLabel", () => {
    it("should be an alias for keyLabel", () => {
      expect(modifierLabel("ctrl")).toBe(keyLabel("ctrl"))
      expect(modifierLabel("f1")).toBe(keyLabel("f1"))
      expect(modifierLabel("a")).toBe(keyLabel("a"))
    })
  })
})
