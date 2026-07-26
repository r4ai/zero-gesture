#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process"
import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const SCHEMA_VERSION = 1
const ANALYZER_VERSION = "2.0.0"
const CANONICAL_PLATFORM = "ubuntu-24.04-x86_64"
const DIAGNOSTIC_THRESHOLDS = Object.freeze({
  cognitive: 15,
  cyclomatic: 10,
})

// This is the single executable definition of the measurement scope.
const SCOPE = Object.freeze({
  id: "tracked-rust-typescript-v1",
  extensions: [".rs", ".ts", ".tsx"],
  generatedSegments: [
    "build",
    "coverage",
    "dist",
    "generated",
    "node_modules",
    "storybook-static",
    "target",
    "vendor",
  ],
  generatedSuffixes: [".gen.ts", ".gen.tsx", ".generated.ts", ".generated.tsx"],
  testSegments: ["__tests__", "test", "tests"],
  testMarkers: [".spec.", ".stories.", ".test."],
  supportFiles: [
    "src-tauri/build.rs",
    "vite.config.ts",
    "vitest.config.ts",
    "vitest.shims.d.ts",
  ],
  supportPrefixes: [".storybook/"],
  productPrefixes: ["src-tauri/src/", "src/"],
})

const LANGUAGES = ["rust", "typescript"]
const CLASSIFICATIONS = ["product", "test", "support"]

function fail(message) {
  throw new Error(message)
}

export function normalizePath(value) {
  return value.replaceAll("\\", "/").replace(/^\.\/+/, "")
}

function extensionOf(file) {
  if (file.endsWith(".tsx")) return ".tsx"
  if (file.endsWith(".ts")) return ".ts"
  if (file.endsWith(".rs")) return ".rs"
  return path.posix.extname(file)
}

export function classifySource(input) {
  const file = normalizePath(input)
  const extension = extensionOf(file)
  if (!SCOPE.extensions.includes(extension)) return null

  const segments = file.split("/")
  const filename = segments.at(-1)
  const language = extension === ".rs" ? "rust" : "typescript"
  const generated =
    SCOPE.generatedSegments.some((segment) => segments.includes(segment)) ||
    SCOPE.generatedSuffixes.some((suffix) => file.endsWith(suffix))

  if (generated) {
    return {
      path: file,
      language,
      classification: "excluded",
      mode: "generated",
    }
  }

  const declarative = language === "typescript" && file.endsWith(".d.ts")
  if (declarative) {
    return {
      path: file,
      language,
      classification: "support",
      mode: "declarative",
    }
  }

  const test =
    SCOPE.testSegments.some((segment) => segments.includes(segment)) ||
    SCOPE.testMarkers.some((marker) => filename.includes(marker)) ||
    (language === "rust" && filename.endsWith("_test.rs"))
  if (test)
    return { path: file, language, classification: "test", mode: "analyzed" }

  const support =
    SCOPE.supportFiles.includes(file) ||
    SCOPE.supportPrefixes.some((prefix) => file.startsWith(prefix))
  if (support) {
    return {
      path: file,
      language,
      classification: "support",
      mode: "analyzed",
    }
  }

  if (SCOPE.productPrefixes.some((prefix) => file.startsWith(prefix))) {
    return { path: file, language, classification: "product", mode: "analyzed" }
  }

  return {
    path: file,
    language,
    classification: "unclassified",
    mode: "unclassified",
  }
}

function trackedSources(repo) {
  const output = execFileSync(
    "git",
    ["-C", repo, "ls-files", "-z", "--", "*.rs", "*.ts", "*.tsx"],
    { encoding: "utf8" },
  )
  return output
    .split("\0")
    .filter(Boolean)
    .map(classifySource)
    .sort((left, right) => left.path.localeCompare(right.path))
}

function parseArguments(argv) {
  const [command, ...tokens] = argv
  const options = {}
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]
    if (!token.startsWith("--")) fail(`unexpected argument: ${token}`)
    const name = token.slice(2)
    const value = tokens[index + 1]
    if (!value || value.startsWith("--")) fail(`missing value for --${name}`)
    options[name] = value
    index += 1
  }
  return { command, options }
}

function analyzerVersion(bca) {
  const result = spawnSync(bca, ["--version"], { encoding: "utf8" })
  if (result.error) fail(`cannot execute analyzer: ${result.error.message}`)
  if (result.status !== 0)
    fail(`analyzer version check failed: ${result.stderr.trim()}`)
  const versionText = `${result.stdout} ${result.stderr}`.trim()
  if (
    !new RegExp(`\\b${ANALYZER_VERSION.replaceAll(".", "\\.")}\\b`).test(
      versionText,
    )
  ) {
    fail(
      `expected big-code-analysis-cli ${ANALYZER_VERSION}, got: ${versionText}`,
    )
  }
}

