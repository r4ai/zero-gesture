# ADR 0008: Own gesture recognition and session decisions in one portable module

- Status: Accepted
- Date: 2026-07-27

## Context

P02a moves the current pure Windows gesture behavior without introducing the
future Engine owners, config schema, IPC, or macOS adapter.
The previous implementation split one decision path across `gesture.rs`,
`hook/state.rs`, pure button logic in `hook/trigger.rs`, and Win32 effect
application.
Keeping either the old state machine or a compatibility facade beside a new
core would create two canonical implementations.

## Decision

The existing Tauri crate contains one portable gesture module:

```text
src-tauri/src/domain/
  mod.rs          # crate-local interface
  recognition.rs  # private movement recognizer
  session.rs      # owned config, bindings, session state, and decisions
```

`GestureMachine` owns the immutable `GestureConfig` and the only mutable
recognition/session state.
Its interface is limited to construction, `can_start(trigger)` for the
existing Windows pre-start context path, and
`handle(GestureInput) -> Decision`.
`GestureInput` is either a normalized pointer event with point, wrapping
monotonic tick, and optional matched application ID, or a safety-timer tick.

Each `Decision` contains:

- exactly one `Disposition::Pass | Suppress`;
- exactly one closed `GestureTransition`:
  `Continue`, `ContinueWithAction`, `Complete`, `FinishWithAction`, `Replay`,
  or `Cancel`;
- zero to two closed `RenderEffect` values.

Action and replay are transition variants rather than independent optional
effects, so one input cannot select conflicting session outcomes.
The module performs no OS calls, thread or channel operations, blocking,
Tauri ownership, or logging.

`hook/mod.rs` continues to validate and compile the current config snapshot.
Windows application matching remains in `hook/app_match.rs`.
`hook/win32.rs` continues to own Win32 message decoding, injected-event
filtering, target activation and window lookup, application context
resolution, overlay-channel application, deferred action execution, replay
injection, and mouse flag conversion.
It activates and resolves the target before handing a configured trigger to
`GestureMachine`, then applies render effects and queues the selected
action/replay after the decision, preserving the existing causal order.

> P03c update: ADR 0013 supersedes the Windows adapter ownership described
> above. Config compilation now belongs to `config`, application/window
> resolution runs in the context worker, and the real callback uses
> `InputKernel`; `hook/app_match.rs` and the callback-side OS queries were
> removed.

The legacy `src-tauri/src/gesture.rs`, `src-tauri/src/hook/state.rs`, and
`src-tauri/src/hook/trigger.rs` implementations are deleted in this change.
Their non-platform tests move to the portable module interface; Windows code
does not retain a compatibility wrapper or a parallel state machine.

## Deferrals

This decision does not implement:

- schema v2, legacy migration, config transaction ownership, or config IPC;
- bounded owner lanes, dual process modes, or runtime KPI instrumentation;
- accepted-action completion records or the future fail-open owner protocol;
- a macOS adapter, macOS input behavior, or platform trait hierarchy;
- settings or frontend changes.

Those remain assigned to P02b and the later phases recorded by ADR 0005.

## Consequences

Windows remains the only caller, but gesture behavior is now exercised through
one portable, owned interface.
Adding a second crate, DTO layer, actor framework, platform trait, or
future-adapter interface is unnecessary for this relocation.
