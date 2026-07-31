# ADR 0013: Wire the Windows native input owner to InputKernel and two-slot snapshots

- Status: Accepted
- Date: 2026-07-31

## Context

ADR 0010 proves the bounded `InputKernel` policy but leaves the Windows hook on
the older `GestureMachine` path. ADR 0012 proves Engine-owned immutable config
publication but temporarily projects every committed config by stopping and
recreating the Hook/Overlay worker pair.

That compatibility path duplicates recognition ownership, performs window and
process queries inside `WH_MOUSE_LL`, uses allocating/unbounded callback-side
queues, and can mix worker lifecycle with an otherwise non-fallible published
commit. P03c must make the real Windows callback use the already proven
kernel/publication interfaces while preserving Windows gesture, suppression,
replay, rendering, and action ordering.

## Decision

### One native input owner

`hook/owner.rs` is the native input-owner module. Its external interface is one
normalized callback transition plus owner-side context, action-result,
renderer-result, timer, and shutdown inputs. The actual
`low_level_mouse_proc` decodes `MSLLHOOKSTRUCT`, calls that interface, posts
bounded work wakeups, and returns the kernel disposition.

Engine clones `ConfigOwner`'s `ConfigSnapshotReader` after first-pipe readiness
and config startup, then starts one native input-owner thread. The thread stays
alive while config is unavailable, disabled, enabled, or replaced; unavailable
and disabled snapshots pass input. `Applied` observes that the owner is still
alive and schedules the tray label, but never restarts the hook. The P03b
Hook/Overlay compatibility recreation is removed.

This is one deep module at the OS callback seam. There is no platform trait,
actor framework, runtime, RPC, event bus, or second gesture state machine.

### Callback contract

The low-level callback may:

- read the point, mouse data, flags, and event tick already supplied by
  `MSLLHOOKSTRUCT`;
- perform the fixed atomic operations in the P03b two-slot reader;
- clone an already allocated `Arc` snapshot;
- run fixed-capacity `InputKernel` state transitions;
- insert numeric work into fixed-capacity lanes; and
- post a nonblocking `WM_APP` wakeup to its known owner-thread ID.

It performs no heap allocation, lock acquisition, waiting/blocking send,
IPC/JSON, logging, file access, process/window/OS query, thread creation,
renderer call, action execution, or Tauri/WebView call. It does not call
`GetTickCount`; the native event tick is the canonical callback time.

`contracts/p03c-windows-input-owner.json` exercises the actual callback-facing
module. A thread-local counting allocator proves zero allocations for a real
start transition through snapshot selection, `InputKernel`, and both lanes.
The absence of the other forbidden operations follows from the closed module
interface: those operations exist only in message-loop/context functions that
cannot be called by `NativeInputOwner::callback`.

### Snapshot and generation lifetime

An idle owner reads the immutable `RuntimeConfig` through
`ConfigSnapshotReader`. A successful reader guard clones that slot's existing
`Arc<RuntimeConfig>` and then releases the reader count. The owner publishes
the snapshot's `Arc<GestureConfig>` and exact generation to `InputKernel`.

When a gesture starts, the owner pins the whole `RuntimeConfig` for the
kernel's reported generation. It retains that snapshot through recognition,
activation, accepted-action completion, and pending replay. It releases the
pin only when `InputKernel::pinned_generation()` becomes empty because the
session completed, cancelled, replayed, or entered bypass. Publication during
that interval cannot change action, label, appearance, recognition thresholds,
or safety timeout for the active gesture. The next idle callback reads the
later generation.

There is no retired-generation table. The one active kernel session needs at
most one pinned runtime, and `Arc` lifetime is sufficient after the proven
slot guard has selected a complete immutable value.

### Pre-resolved Windows context

One context worker performs `GetCursorPos`, `WindowFromPoint`, top-level-window
resolution, window/process inspection, application matching, and snapshot
generation selection outside the callback. It overwrites one atomic
latest-value mailbox. A sequence counter prevents mixed fields; the
owner-thread message loop copies only a complete observation into its local
owner state.

