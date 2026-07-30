# ADR 0010: Add a bounded platform-neutral Input owner kernel

- Status: Accepted
- Date: 2026-07-30

## Context

ADR 0002 fixes the Input owner policy, but P02a deliberately retained owned
strings, vectors, and actions in the gesture callback interface.
P02c needs an executable policy seam before the Windows owner and transport
adapters exist.

## Decision

`domain/input.rs` contains one `InputKernel`.
Its external seam is construction, generation publication, and
`handle(InputEvent) -> InputDecision`.
The input envelope contains only canonical pointer/control input, pre-resolved
context validity, numeric binding/target identity, and concrete nonblocking
reservation outcomes.
The decision contains one disposition, one mode, and at most three ordered
numeric effects.

`InputKernel` owns the mutable recognition, activation, accepted-action,
physical-up, replay-cleanup, and bypass state.
It emits `ActivateTarget(SessionId, TargetToken)` before any action for that
session and accepts an action only after same-session `Ready`.
The accepted-action record has one slot and the monotonic internal phases
`PendingBeforeInjection` and `InjectionStarted`; terminal input closes it as
`Completed`, `FailedBeforeInjection`, or `FailedAfterInjection`.
Before-injection failure replays the captured trigger once after suppressing
its matching physical up.
After-injection failure never replays and enters bypass.
Shutdown is idempotent and also enters bypass.

Config compilation assigns dense `BindingSetId` and `ActionId` values once.
`ActionId` identifies the precompiled action and its label.
Gesture recognition uses one eight-step fixed array and decisions use fixed
arrays, so representative `handle` paths allocate no heap memory.
The allocation contract is checked with a test-only thread-local counting
global allocator; counting is enabled only around the representative kernel
calls, so parallel tests on other threads do not affect the result.

`publish_config` changes the view and `ConfigGeneration` used by the next
gesture.
An active gesture retains its immutable compiled view and generation until its
completion/replay record closes.
The future Config owner remains responsible for generation delivery and
resource retention; this kernel adds no handshake or retired-generation table.

The current Windows hook remains a direct `GestureMachine` caller.
It consumes numeric compiled identities to preserve behavior but is not wired
to `InputKernel` in P02c.

## Deferrals

P02c does not add owner loops, channels, reservation transport, native context
sampling, Windows hook integration, renderer or executor adapters, process
modes, IPC, config-owner persistence handshakes, macOS code, Tauri lifecycle,
runtime telemetry, or update behavior.

## Contract accounting

The P02 manifest keeps its 36 existing Windows/config obligations and adds 14
independent Input kernel obligations.
For this bounded manifest, `O = 50`, `O_v = 50`, and `U = 0`.
All 50 entries name one runnable Rust test.

## Consequences

P03 and later adapters can reserve concrete capacity, call one pure kernel
transition, and apply its ordered fixed-size effects without putting policy in
the native callback.
No trait framework, actor runtime, queue abstraction, journal, retry protocol,
or additional crate is introduced.
