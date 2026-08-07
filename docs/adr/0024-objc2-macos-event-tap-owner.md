# ADR 0024: Use generated ownership for the macOS Event Tap owner

- Status: Accepted
- Date: 2026-08-08
- Reviewed base: `6e56ce5a66e5f0852436565e7aada3f109994c49`

## Context

The P04b2/P04b3b input owner kept the callback bounded and fail open, but one
`hook/macos.rs` file also declared and manually owned Core Graphics Event Tap,
Core Foundation Mach-port source, and run-loop objects. ADR 0022 selected
objc2 generated framework bindings and scheduled P04R2 to replace those
declarations without implementing active suppression or changing the existing
input, context, action, process, IPC, or Windows contracts.

## Decision

### Private module seams

`hook::macos` remains one private module with the same single caller:

- `mod.rs` owns the normalized input record, fixed SPSC queue, callback state,
  event normalization, and existing contract tests;
- `callback.rs` owns the Core Graphics callback ABI and reads only fields
  already present in the borrowed event;
- `run_loop.rs` owns tap installation, Mach-port/run-loop resources, degraded
  startup, re-enable requests, and deterministic teardown;
- `consumer.rs` owns run-loop-side context routing and the existing
  `NativeInputOwner`/action consumer; and
- `context/` remains the independent P04R1 context worker/native leaf.

Only `run_loop_macos` crosses to the parent hook module. The other seams are
private implementation details. No trait, adapter hierarchy, public symbol,
behavior mode, or implementation-detail manifest is added.

### Generated bindings and ownership

P04R2 enables only the additional Core Foundation features needed by the
production leaf: `CFDate`, `CFMachPort`, and `CFRunLoop`. It uses:

- `CGEvent::tap_create`, `tap_enable`, and `tap_is_enabled`;
- generated `CGEventTapLocation`, `CGEventTapPlacement`,
  `CGEventTapOptions`, `CGEventType`, and `CGEventField` wrappers;
- `CFRetained<CFMachPort>`, `CFRetained<CFRunLoopSource>`, and
  `CFRetained<CFRunLoop>`;
- `CFMachPort::new_run_loop_source` and `invalidate`; and
- `CFRunLoop::current`, `add_source`, `remove_source`, and `run_in_mode`.

`TapResources::drop` disables the tap, removes its source, and invalidates the
port before generated `CFRetained` drops release all owned objects. Nullable
tap, source, run-loop, or default-mode creation remains a bounded
`CreationFailed` startup and enters the existing pass-through degraded owner.

The callback uses the generated `CGEventTapCallBack` ABI:
`unsafe extern "C-unwind"` with a `NonNull<CGEvent>`. The only pointer
operations recover the stable boxed `TapState`, borrow the callback-duration
event, and return `event.as_ptr()` exactly. No event is retained, released,
replaced, or constructed in the callback.

### Callback and fail-open invariants

The callback remains synchronous, allocation-free, lock-free, and bounded. It
performs one self-marker field read, fixed event-field reads, normalization,
one fixed SPSC enqueue attempt, and relaxed/acquire-release atomic accounting.
It performs no blocking send, I/O, IPC/JSON, log formatting, WebView/Tauri
call, OS query, autorelease-pool work, or object ownership operation.

The tap remains `CGEventTapOptions::ListenOnly` and always returns the exact
input event pointer. Queue saturation drops only the new observation. Timeout
or user-input disable callbacks coalesce one owner-side re-enable request.
Permission loss, nullable setup, non-timeout run-loop return, and overload all
preserve physical input. Active suppression, replay, and rendering remain
deferred.

### Provisional source-user-data field and P04R3 boundary

The existing callback and action executor both use numeric field `55` for
their process marker. Generated
`CGEventField::EventSourceUserData` is `42`; `55` is **not** claimed to be the
Apple constant. Changing only the callback would break reader/writer parity in
this Event Tap-only phase.

