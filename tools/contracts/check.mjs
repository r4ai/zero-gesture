#!/usr/bin/env node

import { existsSync, readFileSync, statSync } from "node:fs"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const ALLOWED_RUNNERS = new Set(["cargo-test", "macos-cargo-test"])
const CASE_FIELDS = [
  "evidence_file",
  "evidence_name",
  "id",
  "obligation",
  "runner",
]
const MANIFESTS = [
  {
    label: "P02",
    path: "contracts/p02-windows-baseline.json",
    idPattern: /^P02-(?:WIN|CONFIG|INPUT)-\d{3}$/,
    idDescription: "P02-WIN-NNN, P02-CONFIG-NNN, or P02-INPUT-NNN",
  },
  {
    label: "P03",
    path: "contracts/p03-process-ipc.json",
    idPattern: /^P03-(?:PROCESS|CODEC|IPC)-\d{3}$/,
    idDescription: "P03-PROCESS-NNN, P03-CODEC-NNN, or P03-IPC-NNN",
  },
  {
    label: "P03b",
    path: "contracts/p03b-config-owner-rcu.json",
    idPattern: /^P03B-(?:OWNER|RCU|CODEC|IPC|RECOVERY|PERSIST)-\d{3}$/,
    idDescription:
      "P03B-OWNER-NNN, P03B-RCU-NNN, P03B-CODEC-NNN, P03B-IPC-NNN, P03B-RECOVERY-NNN, or P03B-PERSIST-NNN",
  },
  {
    label: "P03c",
    path: "contracts/p03c-windows-input-owner.json",
    idPattern:
      /^P03C-(?:HOT|CONTEXT|GENERATION|INPUT|ACTION|OVERLOAD|REPLAY|RENDER|LIFECYCLE)-\d{3}$/,
    idDescription:
      "P03C-HOT-NNN, P03C-CONTEXT-NNN, P03C-GENERATION-NNN, P03C-INPUT-NNN, P03C-ACTION-NNN, P03C-OVERLOAD-NNN, P03C-REPLAY-NNN, P03C-RENDER-NNN, or P03C-LIFECYCLE-NNN",
  },
  {
    label: "P04a",
    path: "contracts/p04a-macos-packaging.json",
    idPattern: /^P04A-(?:TARGET|IDENTITY|BUNDLE|SIGNING|PROCESS)-\d{3}$/,
    idDescription:
      "P04A-TARGET-NNN, P04A-IDENTITY-NNN, P04A-BUNDLE-NNN, P04A-SIGNING-NNN, or P04A-PROCESS-NNN",
  },
  {
    label: "P04b1",
    path: "contracts/p04b1-macos-uds-control.json",
    idPattern: /^P04B1-(?:ENDPOINT|PEER|IPC|CONFIG|PROCESS)-\d{3}$/,
    idDescription:
      "P04B1-ENDPOINT-NNN, P04B1-PEER-NNN, P04B1-IPC-NNN, P04B1-CONFIG-NNN, or P04B1-PROCESS-NNN",
  },
  {
    label: "P04b2",
    path: "contracts/p04b2-macos-event-tap-owner.json",
    idPattern:
      /^P04B2-(?:HOT|OVERLOAD|DISABLE|ORDER|NORMALIZE|PASS|LIFECYCLE)-\d{3}$/,
    idDescription:
      "P04B2-HOT-NNN, P04B2-OVERLOAD-NNN, P04B2-DISABLE-NNN, P04B2-ORDER-NNN, P04B2-NORMALIZE-NNN, P04B2-PASS-NNN, or P04B2-LIFECYCLE-NNN",
  },
  {
    label: "P04b3a",
    path: "contracts/p04b3a-macos-context-resolver.json",
    idPattern:
      /^P04B3A-(?:PERMISSION|ISOLATION|REQUEST|FAILURE|STRING|IDENTITY|CACHE|MATCH|LIFECYCLE|BOUNDARY)-\d{3}$/,
    idDescription:
      "P04B3A-PERMISSION-NNN, P04B3A-ISOLATION-NNN, P04B3A-REQUEST-NNN, P04B3A-FAILURE-NNN, P04B3A-STRING-NNN, P04B3A-IDENTITY-NNN, P04B3A-CACHE-NNN, P04B3A-MATCH-NNN, P04B3A-LIFECYCLE-NNN, or P04B3A-BOUNDARY-NNN",
  },
  {
    label: "P04b3b",
    path: "contracts/p04b3b-macos-action-executor.json",
    idPattern:
      /^P04B3B-(?:TAG|ACTION|ORDER|OVERLOAD|FAILURE|CONTEXT|LIFECYCLE)-\d{3}$/,
    idDescription:
      "P04B3B-TAG-NNN, P04B3B-ACTION-NNN, P04B3B-ORDER-NNN, P04B3B-OVERLOAD-NNN, P04B3B-FAILURE-NNN, P04B3B-CONTEXT-NNN, or P04B3B-LIFECYCLE-NNN",
  },
  {
    label: "P05a",
    path: "contracts/p05a-windows-runtime-shell.json",
    idPattern: /^P05A-(?:AUTOSTART|PROCESS|SETTINGS|TRAY|LIFECYCLE)-\d{3}$/,
    idDescription:
      "P05A-AUTOSTART-NNN, P05A-PROCESS-NNN, P05A-SETTINGS-NNN, P05A-TRAY-NNN, or P05A-LIFECYCLE-NNN",
  },
  {
    label: "P05b",
    path: "contracts/p05b-windows-settings-control.json",
    idPattern: /^P05B-(?:ERROR|IPC|CAPTURE|HOT|LIFECYCLE|MACOS)-\d{3}$/,
    idDescription:
      "P05B-ERROR-NNN, P05B-IPC-NNN, P05B-CAPTURE-NNN, P05B-HOT-NNN, P05B-LIFECYCLE-NNN, or P05B-MACOS-NNN",
  },
]
const REPO_ROOT = path.resolve(fileURLToPath(new URL("../..", import.meta.url)))

