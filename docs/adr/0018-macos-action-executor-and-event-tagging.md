# ADR 0018: Post tagged macOS keyboard actions from one bounded worker

- Status: Accepted
- Date: 2026-08-01

## Context

ADR 0016 stops the listen-only Event Tap callback at a fixed SPSC queue.
ADR 0017 supplies a bounded Accessibility context worker but deliberately
keeps it idle until an action-selection consumer exists. P04b3b introduces
that consumer and the first macOS action executor without taking on physical
input suppression, replay, rendering, or permission-request UI.

Posting an event makes it visible to taps at the posting location. Without a
process marker, a later mouse-posting path could feed its own event back into
gesture recognition. Action creation and posting may also allocate or enter
OS code, so neither belongs in the Event Tap callback or the run-loop input
decision.

## Decision

### Threading and ownership

The existing Event Tap thread remains the native input owner. Its callback
only reads `kCGEventSourceUserData`, compares one `i64` marker, and otherwise
retains the ADR 0016 raw-field normalization, 64-entry SPSC enqueue, and
atomic KPI work. A matching marker returns the original event immediately
without queue or input accounting. A different marker follows the physical
input path.

The run-loop consumer, after callback return, owns the existing
`NativeInputOwner` and `InputKernel`. It starts the P04b3a `ContextWorker`,
submits only mouse movement and button-down observations, reads the exact
point/freshness snapshot, and supplies that snapshot to the existing action
selection. It never waits for context. Button-down remains immediate,
mouse movement remains limited to one request per 25 ms, and button-up,
wheel, and other events do not create context work.

One concrete `macos-action` worker owns Core Graphics event creation and
posting. The run-loop sends the existing `Action` value plus session and
repeat count through an eight-entry bounded FIFO with `try_send`. Results use
one eight-entry bounded FIFO. The portable `InputKernel` still owns activation
before dispatch, one accepted-action completion slot, generation pinning, and
action order. No public API, transport trait, backend hierarchy, actor, retry
loop, or new crate is added.

The Event Tap remains `kCGEventTapOptionListenOnly`. The run-loop ignores the
kernel's suppression disposition, drains renderer effects without creating
an overlay, and discards replay work because the physical trigger was never
suppressed. Input suppression, actual trigger replay, and rendering are
P04b3c.

### Context consumer

Context is started only because the same production run-loop now uses its
binding set and target token to select an action. Unknown, stale, wrong-point,
wrong-generation, denied, timed-out, malformed, overloaded, or stopped
context cannot start a gesture and therefore cannot dispatch an action.

The first `ActivateTarget` work validates the same fresh cached target and
then returns Ready; it does not perform a second AX query or change focus.
This preserves activation-before-action ordering while the listen-only phase
posts to the currently focused application. Re-resolving the target at an
active suppression boundary and changing focus are deferred to P04b3c. A
focus change inside the accepted 100 ms cache window is therefore a remaining
manual risk in this phase.

### Process-instance marker

Before tap installation, the native owner fills one 64-bit marker with
`arc4random_buf`. Zero is replaced by one. The value is copied into the
stable callback context and into the action worker. Every generated event is
assigned that value through `CGEventSetIntegerValueField` with
`kCGEventSourceUserData` before `CGEventPost`.

The marker is correlation data, not authentication. A coincidental 64-bit
collision or an external process deliberately copying the value causes that
event to be ignored. Another value always passes. A restart creates a new
marker, so events tagged by an earlier process instance are not treated as
self-generated. The marker is never persisted or logged.

### Supported action and event ownership

P04b3b supports the existing `Action::Keyboard` model with these unambiguous
macOS virtual-key names:

- `command`, `option`, `ctrl`, and `shift`;
- `a` through `z` and `0` through `9`;
- left, right, up, down, tab, enter, escape, backspace, delete, home, end,
  page-up, page-down, and space; and
- F1 through F20.

The worker resolves every key before creating an event. It creates all
key-down events in configured order and all key-up events in reverse order.
Every event for one repeat must be non-NULL and tagged before any event in
that repeat is posted. Created `CGEventRef` values are held by one `NonNull`
owner and released exactly once with `CFRelease`; NULL is never released.
This leaf uses Core Foundation ownership only and creates no autoreleased
Objective-C object.

F21 through F24 have no unambiguous key-code mapping in the selected leaf and
fail before injection. Mouse actions and trigger replay require
`CGEventCreateMouseEvent` and remain P04b3c. Text/Unicode entry, media keys,
layout translation, a generic key backend, and accessibility-based app
activation are not inferred.

