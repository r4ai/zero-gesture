# ADR 0016: Own a listen-only macOS event tap on one native Engine thread

- Status: Accepted
- Date: 2026-08-01

## Context

ADR 0015 gives macOS Engine mode a secure control plane and keeps the packaged
Engine free of a managed WebView. P04b2 needs the first native mouse-input
owner for Apple Silicon and current macOS without taking on context lookup,
gesture completion, input suppression, action execution, or rendering.

The Core Graphics callback can be disabled when it exceeds the system timeout
or because of user input. Input Monitoring permission may be absent on first
launch and CI cannot grant it interactively. None of those states may make
physical input slow, blocked, or dependent on Settings/control-plane work.

## Decision

### One concrete macOS leaf

`hook/macos.rs` owns `CGEventTap` and `CFRunLoop` on the existing dedicated
Engine input thread. It uses direct Core Graphics/Core Foundation FFI and adds
no dependency, platform trait, Linux adapter, event bus, actor, or public API.
Windows keeps its existing `WH_MOUSE_LL` owner and behavior.

The Engine calls `CGPreflightListenEventAccess` once at the native-owner startup
boundary. It never calls `CGRequestListenEventAccess`; permission UI belongs to
a later Settings phase. Permission denial, tap/source creation failure, or
initial enable failure selects a degraded owner that publishes pass-through
readiness and stays alive until the normal Engine stop signal.

### Pass-through and callback contract

The tap is created with `kCGEventTapOptionListenOnly`. P04b2 therefore cannot
suppress, replace, or inject input. The callback always returns the exact
event reference supplied by Core Graphics, including timeout/user-input
disable notifications and overload.

The callback may only:

- read the event type, location, timestamp, mouse button, and scroll delta
  already carried by the callback event;
- normalize those values to the existing `MouseEvent` and `Point` boundary;
- append one `Copy` value to a 64-entry bounded SPSC ring; and
- update relaxed/release atomic counters and flags.

It performs no allocation, lock, wait, blocking send, IPC/JSON, file I/O, log
formatting, process/window/context query, configuration read, action,
renderer, Tauri, or WebView call. Queue saturation increments one atomic drop
counter and passes the event. The producer publishes each initialized slot
with release ordering; the run-loop consumer reads it with acquire ordering,
so accepted callback order is retained.

P04b2 stops at this normalized Engine boundary. It does not call `InputKernel`
because there is no pre-resolved macOS context and no suppression/replay/action
lifecycle in this phase.

No existing cross-platform self-generated-event marker is available. P04b2
does not invent one; listen-only behavior makes every observed event safe to
pass. A later event-posting phase must define its marker together with the
injection contract.

### Disable, overload, and failure handling

`kCGEventTapDisabledByTimeout` and
`kCGEventTapDisabledByUserInput` callbacks set one coalescing atomic request.
The callback does not re-enable or retry. After `CFRunLoopRunInMode` returns,
the owner side performs at most one `CGEventTapEnable` attempt for that
request and records attempts/failures atomically. An unstable tap remains
fail-open; another Core Graphics disable notification is required for another
attempt.

The callback KPI set is deliberately bounded to received, accepted, dropped,
and disabled counters. Processed input, re-enable attempts, and re-enable
failures are updated on the owner side. There is no callback clock read or
histogram allocation.

### Lifecycle

Readiness is published after the startup permission/setup decision. Active
owners run Core Foundation in fixed 10 ms slices, drain accepted normalized
events, handle one coalesced re-enable request, and observe the Engine stop
atomic. Degraded owners observe the same stop atomic at the same bound.

Shutdown sets the stop atomic before Windows-specific wakeup handling. The
macOS owner drains accepted events, disables and invalidates the tap, removes
and releases its run-loop source, releases the tap, and joins before Engine
teardown completes. The existing configuration owner and UDS worker are not
restarted or synchronized from the callback.

## Verification

`contracts/p04b2-macos-event-tap-owner.json` maps seven independently
falsifiable obligations to seven uniquely named macOS tests:
`O = 7`, `O_v = 7`, `U = 0`, `T = 7`, `T_u = 6`, `T_i = 1`, `T_e = 0`,
`T_r = 0`, `P = 0`, `D = 0`, and `F = 0`.

The Apple Silicon macOS CI job compiles every target with Clippy, runs the
library/contract tests without requiring real permission or injected events,
builds the ad-hoc signed application, and reruns the existing packaged
same-executable/UDS gates. Windows format, Clippy, tests, rustdoc, contracts,
frontend checks, and Tauri build remain regression gates.

## Deferrals

P04b2 does not add:

- Listen Event/Input Monitoring permission request UI;
- macOS window/process context or Accessibility APIs;
- `InputKernel` gesture completion or physical-input suppression/replay;
- keyboard/mouse action posting or a self-generated-event marker;
- AppKit/Core Animation rendering or status UI changes;
- login items, updater changes, Developer ID credentials, or notarization
  evidence; or
- final P06 latency/RSS acceptance.

## Consequences

Engine mode now owns the real macOS input observation boundary while preserving
physical input under permission loss, setup failure, callback disablement, and
load. Later context/suppression work can consume the closed normalized record
without expanding this phase into a generic platform framework.