function fail(message) {
  throw new Error(message)
}

function requireExactFields(value, expected, location) {
  const actual = Object.keys(value).sort()
  if (
    actual.length !== expected.length ||
    actual.some((field, index) => field !== expected[index])
  ) {
    fail(`${location} must contain exactly: ${expected.join(", ")}`)
  }
}

function requireNonemptyString(value, location) {
  if (
    typeof value !== "string" ||
    value.trim() !== value ||
    value.length === 0
  ) {
    fail(`${location} must be a non-empty trimmed string`)
  }
}

function evidencePath(repoRoot, relativePath, location) {
  const normalized = relativePath.replaceAll("\\", "/")
  if (
    normalized !== relativePath ||
    path.posix.isAbsolute(normalized) ||
    normalized.split("/").includes("..") ||
    !(
      normalized.startsWith("src-tauri/src/") ||
      normalized.startsWith("src-tauri/tests/")
    ) ||
    !normalized.endsWith(".rs")
  ) {
    fail(
      `${location} must be a Rust source path below src-tauri/src or src-tauri/tests`,
    )
  }

  const absolute = path.resolve(repoRoot, ...normalized.split("/"))
  if (!absolute.startsWith(`${path.resolve(repoRoot)}${path.sep}`)) {
    fail(`${location} escapes the repository`)
  }
  if (!existsSync(absolute) || !statSync(absolute).isFile()) {
    fail(`${location} does not exist: ${normalized}`)
  }
  return absolute
}

function rustTestPattern(name) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
  return new RegExp(`#\\s*\\[\\s*test\\s*\\]\\s*fn\\s+${escaped}\\s*\\(`)
}

function cargoTestName(contractCase) {
  if (contractCase.evidence_file.startsWith("src-tauri/tests/")) {
    return `${contractCase.evidence_name}: test`
  }
  const sourceModule = contractCase.evidence_file.slice(
    "src-tauri/src/".length,
    -".rs".length,
  )
  const moduleName =
    sourceModule === "lib"
      ? ""
      : `${sourceModule.replace(/\/mod$/, "").replaceAll("/", "::")}::`
  return `${moduleName}tests::${contractCase.evidence_name}: test`
}

