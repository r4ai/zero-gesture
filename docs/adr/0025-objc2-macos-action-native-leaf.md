# ADR 0025: Isolate the macOS action native leaf and use objc2 event ownership

- Status: Accepted
- Date: 2026-08-08
- Reviewed base: `06cc5dbec49c49e601c7323d15d2736c60201b1e`

## Context

The P04b3b action executor combined bounded worker/control policy, virtual-key
mapping, handwritten Core Graphics declarations, manual `NonNull`/`CFRelease`
ownership, event tagging, and posting in one file. ADR 0022 selected objc2 for
maintained Apple framework bindings, but P04R3 must migrate that implementation
without changing action selection, queueing, completion order, failure
classification, shutdown, or Windows behavior.

The migration also exposed an inherited constant error. The handwritten reader
and writer both used numeric event field `55`; the generated
`CGEventField::EventSourceUserData` is `42`. Preserving reader/writer parity at
the wrong field would continue to hide the error, while changing only the writer
would temporarily break self-event filtering.

## Decision

### Module ownership

`executor::macos` remains one private deep module behind the existing
`MacosActionExecutor` interface:

- `macos/mod.rs` owns the eight-entry command/result FIFOs, worker lifecycle,
  KPI accounting, repeat outcome classification, and bounded shutdown;
- `macos/keymap.rs` owns the closed unambiguous macOS virtual-key mapping; and
- macOS-only `macos/native.rs` owns Core Graphics event creation, generated
  ownership, marker tagging, and session-tap posting.

The split adds no public interface, trait, backend hierarchy, retry, or generic
platform adapter. The only internal seam accepts resolved key codes and one
marker for one repeat. Core Graphics types do not cross into the worker/control
module.

### Generated bindings and ownership

The native leaf uses generated objc2 APIs and types:

- `CGEvent::new_keyboard_event` returns `Option<CFRetained<CGEvent>>`;
- `CGEvent::set_integer_value_field` tags each created event with
  `CGEventField::EventSourceUserData`;
- `CGEvent::post` posts the completed batch at
  `CGEventTapLocation::SessionEventTap`; and
- `CFRetained` releases every created event exactly once when the batch leaves
  scope, including a partially created batch that is rejected before posting.

All events for a repeat are created and tagged before any event is posted.
Key-down order and reverse key-up order are unchanged. A nullable Create result
still posts none of that repeat and is classified before or after injection by
the unchanged worker/control policy.

The deterministic native function-pointer seam uses `extern "C-unwind"` for
creation, integer tagging, and posting in both production and test doubles.
There is no handwritten Apple declaration, raw event pointer owner, explicit
retain/release, or autorelease pool in the action module.

The existing direct objc2 dependency feature sets already contain the minimum
features required by this leaf: Core Graphics `CGEvent`, `CGEventTypes`, and
`CGRemoteOperation`, plus Core Foundation `CFBase` for `CFRetained`. P04R3 adds
no feature or dependency.

### Atomic self-event field integration

P04R2 preserved the pre-migration reader/writer parity with one provisional
typed field `CGEventField(55)`. P04R3 removes that project numeric constant and
its callback import, then changes both sides atomically:

- the action writer tags with `CGEventField::EventSourceUserData`; and
- the callback reader reads `CGEventField::EventSourceUserData`.

The focused callback test creates and tags a real CGEvent with the named field
before invoking the production callback. The native writer test records the
same generated field on every event before posting. The final shared seam is
therefore the generated Core Graphics constant itself, not another
project-owned number exposed by the executor.

### Preserved behavior and failure contract

P04R3 does not change the P04b3b external behavior:

- the process-instance `i64` marker remains exact and is never persisted or
  logged;
- accepted actions remain FIFO, activation stays before injection, and the
  existing completion drives `AfterInjection`;
- command/results capacity remains eight, dispatch remains nonblocking, and
  shutdown waits at most 100 ms before detaching an in-flight OS call;
- post-event access uses prompt-free preflight;
- zero repeat, unsupported keys, permission denial, first-repeat nullable
  creation, overload, disconnect, and worker loss fail before injection;
- a later-repeat creation failure remains failed after injection; and
- F21-F24 and all previously deferred action kinds remain unsupported.

The only Event Tap callback change is the atomic field replacement above. All
other callback work and every Windows source remain unchanged.

## Contract and KPI record

P04R3 adds no external behavior and no manifest. The existing P04b3b manifest
still maps all 17 independent obligations once (`O = 17`, `O_v = 17`, `U = 0`,
`T = 17`, `T_r = 0`, `T_u = 17`, `T_i = 0`, `T_e = 0`, `P = 0`, `D = 0`, and
`F = 0`). Evidence paths move from the former monolith to `mod.rs` or
`native.rs`. The two source-policy tests are support checks and are not added
to `O` or `T`.

The migration-local comparison fixes the production portion of
`executor/macos.rs` at base `06cc5dbe...` against the production portions of
`executor/macos/{mod,native,keymap}.rs`. `#[cfg(test)]` items and generated
dependency sources are excluded. Physical/nonblank lines, function
declarations, exact `unsafe` tokens, and raw extern blocks are counted with the
same line/token rules on both sides.

| Migrated action scope | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Production physical lines | 408 | 403 | -5 |
| Production nonblank lines | 376 | 369 | -7 |
| Production functions | 20 | 19 | -1 |
| Production `unsafe` tokens | 13 | 0 | -13 |
| Raw Apple extern blocks | 2 | 0 | -2 |
| Handwritten Apple function declarations | 5 | 0 | -5 |
| Test physical lines | 351 | 350 | -1 |
| Test functions | 19 | 16 | -3 |
| P04b3b logical cases | 17 | 17 | 0 |

The repository-wide canonical cognitive/cyclomatic maxima and sums, function
counts, and PLOC remain the macOS/quality CI measurement defined by ADR 0006.
This Windows implementation environment does not contain the pinned
`big-code-analysis-cli 2.0.0`, so no canonical complexity value is fabricated.
The worker/control branches and semantic state machine are unchanged; the
native leaf replaces raw ownership paths with generated ownership rather than
moving them into tests or configuration.

## Verification and limits

Windows-host checks validate formatting, all host tests, Clippy, dependency
target scoping/default-feature policy, contract evidence registration, and
Windows source preservation. A Windows-host Apple Silicon `cargo check` with
the objc2 docs.rs build configuration validates the generated types, native
leaf, and macOS-only tests without linking or running them.

The authoritative macOS 26 arm64 job must run Clippy and all library/contract
tests, including the real-CGEvent ownership doubles, then build the ad-hoc
signed bundle and rerun packaged process/UDS acceptance. Noninteractive CI
still cannot prove a live Post Event TCC grant, actual delivery to another
application, or resistance to an external process copying the marker. Those
remain physical/manual acceptance.

## Consequences

The bounded executor policy is smaller, key mapping is local, and Apple event
ownership is represented by maintained generated types. P04R3 removes all
action-module handwritten Apple declarations and local unsafe code while
retaining the established worker and failure contract. Both reader and writer
now derive the self-event field from the same generated Core Graphics constant.