function runAnalyzer({ bca, repo, files, output, excludeTests }) {
  const args = [
    "--report-skipped",
    "--warnings",
    "metrics",
    "--no-config",
    "--no-skip-generated",
    "--jobs",
    "1",
    "--cyclomatic-count-try=true",
    "--metrics",
    "cognitive,cyclomatic,ploc,nom",
    "--format",
    "json",
    "--pretty",
    "--output",
    output,
  ]
  if (excludeTests) args.push("--exclude-tests")
  args.push(...files)

  const result = spawnSync(bca, args, {
    cwd: repo,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  })
  if (result.error) fail(`analyzer execution failed: ${result.error.message}`)
  if (result.status !== 0) {
    fail(
      `analyzer exited ${result.status}: ${result.stderr.trim() || result.stdout.trim()}`,
    )
  }
  if (result.stderr.trim())
    fail(`unexpected analyzer skip/error output: ${result.stderr.trim()}`)
}

function readRaw(file) {
  let value
  try {
    value = JSON.parse(readFileSync(file, "utf8"))
  } catch (error) {
    fail(`cannot parse analyzer JSON ${file}: ${error.message}`)
  }
  if (!Array.isArray(value)) fail(`analyzer JSON must be an array: ${file}`)
  return value
}

function recordMap(records, expectedPaths, label) {
  const mapped = new Map()
  for (const record of records) {
    if (
      !record ||
      typeof record !== "object" ||
      typeof record.name !== "string"
    ) {
      fail(`${label} contains a record without a string name`)
    }
    const name = normalizePath(record.name)
    if (mapped.has(name)) fail(`${label} contains duplicate record: ${name}`)
    mapped.set(name, record)
  }

  const expected = [...expectedPaths].sort()
  const actual = [...mapped.keys()].sort()
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    const missing = expected.filter((file) => !mapped.has(file))
    const unexpected = actual.filter((file) => !expectedPaths.has(file))
    fail(
      `${label} scope mismatch; missing=[${missing.join(", ")}], unexpected=[${unexpected.join(", ")}]`,
    )
  }
  return mapped
}

function numberAt(object, keys, label) {
  let value = object
  for (const key of keys) value = value?.[key]
  if (!Number.isFinite(value) || value < 0)
    fail(`invalid analyzer metric ${label}`)
  return value
}

function functionSpaces(record) {
  const spaces = []
  const visit = (space) => {
    if (!space || typeof space !== "object" || !Array.isArray(space.spaces)) {
      fail(`invalid analyzer space tree for ${record.name}`)
    }
    if (space.kind === "function") spaces.push(space)
    for (const child of space.spaces) visit(child)
  }
  visit(record)
  return spaces
}

function spaceIdentity(space) {
  return [space.kind, space.start_line, space.end_line, space.name ?? ""].join(
    ":",
  )
}

function recordContribution(record) {
  const spaces = functionSpaces(record)
  const cognitiveValues = spaces.map((space) =>
    numberAt(space, ["metrics", "cognitive", "value"], "cognitive.value"),
  )
  const cyclomaticValues = spaces.map((space) =>
    numberAt(space, ["metrics", "cyclomatic", "value"], "cyclomatic.value"),
  )
  return {
    ploc: numberAt(record, ["metrics", "loc", "ploc"], "loc.ploc"),
    function_count: numberAt(record, ["metrics", "nom", "total"], "nom.total"),
    cognitive_max: Math.max(0, ...cognitiveValues),
    cognitive_sum: cognitiveValues.reduce((sum, value) => sum + value, 0),
    cyclomatic_max: Math.max(0, ...cyclomaticValues),
    cyclomatic_sum: cyclomaticValues.reduce((sum, value) => sum + value, 0),
  }
}

function diagnosticCounts(spaces) {
  let cognitive = 0
  let cyclomatic = 0
  for (const space of spaces) {
    if (
      numberAt(space, ["metrics", "cognitive", "value"], "cognitive.value") >
      DIAGNOSTIC_THRESHOLDS.cognitive
    ) {
      cognitive += 1
    }
    if (
      numberAt(space, ["metrics", "cyclomatic", "value"], "cyclomatic.value") >
      DIAGNOSTIC_THRESHOLDS.cyclomatic
    ) {
      cyclomatic += 1
    }
  }
  return { cognitive, cyclomatic }
}

