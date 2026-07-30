#!/usr/bin/env node

import { existsSync, readFileSync, statSync } from "node:fs"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const ALLOWED_RUNNERS = new Set(["cargo-test"])
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

    if (cargoList !== undefined) {
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
