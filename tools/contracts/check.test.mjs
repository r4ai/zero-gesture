import assert from "node:assert/strict"
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"

import { validateManifestText } from "./check.mjs"

const validCase = {
  id: "P02-WIN-001",
  obligation: "A configured trigger starts a gesture.",
  runner: "cargo-test",
  evidence_file: "src-tauri/src/domain/session.rs",
  evidence_name: "idle_starts_gesture_on_configured_trigger",
}

test("rejects malformed JSON", () => {
  assert.throws(() => validateManifestText("{"), /manifest must be valid JSON/)
})

test("rejects duplicate contract IDs", () => {
  const manifest = JSON.stringify({ cases: [validCase, validCase] })
  assert.throws(() => validateManifestText(manifest), /duplicate contract id/)
})

test("rejects duplicate evidence pairs", () => {
  const manifest = JSON.stringify({
    cases: [
      validCase,
      {
        ...validCase,
        id: "P02-WIN-002",
        obligation: "Another obligation.",
      },
    ],
  })
  assert.throws(() => validateManifestText(manifest), /duplicate evidence/)
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

test("rejects a commented source marker absent from the Cargo test list", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "contracts-cargo-list-"))
  const evidence = path.join(repo, "src-tauri", "src", "domain", "session.rs")
  mkdirSync(path.dirname(evidence), { recursive: true })
  writeFileSync(evidence, "/* #[test]\nfn commented_marker() {}\n*/\n")

  const manifest = JSON.stringify({
    cases: [
      {
        ...validCase,
        evidence_name: "commented_marker",
      },
    ],
  })
  assert.throws(
    () => validateManifestText(manifest, repo, ""),
    /Cargo test list/,
  )
})