function subtractContribution(full, pruned, fullRecord, prunedRecord) {
  const difference = {}
  for (const key of ["ploc", "function_count"]) {
    difference[key] = full[key] - pruned[key]
    if (difference[key] < 0)
      fail(`Rust test subtraction produced a negative ${key}`)
  }

  const retained = new Set(functionSpaces(prunedRecord).map(spaceIdentity))
  const testSpaces = functionSpaces(fullRecord).filter(
    (space) => !retained.has(spaceIdentity(space)),
  )
  const cognitiveValues = testSpaces.map((space) =>
    numberAt(space, ["metrics", "cognitive", "value"], "cognitive.value"),
  )
  const cyclomaticValues = testSpaces.map((space) =>
    numberAt(space, ["metrics", "cyclomatic", "value"], "cyclomatic.value"),
  )
  difference.cognitive_max = Math.max(0, ...cognitiveValues)
  difference.cognitive_sum = cognitiveValues.reduce(
    (sum, value) => sum + value,
    0,
  )
  difference.cyclomatic_max = Math.max(0, ...cyclomaticValues)
  difference.cyclomatic_sum = cyclomaticValues.reduce(
    (sum, value) => sum + value,
    0,
  )
  return { contribution: difference, spaces: testSpaces }
}

function emptyRow(language, classification) {
  return {
    language,
    classification,
    file_count: 0,
    analyzed_file_count: 0,
    declarative_file_count: 0,
    declarative_nonblank_lines: 0,
    ploc: 0,
    function_count: 0,
    cognitive: { max: 0, sum: 0, above_diagnostic_threshold: 0 },
    cyclomatic: { max: 0, sum: 0, above_diagnostic_threshold: 0 },
  }
}

function addContribution(row, file, contribution, spaces) {
  row.files.add(file)
  row.analyzedFiles.add(file)
  row.ploc += contribution.ploc
  row.function_count += contribution.function_count
  row.cognitive.max = Math.max(row.cognitive.max, contribution.cognitive_max)
  row.cognitive.sum += contribution.cognitive_sum
  row.cyclomatic.max = Math.max(row.cyclomatic.max, contribution.cyclomatic_max)
  row.cyclomatic.sum += contribution.cyclomatic_sum
  const diagnostics = diagnosticCounts(spaces)
  row.cognitive.above_diagnostic_threshold += diagnostics.cognitive
  row.cyclomatic.above_diagnostic_threshold += diagnostics.cyclomatic
}

function finalizeRows(rows) {
  return rows.map((row) => {
    const { files, analyzedFiles, ...values } = row
    return {
      ...values,
      file_count: files.size,
      analyzed_file_count: analyzedFiles.size,
    }
  })
}