### Failure and shutdown semantics

`CGPreflightPostEventAccess` is checked on the worker before generation and
never requests a prompt. Permission denial, unsupported key, zero repeat,
first-repeat NULL generation, a full/disconnected command FIFO, or a stopped
worker closes the accepted action as failed before injection. Because the tap
is listen-only, any resulting replay cleanup is discarded and physical input
has already passed.

If a later repeat cannot be generated after an earlier repeat was posted, the
result is failed after injection and no replay is attempted. `CGEventPost`
returns no status, so a successful call after positive preflight is the
strongest programmatic boundary; actual delivery cannot be claimed.

Shutdown first makes the tap pass-through by disabling and releasing its
resources. It then stops the owner and workers. The action worker has a 100 ms
completion wait. If an in-flight OS call has not returned, its thread is
detached and releases its command, channels, and generated events when the
call eventually returns. User-space Rust cannot kill that call, but it cannot
hold Engine input teardown past the fixed wait.

Worker logs contain only queued, dropped, posted-call, failed-before, and
failed-after counters. The callback keeps only the bounded ADR 0016 counters.
No input value, key sequence, title, process name, bundle identifier, path,
marker, or configuration body is logged.

Post Event and Accessibility access are TCC runtime capabilities. P04b3b adds
no entitlement file and does not invent an App Sandbox or Hardened Runtime
exception.

## Verification

`contracts/p04b3b-macos-action-executor.json` maps sixteen independent
obligations to sixteen uniquely named tests: `O = 16`, `O_v = 16`, `U = 0`,
`T = 16`, `T_u = 16`, `T_i = 0`, `T_e = 0`, `T_r = 0`, `P = 0`, `D = 0`,
and `F = 0`.

The deterministic core and concrete function-pointer seam verify self-marker
filtering without allocation, different-marker passage, marker copy order,
tagging of every generated event, keyboard ordering, permission and NULL
failure, before/after-injection classification, bounded FIFO overload and
FIFO order, worker stop, bounded shutdown, production context connection,
request filtering, fresh selection, and unknown/stale rejection.

The Apple Silicon macOS job compiles and lints every target, runs the same
tests without interactive permission or actual input injection, builds the
ad-hoc signed bundle, and reruns the packaged same-executable/UDS gates.
Windows and frontend gates remain unchanged.

Noninteractive CI does not prove that TCC grants Post Event or Accessibility
access, that a real `CGEventPost` is delivered to another application, or
that an external event cannot intentionally forge the marker. Those remain
manual integration evidence and are not inferred from seam tests.

## Apple API basis

- [`kCGEventSourceUserData`](https://developer.apple.com/documentation/coregraphics/cgeventfield/eventsourceuserdata)
  is a per-event 64-bit user field and is accessed by
  [`CGEventGetIntegerValueField`](https://developer.apple.com/documentation/coregraphics/1455885-cgeventgetintegervaluefield)
  and
  [`CGEventSetIntegerValueField`](https://developer.apple.com/documentation/coregraphics/cgevent/setintegervaluefield(_:value:)).
- [`CGEventCreateKeyboardEvent`](https://developer.apple.com/documentation/coregraphics/cgevent/init(keyboardeventsource:virtualkey:keydown:))
  and
  [`CGEventCreateMouseEvent`](https://developer.apple.com/documentation/coregraphics/cgevent/init(mouseeventsource:mousetype:mousecursorposition:mousebutton:))
  return NULL on failure and transfer one created reference that must be
  released.
- [`CGEventPost`](https://developer.apple.com/documentation/coregraphics/cgevent/post(tap:))
  inserts an event before taps at the selected location.
- [`CFRelease`](https://developer.apple.com/documentation/corefoundation/cfrelease)
  must receive a non-NULL owned reference.

## Deferrals

P04b3c owns active Event Tap suppression, mouse trigger replay, target
revalidation/activation, and native overlay/renderer integration. P05 owns
permission explanation/request UI. Login items, updater behavior, Developer
ID credentials, notarization evidence, and final P06 latency/RSS acceptance
remain unchanged.

## Consequences

macOS now reuses the portable selection and ordering semantics and can post a
bounded subset of existing keyboard actions without expanding the callback.
Every generated event is ready for self-filtering, while permission loss,
context loss, overload, generation failure, and worker failure preserve
physical input.
