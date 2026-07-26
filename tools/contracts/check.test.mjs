import assert from "node:assert/strict"
import test from "node:test"

import { validateManifestText } from "./check.mjs"

const validCase = {
  id: "P02-WIN-001",
  obligation: "A configured trigger starts a gesture.",
  runner: "cargo-test",
  evidence_file: "src-tauri/src/hook/state.rs",
  evidence_name: "idle_starts_gesture_on_configured_trigger",
}

test("rejects malformed JSON", () => {
  assert.throws(() => validateManifestText("{"), /manifest must be valid JSON/)
})

test("rejects duplicate contract IDs", () => {
  const manifest = JSON.stringify({ cases: [validCase, validCase] })
  assert.throws(() => validateManifestText(manifest), /duplicate contract id/)
})

test("rejects missing evidence files", () => {
  const manifest = JSON.stringify({
    cases: [
      {
        ...validCase,
        evidence_file: "src-tauri/src/missing.rs",
      },
    ],
  })
  assert.throws(() => validateManifestText(manifest), /does not exist/)
})