export function buildSummary({
  repo,
  revision,
  cliSource,
  sources,
  fullRecords,
  prunedRecords,
}) {
  const orderedSources = [...sources].sort((left, right) =>
    left.path.localeCompare(right.path),
  )
  const unclassified = orderedSources.filter(
    (source) => source.classification === "unclassified",
  )
  if (unclassified.length) {
    fail(
      `unclassified supported source: ${unclassified.map((source) => source.path).join(", ")}`,
    )
  }

  const analyzed = orderedSources.filter((source) => source.mode === "analyzed")
  const analyzedPaths = new Set(analyzed.map((source) => source.path))
  const full = recordMap(fullRecords, analyzedPaths, "full analyzer output")
  const pruned = recordMap(
    prunedRecords,
    analyzedPaths,
    "test-pruned analyzer output",
  )

  const rows = new Map()
  for (const language of LANGUAGES) {
    for (const classification of CLASSIFICATIONS) {
      const row = emptyRow(language, classification)
      row.files = new Set()
      row.analyzedFiles = new Set()
      rows.set(`${language}:${classification}`, row)
    }
  }

  for (const source of orderedSources) {
    if (source.mode === "generated") continue
    const row = rows.get(`${source.language}:${source.classification}`)
    if (source.mode === "declarative") {
      row.files.add(source.path)
      row.declarative_file_count += 1
      const contents = readFileSync(
        path.join(repo, ...source.path.split("/")),
        "utf8",
      )
      row.declarative_nonblank_lines += contents
        .split(/\r?\n/)
        .filter((line) => line.trim()).length
      continue
    }

    const fullRecord = full.get(source.path)
    const prunedRecord = pruned.get(source.path)
    if (
      source.language === "rust" &&
      (source.classification === "product" ||
        source.classification === "support")
    ) {
      addContribution(
        row,
        source.path,
        recordContribution(prunedRecord),
        functionSpaces(prunedRecord),
      )
      const test = subtractContribution(
        recordContribution(fullRecord),
        recordContribution(prunedRecord),
        fullRecord,
        prunedRecord,
      )
      const hasTests =
        test.spaces.length > 0 ||
        Object.values(test.contribution).some((metric) => metric !== 0)
      if (hasTests) {
        addContribution(
          rows.get(`${source.language}:test`),
          source.path,
          test.contribution,
          test.spaces,
        )
      }
    } else {
      addContribution(
        row,
        source.path,
        recordContribution(fullRecord),
        functionSpaces(fullRecord),
      )
    }
  }

  const normalizedSources = orderedSources.map(
    ({ path: sourcePath, language, classification, mode }) => ({
      path: sourcePath,
      language,
      classification,
      mode,
    }),
  )
  const excludedCount = normalizedSources.filter(
    (source) => source.mode === "generated",
  ).length
  const declarativeCount = normalizedSources.filter(
    (source) => source.mode === "declarative",
  ).length

  return {
    schema_version: SCHEMA_VERSION,
    revision,
    measurement: {
      platform: CANONICAL_PLATFORM,
      analyzer: {
        name: "big-code-analysis-cli",
        version: ANALYZER_VERSION,
      },
      scope_id: SCOPE.id,
      cli_source: cliSource,
      bootstrap_exception: cliSource === "head-bootstrap",
      diagnostic_thresholds: DIAGNOSTIC_THRESHOLDS,
      aggregation:
        "file-root PLOC/counts; one own value per function space for complexity; no nested double count",
      runtime_kpis: "deferred",
    },
    scope: {
      tracked_supported_file_count: normalizedSources.length,
      included_file_count: normalizedSources.length - excludedCount,
      analyzed_file_count: analyzed.length,
      declarative_file_count: declarativeCount,
      excluded_generated_file_count: excludedCount,
      unclassified_file_count: 0,
      analyzer_scope_mismatch_count: 0,
      files: normalizedSources,
    },
    metrics: finalizeRows([...rows.values()]),
    analyzer_diagnostics: {
      skipped_file_count: 0,
      error_count: 0,
    },
  }
}

export function serializeSummary(summary) {
  return `${JSON.stringify(summary, null, 2)}\n`
}

function metricRows(summary) {
  return new Map(
    summary.metrics.map((row) => [
      `${row.language}:${row.classification}`,
      row,
    ]),
  )
}

function delta(value, baseline) {
  return value - baseline
}

function signed(value) {
  return value > 0 ? `+${value}` : `${value}`
}

export function compareSummaries(base, head) {
  for (const summary of [base, head]) {
    if (summary.schema_version !== SCHEMA_VERSION)
      fail("summary schema version mismatch")
    if (summary.measurement.scope_id !== SCOPE.id)
      fail("summary scope definition mismatch")
    if (summary.measurement.platform !== CANONICAL_PLATFORM)
      fail("summary platform mismatch")
    if (summary.measurement.analyzer.version !== ANALYZER_VERSION) {
      fail("summary analyzer version mismatch")
    }
  }

  const baseRows = metricRows(base)
  const headRows = metricRows(head)
  const metrics = []
  for (const language of LANGUAGES) {
    for (const classification of CLASSIFICATIONS) {
      const key = `${language}:${classification}`
      const before = baseRows.get(key)
      const after = headRows.get(key)
      if (!before || !after) fail(`missing metric row: ${key}`)
      metrics.push({
        language,
        classification,
        file_count: delta(after.file_count, before.file_count),
        ploc: delta(after.ploc, before.ploc),
        function_count: delta(after.function_count, before.function_count),
        cognitive: {
          max: delta(after.cognitive.max, before.cognitive.max),
          sum: delta(after.cognitive.sum, before.cognitive.sum),
          above_diagnostic_threshold: delta(
            after.cognitive.above_diagnostic_threshold,
            before.cognitive.above_diagnostic_threshold,
          ),
        },
        cyclomatic: {
          max: delta(after.cyclomatic.max, before.cyclomatic.max),
          sum: delta(after.cyclomatic.sum, before.cyclomatic.sum),
          above_diagnostic_threshold: delta(
            after.cyclomatic.above_diagnostic_threshold,
            before.cyclomatic.above_diagnostic_threshold,
          ),
        },
      })
    }
  }

  return {
    schema_version: SCHEMA_VERSION,
    base_revision: base.revision,
    head_revision: head.revision,
    scope_id: SCOPE.id,
    cli_source: head.measurement.cli_source,
    bootstrap_exception: head.measurement.bootstrap_exception,
    scope: {
      tracked_supported_file_count: delta(
        head.scope.tracked_supported_file_count,
        base.scope.tracked_supported_file_count,
      ),
      included_file_count: delta(
        head.scope.included_file_count,
        base.scope.included_file_count,
      ),
      analyzer_scope_mismatch_count: 0,
    },
    metrics,
    gate: {
      complexity_growth: "review-only",
      hard_failures: "reproducibility/scope/parse/skip/error/tool only",
    },
  }
}

