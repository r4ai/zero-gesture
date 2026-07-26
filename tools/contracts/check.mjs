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
const DEFAULT_MANIFEST = "contracts/p02-windows-baseline.json"
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
    !normalized.startsWith("src-tauri/src/") ||
    !normalized.endsWith(".rs")
  ) {
    fail(`${location} must be a Rust source path below src-tauri/src`)
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

export function validateManifest(manifest, repoRoot = REPO_ROOT) {
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
    if (!/^P02-WIN-\d{3}$/.test(contractCase.id)) {
      fail(`${location}.id must match P02-WIN-NNN`)
    }
    if (ids.has(contractCase.id)) {
      fail(`duplicate contract id: ${contractCase.id}`)
    }
    ids.add(contractCase.id)

    if (obligations.has(contractCase.obligation)) {
      fail(`duplicate obligation: ${contractCase.obligation}`)
    }
    obligations.add(contractCase.obligation)

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
  }

  return manifest.cases.length
}

export function validateManifestText(text, repoRoot = REPO_ROOT) {
  let manifest
  try {
    manifest = JSON.parse(text)
  } catch (error) {
    fail(`manifest must be valid JSON: ${error.message}`)
  }
  return validateManifest(manifest, repoRoot)
}

function main() {
  const manifestPath = path.resolve(
    REPO_ROOT,
    process.argv[2] ?? DEFAULT_MANIFEST,
  )
  const count = validateManifestText(readFileSync(manifestPath, "utf8"))
  console.log(`Validated ${count} P02 Windows contract cases.`)
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  main()
}
