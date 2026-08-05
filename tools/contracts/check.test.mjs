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

test("rejects evidence reused by another phase manifest", () => {
  const usedEvidence = new Map()
  const p02 = {
    label: "P02",
    idPattern: /^P02-WIN-\d{3}$/,
    idDescription: "P02-WIN-NNN",
  }
  const p03 = {
    label: "P03",
    idPattern: /^P03-IPC-\d{3}$/,
    idDescription: "P03-IPC-NNN",
  }
  assert.equal(
    validateManifestText(
      JSON.stringify({ cases: [validCase] }),
      undefined,
      undefined,
      p02,
      usedEvidence,
    ),
    1,
  )
  assert.throws(
    () =>
      validateManifestText(
        JSON.stringify({
          cases: [
            {
              ...validCase,
              id: "P03-IPC-001",
              obligation: "The later phase makes a different claim.",
            },
          ],
        }),
        undefined,
        undefined,
        p03,
        usedEvidence,
      ),
    /cross-manifest evidence reuse/,
  )
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

test("accepts runnable integration-test evidence", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "contracts-integration-"))
  const evidence = path.join(
    repo,
    "src-tauri",
    "tests",
    "p03_engine_process.rs",
  )
  mkdirSync(path.dirname(evidence), { recursive: true })
  writeFileSync(evidence, "#[test]\nfn actual_engine_child() {}\n")

  const manifest = JSON.stringify({
    cases: [
      {
        ...validCase,
        evidence_file: "src-tauri/tests/p03_engine_process.rs",
        evidence_name: "actual_engine_child",
      },
    ],
  })
  assert.equal(
    validateManifestText(manifest, repo, "actual_engine_child: test"),
    1,
  )
})

test("accepts runnable crate-root unit-test evidence", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "contracts-lib-test-"))
  const evidence = path.join(repo, "src-tauri", "src", "lib.rs")
  mkdirSync(path.dirname(evidence), { recursive: true })
  writeFileSync(evidence, "#[test]\nfn runtime_projection() {}\n")

  const manifest = JSON.stringify({
    cases: [
      {
        ...validCase,
        evidence_file: "src-tauri/src/lib.rs",
        evidence_name: "runtime_projection",
      },
    ],
  })
  assert.equal(
    validateManifestText(manifest, repo, "tests::runtime_projection: test"),
    1,
  )
})

test("accepts macOS-only Cargo evidence absent from a Windows Cargo list", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "contracts-macos-list-"))
  const evidence = path.join(
    repo,
    "src-tauri",
    "tests",
    "p04a_macos_packaging.rs",
  )
  mkdirSync(path.dirname(evidence), { recursive: true })
  writeFileSync(evidence, "#[test]\nfn packaged_engine_has_no_webview() {}\n")

  const manifest = JSON.stringify({
    cases: [
      {
        ...validCase,
        runner: "macos-cargo-test",
        evidence_file: "src-tauri/tests/p04a_macos_packaging.rs",
        evidence_name: "packaged_engine_has_no_webview",
      },
    ],
  })
  assert.equal(validateManifestText(manifest, repo, ""), 1)
})

test("accepts installed Windows evidence outside the ordinary Cargo list", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "contracts-windows-installed-"))
  const evidence = path.join(
    repo,
    "src-tauri",
    "tests",
    "p05c_windows_installed.rs",
  )
  mkdirSync(path.dirname(evidence), { recursive: true })
  writeFileSync(evidence, "#[test]\nfn installed_release_lifecycle() {}\n")

  const manifest = JSON.stringify({
    cases: [
      {
        ...validCase,
        runner: "windows-installed-acceptance",
        evidence_file: "src-tauri/tests/p05c_windows_installed.rs",
        evidence_name: "installed_release_lifecycle",
      },
    ],
  })
  assert.equal(validateManifestText(manifest, repo, ""), 1)
})