P04R2 therefore moves `55` to one crate-private executor constant and has the
callback import that same fact. P04R3 must migrate the action writer and
callback reader atomically to generated
`CGEventField::EventSourceUserData`, then validate real self-event filtering.
The rollback boundary is the four private input files, three added Core
Foundation features, the shared provisional constant, and evidence-path-only
manifest edits. Reverting that set restores the prior raw owner without a
data, protocol, or Windows migration.

### Retained raw leaf

No handwritten Core Graphics or Core Foundation declaration remains in the
Event Tap owner. One `arc4random_buf` declaration remains for the existing
process-instance marker because it is a libSystem random function rather than
a Core Graphics/Core Foundation ownership interface. Its leaf passes one
initialized stack `u64` and its exact byte size; the pointer does not escape.

The generated callback necessarily retains a raw `user_info` pointer and raw
event-pointer return in its ABI. Re-wrapping those in an owned object or
generic callback trait would add lifetime and allocation risk to the hot path
without improving the generated contract.

## Contract and KPI record

P04R2 adds no external behavior and no manifest. All P04b2 obligations retain
one evidence mapping: `O = 8`, `O_v = 8`, `U = 0`, `T = 8`, `T_r = 0`, and
`O_v / O = 100%`. The inherited P04b3a and P04b3b evidence paths move with the
same tests; their 24 and 17 obligations remain unchanged. No logical test case
is added, removed, retried, or duplicated, so inherited `P = 0`, `D = 0`, and
`F = 0` remain unchanged.

The migration-local comparison fixes the production portion of old
`hook/macos.rs` against the production portions of
`hook/macos/{mod,callback,run_loop,consumer}.rs`. It counts rustfmt-formatted
physical/nonblank lines, function declarations, and exact source tokens;
dependency/generated sources are excluded.

| Migrated Event Tap scope | Before | After | Delta |
| --- | ---: | ---: | ---: |
| Production physical lines | 865 | 858 | -7 |
| Production nonblank lines | 788 | 781 | -7 |
| Production functions | 61 | 45 | -16 |
| Production `unsafe` tokens | 16 | 13 | -3 |
| All raw extern blocks | 3 | 1 | -2 |
| Raw Apple framework extern blocks | 2 | 0 | -2 |
| Handwritten Apple function declarations | 16 | 0 | -16 |
| Production files | 1 | 4 | +3 |
| Test physical lines | 786 | 750 | -36 |
| Test functions | 31 | 28 | -3 |
| Test `unsafe` tokens | 10 | 3 | -7 |
| `#[test]` attributes | 16 | 16 | 0 |

The file count increase localizes three independently changing ownership
reasons while preserving one external interface; no file is a pass-through.
The canonical cognitive/cyclomatic maxima and sums remain measured by ADR
0006's macOS CI quality comparison. `big-code-analysis-cli 2.0.0` is not
installed in this Windows implementation environment, so those values are not
fabricated.

## Verification and limits

Windows host tests and Clippy validate unchanged Windows semantics, portable
callback/queue behavior, evidence paths, and target-scoped dependency policy.
A Windows-host Apple Silicon cross-check with `DOCS_RS=1` validates generated
types and callback/ownership compilation while skipping the unavailable
Objective-C exception-helper native build.

The authoritative macOS 26 arm64 job must run Clippy, all library/contract
tests, application packaging, and packaged process/UDS acceptance.
Noninteractive CI still cannot prove a live TCC grant, physical
cross-application event delivery, disable/re-enable delivery, or actual
Mach-port invalidation timing. It also cannot validate the provisional marker
field against physical self-posted events; that is an explicit P04R3/manual
acceptance item.

## Consequences

Generated types now encode Event Tap, Mach-port, source, run-loop, callback,
and retain/release contracts behind the existing input-owner interface. The
hot path remains small and fail open, and active suppression remains out of
scope. P04R3 has one explicit atomic marker-field migration before it replaces
the action executor's remaining raw Core Graphics implementation.