export function validateManifest(
  manifest,
  repoRoot = REPO_ROOT,
  cargoList,
  profile = MANIFESTS[0],
  usedEvidence,
) {
  if (
    manifest === null ||
    typeof manifest !== "object" ||
    Array.isArray(manifest)
  ) {
    fail("manifest must be an object")
  }
  requireExactFields(manifest, ["cases"], "manifest")
  if (!Array.isArray(manifest.cases) || manifest.cases.length === 0) {
    fail("manifest.cases must be a non-empty array")
  }

  const ids = new Set()
  const obligations = new Set()
  const evidencePairs = new Set()

  for (const [index, contractCase] of manifest.cases.entries()) {
    const location = `manifest.cases[${index}]`
    if (
      contractCase === null ||
      typeof contractCase !== "object" ||
      Array.isArray(contractCase)
    ) {
      fail(`${location} must be an object`)
    }
    requireExactFields(contractCase, CASE_FIELDS, location)

    for (const field of CASE_FIELDS) {
      requireNonemptyString(contractCase[field], `${location}.${field}`)
    }
    if (!profile.idPattern.test(contractCase.id)) {
      fail(`${location}.id must match ${profile.idDescription}`)
    }
    if (ids.has(contractCase.id)) {
      fail(`duplicate contract id: ${contractCase.id}`)
    }
    ids.add(contractCase.id)

    if (obligations.has(contractCase.obligation)) {
      fail(`duplicate obligation: ${contractCase.obligation}`)
    }
    obligations.add(contractCase.obligation)

    const evidencePair = `${contractCase.evidence_file}:${contractCase.evidence_name}`
    if (evidencePairs.has(evidencePair)) {
      fail(`duplicate evidence: ${evidencePair}`)
    }
    evidencePairs.add(evidencePair)
    const priorProfile = usedEvidence?.get(evidencePair)
    if (priorProfile !== undefined) {
      fail(
        `cross-manifest evidence reuse: ${evidencePair} (${priorProfile} and ${profile.label})`,
      )
    }
    usedEvidence?.set(evidencePair, profile.label)

    if (!ALLOWED_RUNNERS.has(contractCase.runner)) {
      fail(`${location}.runner is not allowed: ${contractCase.runner}`)
    }
    if (!/^[a-z][a-z0-9_]*$/.test(contractCase.evidence_name)) {
      fail(`${location}.evidence_name must be a Rust test function name`)
    }

    const absoluteEvidence = evidencePath(
      repoRoot,
      contractCase.evidence_file,
      `${location}.evidence_file`,
    )
    const source = readFileSync(absoluteEvidence, "utf8")
    if (!rustTestPattern(contractCase.evidence_name).test(source)) {
      fail(
        `${location}.evidence_name is not a #[test] function in ${contractCase.evidence_file}: ${contractCase.evidence_name}`,
      )
    }

    if (cargoList !== undefined && contractCase.runner === "cargo-test") {
      const expected = cargoTestName(contractCase)
      const listedCount = cargoList
        .split(/\r?\n/)
        .filter((line) => line.trim() === expected).length
      if (listedCount !== 1) {
        fail(
          `${location}.evidence_name must appear exactly once in the Cargo test list: ${expected} (found ${listedCount})`,
        )
      }
    }
  }

  return manifest.cases.length
}

export function validateManifestText(
  text,
  repoRoot = REPO_ROOT,
  cargoList,
  profile = MANIFESTS[0],
  usedEvidence,
) {
  let manifest
  try {
    manifest = JSON.parse(text)
  } catch (error) {
    fail(`manifest must be valid JSON: ${error.message}`)
  }
  return validateManifest(manifest, repoRoot, cargoList, profile, usedEvidence)
}

function main() {
  const args = process.argv.slice(2)
  if (args.length !== 0 && (args.length !== 2 || args[0] !== "--cargo-list")) {
    fail("usage: check.mjs [--cargo-list <path>]")
  }
  const cargoList =
    args.length === 0 ? undefined : readFileSync(args[1], "utf8")
  const usedEvidence = new Map()
  for (const profile of MANIFESTS) {
    const manifestPath = path.resolve(REPO_ROOT, profile.path)
    const count = validateManifestText(
      readFileSync(manifestPath, "utf8"),
      REPO_ROOT,
      cargoList,
      profile,
      usedEvidence,
    )
    console.log(`Validated ${count} ${profile.label} contract cases.`)
  }
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  main()
}
