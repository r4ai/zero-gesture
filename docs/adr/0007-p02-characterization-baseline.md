# ADR 0007: Bound P01b to an executable P02 characterization baseline

- Status: Accepted
- Date: 2026-07-27

## Context

ADR 0005 defines the eventual product and architecture contracts, while ADR 0006 separates static measurement, contract evidence, and runtime measurement into P01a, P01b, and P01c.
The next implementation step, P02, moves current Windows gesture and configuration behavior into a platform-neutral core.
Attempting to build the eventual all-project contract inventory before that move would add a generic schema, gap states, ownership metadata, and evidence kinds that P02 does not consume.
Conversely, relying on the existing runner count or Storybook render smoke would not identify which current behaviors P02 must preserve.

P01b therefore needs a narrow executable characterization boundary.
It records observed current behavior that P02 can disturb; it does not promote every existing test or every future ADR clause into a contract framework.

## Decision

`contracts/p02-windows-baseline.json` is the one machine-readable source of truth for this baseline.
Each case has exactly five fields:

- a stable `P02-WIN-NNN` ID;
- one sentence describing the observed obligation;
- the `cargo-test` runner;
- the repository-relative Rust evidence file;
- the exact `#[test]` function name.

The manifest contains 15 one-to-one cases.
They characterize only:

- configured and unconfigured trigger-down suppression;
- movement pass-through during capture;
- release action versus replay behavior;
- short-click replay and long-travel no-replay behavior;
- hold-session continuation, repeated wheel input, multi-notch repetition, and no replay after a hold action request;
- direction confirmation and direction-switch transitions;
- valid current release/hold configuration decoding, app-specific binding precedence, and the disabled setting.

These are the current pure Rust behaviors directly at risk when P02 relocates recognition and valid v1 configuration semantics.
The selected evidence is already runnable in the existing Cargo test target, except for two adjacent characterization tests added for movement pass-through and the direction threshold boundary.
No production seam or runtime behavior is changed.

`tools/contracts/check.mjs` is deliberately specific to this manifest.
It uses only Node standard-library modules and rejects an unexpected shape, empty or malformed fields, duplicate IDs, obligations, or evidence pairs, a runner other than `cargo-test`, a missing or escaping Rust evidence path, and an evidence name that is not present as a `#[test]` function.
When the Rust Test job supplies `cargo test --all-targets -- --list` output, every evidence test must also appear there exactly once.
It is not a JSON Schema, a contract DSL, or a registry of future evidence types.
`contracts:check` runs the validator and its five direct failure tests.
The existing least-privilege quality job runs that command, while the existing Windows Rust test job runs every referenced test.
The path check does not resolve symlinks; repository-controlled input in no-secret, read-only CI makes that an accepted residual risk for this phase.

Within this deliberately bounded inventory, `O = 15`, `O_v = 15`, and `U = 0`.
There are 15 unit-level logical cases, so `T = T_u = 15` and `T_i = T_e = 0`.
These values describe only the P02 characterization boundary and must not be reported as whole-product contract coverage.

## Exclusions and deferrals

This baseline does not enumerate every normative statement linked by ADR 0005 and does not satisfy or replace the eventual all-project contract inventory.
It does not claim `R_runner`, Storybook rendering smoke, or unrelated existing tests as contract coverage.
Settings UI workflows, native overlay behavior, packaging, permissions, filesystem failure, injection failure, and unimplemented architecture are outside this P02 boundary.

The current Win32 callback performs target activation before calling the pure state transition, but there is no runnable seam that can verify activation-to-action ordering without changing production code.
P01b therefore makes no ordering claim.
The session-bound activation and action ordering required by ADR 0002 belongs with the Executor owner implementation and fault tests in P04.

The current state machine also has no accepted-action completion record and its safety timeout does not implement ADR 0002's captured-trigger failure rule.
The baseline characterizes the current observable release/replay and hold-action-request behavior only.
It does not mislabel future abort replay or completion semantics as current coverage.

The following remain explicitly deferred:

- dual-process Engine/Settings IPC and config ownership to P03;
- the macOS adapter and packaging evidence to P05 and P06;
- runtime CPU, RSS, callback latency, and action latency KPIs to P01c and later platform acceptance;
- future protocol compatibility and version-mismatch behavior to P03 and P07.

## Consequences

P02 gets a small, executable preservation gate without a new dependency or framework.
A manifest change must name a runnable Rust test and remains reviewable as a short list of observed obligations.
Later phases must add their own evidence at the layer where the behavior becomes implementable rather than extending this file with speculative cases.
