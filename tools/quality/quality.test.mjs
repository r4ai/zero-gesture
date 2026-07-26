import assert from "node:assert/strict"
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"

import {
  buildSummary,
  classifySource,
  normalizePath,
  serializeSummary,
} from "./quality.mjs"

function metrics({
  ploc,
  functions,
  cognitiveSum,
  cognitiveMax,
  cognitiveValue = 0,
  cyclomaticSum,
  cyclomaticMax,
  cyclomaticValue = 1,
}) {
  return {
    loc: { ploc },
    nom: { total: functions },
    cognitive: { sum: cognitiveSum, max: cognitiveMax, value: cognitiveValue },
    cyclomatic: {
      sum: cyclomaticSum,
      max: cyclomaticMax,
      value: cyclomaticValue,
    },
  }
}

function space(name, start, end, values, spaces = []) {
  return {
    name,
    start_line: start,
    end_line: end,
    kind: "function",
    spaces,
    metrics: metrics(values),
  }
}

function record(name, values, spaces = []) {
  return {
    name,
    start_line: 1,
    end_line: 100,
    kind: "unit",
    spaces,
    metrics: metrics(values),
  }
}

test("classification includes product/test/support and explicitly excludes generated output", () => {
  assert.deepEqual(classifySource("src\\main.tsx"), {
    path: "src/main.tsx",
    language: "typescript",
    classification: "product",
    mode: "analyzed",
  })
  assert.equal(classifySource("src/button.stories.tsx").classification, "test")
  assert.deepEqual(classifySource("src/vite-env.d.ts"), {
    path: "src/vite-env.d.ts",
    language: "typescript",
    classification: "support",
    mode: "declarative",
  })
  assert.equal(classifySource("src/routeTree.gen.ts").mode, "generated")
  assert.equal(classifySource("target/debug/generated.rs").mode, "generated")
  assert.equal(classifySource("loose.ts").classification, "unclassified")
  assert.equal(normalizePath(".\\src\\main.tsx"), "src/main.tsx")
})

test("aggregation uses function own-values and does not double-count nested functions", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "quality-aggregation-"))
  const source = classifySource("src-tauri/src/lib.rs")
  const nested = space("nested", 4, 6, {
    ploc: 2,
    functions: 1,
    cognitiveSum: 2,
    cognitiveMax: 2,
    cognitiveValue: 2,
    cyclomaticSum: 3,
    cyclomaticMax: 3,
    cyclomaticValue: 3,
  })
  const product = space(
    "product",
    1,
    8,
    {
      ploc: 8,
      functions: 2,
      cognitiveSum: 6,
      cognitiveMax: 4,
      cognitiveValue: 4,
      cyclomaticSum: 8,
      cyclomaticMax: 5,
      cyclomaticValue: 5,
    },
    [nested],
  )
  const rustTest = space("test_only", 20, 23, {
    ploc: 4,
    functions: 1,
    cognitiveSum: 3,
    cognitiveMax: 3,
    cognitiveValue: 3,
    cyclomaticSum: 4,
    cyclomaticMax: 4,
    cyclomaticValue: 4,
  })
  const full = record(
    source.path,
    {
      ploc: 12,
      functions: 3,
      cognitiveSum: 99,
      cognitiveMax: 99,
      cyclomaticSum: 99,
      cyclomaticMax: 99,
    },
    [product, rustTest],
  )
  const pruned = record(
    source.path,
    {
      ploc: 8,
      functions: 2,
      cognitiveSum: 88,
      cognitiveMax: 88,
      cyclomaticSum: 88,
      cyclomaticMax: 88,
    },
    [product],
  )

  const summary = buildSummary({
    repo,
    revision: "a".repeat(40),
    cliSource: "merge-base",
    sources: [source],
    fullRecords: [full],
    prunedRecords: [pruned],
  })
  const productRow = summary.metrics.find(
    (row) => row.language === "rust" && row.classification === "product",
  )
  const testRow = summary.metrics.find(
    (row) => row.language === "rust" && row.classification === "test",
  )

  assert.equal(productRow.cognitive.sum, 6)
  assert.equal(productRow.cyclomatic.sum, 8)
  assert.equal(productRow.function_count, 2)
  assert.equal(testRow.cognitive.sum, 3)
  assert.equal(testRow.cognitive.max, 3)
  assert.equal(testRow.function_count, 1)
})

test("path and record order normalize to byte-identical summaries", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "quality-determinism-"))
  mkdirSync(path.join(repo, "src"), { recursive: true })
  writeFileSync(
    path.join(repo, "src", "types.d.ts"),
    "declare const a: string\n\nexport { a }\n",
  )

  const sources = [
    classifySource("src\\b.tsx"),
    classifySource(".\\src\\a.ts"),
    classifySource("src/types.d.ts"),
  ]
  const a = record("./src/a.ts", {
    ploc: 3,
    functions: 1,
    cognitiveSum: 1,
    cognitiveMax: 1,
    cyclomaticSum: 2,
    cyclomaticMax: 2,
  })
  const b = record("src\\b.tsx", {
    ploc: 5,
    functions: 2,
    cognitiveSum: 4,
    cognitiveMax: 3,
    cyclomaticSum: 6,
    cyclomaticMax: 4,
  })
  const args = {
    repo,
    revision: "b".repeat(40),
    cliSource: "head-bootstrap",
  }

  const first = buildSummary({
    ...args,
    sources,
    fullRecords: [b, a],
    prunedRecords: [a, b],
  })
  const second = buildSummary({
    ...args,
    sources: [...sources].reverse(),
    fullRecords: [a, b],
    prunedRecords: [b, a],
  })

  assert.equal(serializeSummary(first), serializeSummary(second))
  const support = first.metrics.find(
    (row) => row.language === "typescript" && row.classification === "support",
  )
  assert.equal(support.declarative_file_count, 1)
  assert.equal(support.declarative_nonblank_lines, 2)
})