The worker samples every 4 ms and republishes at least every 25 ms. A trigger
may start only when the cached point exactly equals the event point, the cache
age is at most 100 ms under wrapping ticks, and the cached generation equals
the currently selected snapshot. Missing, stale, wrong-generation, disabled,
or unavailable context passes the physical event without starting. These
constants are conservative correctness bounds, not a final P06 latency
acceptance claim.

The context contains a dense `BindingSetId` and opaque HWND-derived
`TargetToken`. Target activation occurs later in the action lane. The callback
never discovers or activates a window.

### Distinct bounded lanes and outcomes

The callback owns two independent fixed-capacity FIFO lanes:

- action lane: 16 numeric entries for activation, accepted action, and trigger
  replay;
- renderer lane: 64 numeric entries for start, point, label identity, and end.

A start reserves action capacity for activation and the possible terminal
replay before suppressing the trigger. Accepted action delivery uses the
already retained replay reservation. Exhaustion passes before suppression.
Activation and accepted actions share one FIFO, so `ActivateTarget` is
observed before `DispatchAction`. Action entries carry session, generation,
and dense action identity; the message loop resolves owned action data only
from the pinned runtime.
Each non-empty transition posts a `WM_APP` wakeup. The existing 100 ms safety
timer also drains both lanes, so a transient native message-post failure cannot
strand bounded work indefinitely.

The renderer lane reserves one terminal slot from start through end. Points and
labels are lossy when only that terminal slot remains. The existing overlay
bridge is also bounded at 64 entries; point/label sends use nonblocking
drop-on-full, while start/end full or disconnect is a renderer fault. A
renderer fault is fed back to `InputKernel`, which follows its replay/cancel
and bypass policy without delaying physical input.

The action message loop reports activation ready/failed, injection started,
completion, before-injection failure, and after-injection failure through
distinct kernel inputs. `executor::execute` returns whether every planned
`SendInput` event was accepted. Partial or zero injection after
`InjectionStarted` is an after-injection failure and never replays. Replays
use the captured suppressed down and physical-up coordinates exactly once.

### Renderer and owner lifecycle

The overlay renderer is lazy and owned by the hook message loop. `RenderStart`
starts it for the pinned generation; a later generation replaces it only when
the prior gesture has ended and the next start arrives. Point overload does
not delay action delivery. Lifecycle failure enters the kernel fault path
rather than continuing a headless gesture.

Engine startup order remains singleton/secret/first pipe, config startup and
publication, then input owner. Shutdown posts `WM_QUIT`, unhooks input, stops
and joins the context worker, puts the kernel in idempotent bypass, clears both
lanes and the generation pin, and stops the renderer. A worker panic is a
typed fatal owner failure. Once the hook is removed or the kernel is shut
down, later physical input is never suppressed; ordinary tray, Settings, IPC,
JSON, and WebView failures do not enter the callback.

## Contract and complexity accounting

The P03c manifest has 12 independent obligations and 12 unique runnable cases:
`O = 12`, `O_v = 12`, `U = 0`. No evidence pair appears in P02, P03a, P03b,
or more than once in P03c. Logical contract cases are `T = 12`, `T_r = 0`,
`P = 0`, `D = 0`, and `F = 0`.

No dependency is added. Fixed lanes and one latest-value mailbox replace the
callback-side `VecDeque`, direct overlay send, synchronous OS context queries,
and config-change hook restart.

## Deferrals

P03c does not add macOS CGEventTap/UDS/AppKit adapters, redesign Settings,
change updater/distribution, claim final RSS or latency acceptance, build a
packaging matrix, or add a generic cross-platform actor/runtime/RPC/plugin
framework. Those remain P04, P05, and P06 work under the current phase plan.

## Consequences

Windows input policy now has one canonical implementation and one actual
native owner. A fresh config can become visible without restarting the hook,
while an active gesture cannot observe mixed generations. Overload is bounded:
input/action capacity fails open, rendering degrades independently, and slow
management-plane work cannot make the callback heavy.
