# ADR 0027: Own the macOS native overlay on the Tauri main thread

- Status: Accepted
- Date: 2026-08-30
- Reviewed base: `716c652`

## Context

P04b3c-a made macOS suppression, action dispatch, and replay real, but the
run-loop consumer still discarded the existing bounded `RenderWork` stream.
AppKit objects require the main thread. The resident Engine must render without
adding WebView memory, blocking the input callback, or changing the proven
Windows overlay.

## Decision

The macOS consumer translates the existing start, point, label, and end work
to one concrete `MacosOverlayClient`. The client owns a 64-entry bounded queue.
An accepted start reserves lifecycle capacity; point and label may be dropped
under load, while lifecycle delivery failure is fatal. A single atomic wake
gate coalesces `AppHandle::run_on_main_thread` scheduling and rechecks after
clearing the gate so concurrent work is not stranded.

The main-thread closure is only a scheduler boundary. AppKit and Core Animation
objects stay in private thread-local state in `overlay/macos/native.rs`. The
first accepted start lazily creates transparent, click-through, nonactivating
native windows. A `CAShapeLayer` retains at most 64 points, and `CATextLayer`
uses the pinned family, size, and a bounded nine-step weight mapping with an
explicit system-font fallback. No framework object crosses a queue.

No dedicated renderer thread, managed Tauri window, WebView, shared renderer
trait, actor framework, or dynamic dispatch is introduced. The Windows overlay
thread and renderer implementations are unchanged.

Screen-frame signature changes clear retained points before later rendering.
End clears retained trail and label state. Native resource or scheduling panic
is caught at the scheduling boundary and becomes a renderer fault.

Renderer failure first disables active input, then calls the existing
`renderer_terminated` kernel path. Shutdown drains the already-generated
Replay/Cancel work and must not call `shutdown_with_replay` again, because that
would clear the reserved replay lane. Overlay shutdown waits at most 100 ms.
The tray main-thread handler delegates Engine shutdown to a named background
thread so it never waits for its own main-thread overlay drain.

## Contract and KPI record

`contracts/p04b3c-b-macos-native-overlay.json` adds ten unique obligations:
`O = 10`, `O_v = 10`, `U = 0`, `T = 10`, `T_r = 0`, `T_u = 10`, `T_i = 0`,
`T_e = 0`, `P = 0`, `D = 0`, and `F = 0`. Together with the inherited 104
P04 obligations, the inventory is 114.

The hot callback adds zero locks, allocation, blocking sends, IPC, I/O,
logging, OS queries, native UI calls, or Tauri calls. All new queue/history
bounds are 64 and shutdown acknowledgement is 100 ms. Canonical production
and test cognitive/cyclomatic max and sum, formatted line counts, and function
counts use ADR 0006's fixed base/head quality workflow; no substitute local
metric is invented in this ADR.

## Verification and limits

Windows formatting, Clippy, tests, and contract checks are required to prove
the macOS-only addition does not regress the supported Windows implementation.
Apple Silicon CI is the authoritative compilation check for objc2 APIs.

The focused tests prove queue policy, wake coalescing, pure window/trail/label
specification, retained-state transitions, failure replay preservation, and
main-thread non-waiting. They do not prove live AppKit presentation,
click-through, Spaces/full-screen behavior, mixed-display coordinates, or TCC
event delivery. Those remain P06m physical acceptance and are not blockers for
this implementation phase.

## Consequences

macOS now consumes the portable render stream with finite memory and a narrow
main-thread native leaf. Visual overload degrades visual detail rather than
input responsiveness, while lifecycle failure preserves fail-open replay and
terminates the faulty owner.