export function renderMarkdown(comparison) {
  const lines = [
    "## Static quality/KPI delta",
    "",
    `Base \`${comparison.base_revision}\` → head \`${comparison.head_revision}\``,
    "",
  ]
  if (comparison.bootstrap_exception) {
    lines.push(
      "> P01a bootstrap: the merge base has no quality CLI, so the head CLI/scope measured both sides.",
      "",
    )
  }
  lines.push(
    "| Language | Class | Δ files | Δ PLOC | Δ functions | Δ cognitive max / sum / diagnostic | Δ cyclomatic max / sum / diagnostic |",
    "| --- | --- | ---: | ---: | ---: | ---: | ---: |",
  )
  for (const row of comparison.metrics) {
    lines.push(
      `| ${row.language} | ${row.classification} | ${signed(row.file_count)} | ${signed(row.ploc)} | ${signed(row.function_count)} | ${signed(row.cognitive.max)} / ${signed(row.cognitive.sum)} / ${signed(row.cognitive.above_diagnostic_threshold)} | ${signed(row.cyclomatic.max)} / ${signed(row.cyclomatic.sum)} / ${signed(row.cyclomatic.above_diagnostic_threshold)} |`,
    )
  }
  lines.push(
    "",
    "Complexity deltas are review signals, not hard gates. Scope, parse, analyzer skip/error, tool, and reproducibility failures are hard gates.",
    "",
  )
  return lines.join("\n")
}

function snapshot(options) {
  const repo = path.resolve(options.repo ?? process.cwd())
  const outputDirectory = path.resolve(
    options.out ?? path.join(repo, ".quality", "head"),
  )
  const rawDirectory = path.join(outputDirectory, "raw")
  const bca = options.bca ?? process.env.BCA ?? "bca"
  const revision =
    options.revision ??
    execFileSync("git", ["-C", repo, "rev-parse", "HEAD"], {
      encoding: "utf8",
    }).trim()
  const cliSource = options["cli-source"] ?? "merge-base"
  if (!["merge-base", "head-bootstrap"].includes(cliSource)) {
    fail(`invalid --cli-source: ${cliSource}`)
  }

  const sources = trackedSources(repo)
  const unclassified = sources.filter(
    (source) => source.classification === "unclassified",
  )
  if (unclassified.length) {
    fail(
      `unclassified supported source: ${unclassified.map((source) => source.path).join(", ")}`,
    )
  }
  const files = sources
    .filter((source) => source.mode === "analyzed")
    .map((source) => source.path)
  mkdirSync(rawDirectory, { recursive: true })
  analyzerVersion(bca)

  const fullPath = path.join(rawDirectory, "bca-full.json")
  const prunedPath = path.join(rawDirectory, "bca-test-pruned.json")
  runAnalyzer({ bca, repo, files, output: fullPath, excludeTests: false })
  runAnalyzer({ bca, repo, files, output: prunedPath, excludeTests: true })

  const summary = buildSummary({
    repo,
    revision,
    cliSource,
    sources,
    fullRecords: readRaw(fullPath),
    prunedRecords: readRaw(prunedPath),
  })
  writeFileSync(
    path.join(outputDirectory, "summary.json"),
    serializeSummary(summary),
  )
}

function compare(options) {
  if (!options.base || !options.head) fail("compare requires --base and --head")
  const base = JSON.parse(readFileSync(path.resolve(options.base), "utf8"))
  const head = JSON.parse(readFileSync(path.resolve(options.head), "utf8"))
  const comparison = compareSummaries(base, head)
  if (options.output) {
    const output = path.resolve(options.output)
    mkdirSync(path.dirname(output), { recursive: true })
    writeFileSync(output, serializeSummary(comparison))
  }
  process.stdout.write(renderMarkdown(comparison))
}

function main() {
  const { command, options } = parseArguments(process.argv.slice(2))
  if (command === "snapshot") snapshot(options)
  else if (command === "compare") compare(options)
  else fail("usage: quality.mjs <snapshot|compare> [--name value ...]")
}

const invokedPath = process.argv[1] ? path.resolve(process.argv[1]) : ""
if (invokedPath === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (error) {
    process.stderr.write(`quality: ${error.message}\n`)
    process.exitCode = 1
  }
}
