# ADR 0006: Measure static quality reproducibly before adding contract and runtime gates

- Status: Accepted
- Date: 2026-07-27

## Context

[ADR 0005](0005-quality-contracts-and-delivery-plan.md) fixes the product contracts, provisional complexity snapshot, observed runner cases, and delivery order.
P01 originally grouped three different kinds of evidence: static source measurement, machine-readable contract/evidence mapping, and runtime performance measurement.
They have different prerequisites and failure modes.
The current repository can measure source and run existing tests, but the separated Engine process required for CPU, RSS, and input-latency measurement does not exist yet.

The P01 work is therefore split without redefining any ADR 0005 contract:

| Part | Scope | Exit |
| --- | --- | --- |
| P01a | reproducible static quality/KPI measurement and execution of existing tests in CI | canonical base/head artifacts and deterministic delta summary |
| P01b | machine-readable contract and logical-test inventory | ADR 0005 `O`, `O_v`, `U`, `T`, and related evidence fields become measurable |
| P01c | runtime CPU, RSS, callback, and accepted-action latency harness | runs only after the relevant Engine/process boundary exists |

This ADR decides P01a only.
It does not add a contract manifest, claim contract coverage, or change product/runtime behavior.

## Decision

### Canonical environment and analyzer

The canonical measurement environment is GitHub-hosted `ubuntu-24.04` on `x86_64`.
Static complexity uses `big-code-analysis-cli==2.0.0` only in the quality CI job.
It is installed into a new isolated virtual environment with `--require-hashes`, `--only-binary=:all:`, and `--no-deps`.
The only accepted artifact is the manylinux x86-64 wheel with SHA-256 `62316880b772e2be633dccb27773f3bd42b2915376d50f021dd01e38c0405a52`.
No runtime/package dependency and no cross-OS wheel hash are added.

The Node quality CLI uses only Node standard-library modules.
It invokes BCA with one worker, explicit metric selection, generated-code auto-skipping disabled, Rust `?` counting enabled, and repository BCA configuration discovery disabled.
Analyzer non-zero exit, stderr warning/skip output, invalid JSON, missing metric fields, duplicate output, or any difference between requested and returned files is a hard failure.

The published BCA 2.0.0 wheel rejects the aggregate selector `loc` even though its JSON groups line metrics below `metrics.loc`.
P01a therefore requests the exact supported selector `ploc` and reads only `metrics.loc.ploc`; it does not request or infer the other LOC variants.

### Fixed scope and classification

`tools/quality/quality.mjs` is the single executable scope definition.
The entry set is every Git-tracked `.rs`, `.ts`, and `.tsx` file.
It classifies every entry as product, test, support, declarative support, or an explicit generated/vendor/build exclusion.
An otherwise supported file that matches no classification is a hard failure.

- product is handwritten Rust below `src-tauri/src/` and TypeScript/TSX below `src/`;
- test is a conventional test/spec/story filename or test directory;
- support is build/configuration source such as `src-tauri/build.rs`, Vite/Vitest configuration, and Storybook configuration;
- TypeScript declaration files are declarative support and receive file and nonblank-line counts only;
- generated, vendored, dependency, coverage, and build-output paths are excluded explicitly, including `src/routeTree.gen.ts`.

Rust inline tests share product/support files.
BCA 2.0.0 does not label them in its JSON, so P01a runs the same Rust inputs both with and without `--exclude-tests`.
Product/support PLOC and function counts come from the test-pruned file root.
Test PLOC and function counts are the non-negative full-minus-pruned difference.
Test complexity sums, maxima, and diagnostic counts come from function spaces present only in the full tree.
This is the smallest adaptation to BCA 2.0.0's actual output; no missing label or metric is invented.

### Metrics and aggregation

Rust and TypeScript remain separate.
Product, test, and support remain separate.
No composite score or language/category total is produced.

Each row records:

- cognitive complexity maximum and sum;
- cyclomatic complexity maximum and sum;
- count above the fixed diagnostic thresholds `cognitive > 15` and `cyclomatic > 10`;
- BCA PLOC;
- function/closure count;
- contributing file count;
- analyzer skip and error counts.

Declarative support contributes only file count and nonblank-line count.
PLOC and function count use each BCA file-root aggregate once.
Complexity sums, maxima, and threshold counts use each function/closure space's own `value` once.
BCA file-root `sum` also rolls up non-function containers and nested descendants, so P01a does not add that aggregate to the function-space values.
Nested functions are therefore measured once and never added again through a parent aggregate.

The thresholds are review diagnostics, not acceptance limits.
Changing the environment, analyzer/version/hash, scope, classification, thresholds, or aggregation requires an ADR amendment and corresponding fixture updates.

### Base/head comparison and bootstrap

For future pull requests, CI finds the merge base and obtains the quality CLI from that merge-base commit.
That merge-base-side CLI and scope measure both the base worktree and the head worktree.
A feature branch therefore cannot reduce its reported scope by editing the head CLI.

P01a is the one bootstrap exception because its merge base has no quality CLI.
CI detects the missing merge-base path, uses the head CLI for both sides, and records `cli_source: head-bootstrap` plus `bootstrap_exception: true` in both summaries and the job summary.
After P01a is merged into the integration branch, a missing merge-base CLI is a hard failure rather than an implicit fallback.

### Artifacts and presentation

Each side stores BCA's raw full and Rust-test-pruned JSON plus a normalized deterministic `summary.json`.
CI uploads separate base and head artifacts and a deterministic comparison JSON.
The comparison command writes the Markdown delta table to stdout, which CI appends directly to GitHub Job Summary.
No baseline, `summary.md`, SaaS result, or bot comment is committed or created.

Running a snapshot twice for the same revision, CLI source, and repository contents must produce byte-identical `summary.json`.
CI checks this directly.

### Test execution and gate boundary

CI runs the existing Cargo tests, Vitest unit project, and Storybook Chromium/browser project.
Their executed case counts remain observational `R_runner` under ADR 0005.
Story rendering smoke is never converted into contract fulfillment.

The quality job hard-fails only for tool installation/execution, reproducibility, classification/scope, JSON/metric parsing, analyzer skip/error, or artifact-production failures.
Raw complexity growth and diagnostic-threshold growth remain review-only.
There is no 60-second performance gate; a bounded workflow timeout may stop a stuck job.

Runtime CPU, RSS, callback latency, and accepted-action latency remain governed by ADR 0005 and deferred to P01c after the Engine/process prerequisite exists.

## Consequences

Every pull request gets comparable per-language, per-category static measurements without adding a product dependency or runtime behavior.
Raw analyzer data remains available for audit while the normalized summary stays deterministic.
The bootstrap is explicit and cannot silently persist.

P01b must still provide the contract/evidence inventory before `O`, `O_v`, `U`, logical `T`, or contract coverage can be claimed.
P01c must still implement the runtime measurement conditions and acceptance values already defined by ADR 0005.
