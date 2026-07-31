# ADR 0017: Preflight Accessibility and resolve macOS context on one bounded worker

- Status: Accepted
- Date: 2026-08-01

## Context

ADR 0016 installs a listen-only macOS Event Tap whose callback ends at a
64-entry normalized SPSC queue. P04b3a needs the process/window facts required
by the existing compiled application and `ContextView` contracts, but it must
not add input suppression, action injection, rendering, or permission UI.

Accessibility calls may block, fail because trust is absent, race a target
exit, or return malformed values. A process identifier can also be reused.
None of those conditions may delay the Event Tap callback or cause physical
input to stop passing through.

## Decision

### Capability boundary

The Engine calls `AXIsProcessTrustedWithOptions(NULL)` once on the macOS
context worker. Apple's contract defines `NULL` as no options; the Engine
therefore never supplies `kAXTrustedCheckOptionPrompt` and cannot request the
Accessibility prompt. A denied preflight leaves the resolver alive but
publishes Unknown for every request.

The explicit Settings action that may request trust, explain System Settings,
or open the relevant pane remains P05. Accessibility trust is a TCC runtime
capability, not an entitlement invented by this phase. Input Monitoring
preflight and the listen-only degraded owner remain the separate ADR 0016
boundary.

### Threading and OS leaf

`hook/macos_context.rs` is one concrete private macOS leaf. It adds no public
API, platform trait, generic context model, observer graph, actor runtime, or
crate.

The leaf reads `NSWorkspace.shared.frontmostApplication` and immutable
`NSRunningApplication` properties on the dedicated context worker. Apple
documents the shared workspace as safe to access from any thread and running
application properties as atomic. The worker installs its own autorelease
pool. It does not use AppKit view, window, responder, drawing, or event APIs,
so no Tauri main-thread hop is required.

The native observation contains only:

- executable process name for the existing Shared `process_name` selector;
- bundle identifier for the macOS `bundle_identifier` selector;
- focused AX window title for the existing Shared `title` selector;
- PID plus `proc_bsdinfo` start time and the AX window hash for private
  process/window identity.

`window_class` remains Windows-only. The immutable config compiler selects
Shared plus Macos application/binding records on macOS and Shared plus Windows
records elsewhere. It lowers macOS modifier names only far enough to retain
the existing action table; P04b3a does not execute them.

### Timeout and bounded strings

The worker creates the application AX element and sets a 50 ms element-local
`AXUIElementSetMessagingTimeout` before copying the focused window. It applies
the same element-local timeout to that window before copying its title. It
never changes the process-global AX timeout. `kAXErrorCannotComplete` is
classified as a timeout; every non-success result becomes Unknown.

CF strings are accepted only when their UTF-16 length is at most 512 units and
their complete UTF-8 representation fits a fixed 2,048-byte worker stack
buffer. Partial conversion, wrong Core Foundation type, invalid UTF-8, null
value, or excess length becomes Unknown. No title, bundle identifier, frame,
raw path, or input value is logged.

### Requests, coalescing, and cache

Only the Event Tap run-loop owner can submit context work after it drains an
already normalized event. `event_tap_callback` has no resolver reference and
continues to perform exactly the ADR 0016 queue/counter work.

Mouse movement requests are limited to one per 25 ms. Button-down may request
immediately, but never waits for a result. One fixed atomic latest-request
slot and a capacity-one wake channel coalesce overload to the newest
point/timestamp while a prior AX query is in flight. The worker never builds a
request backlog.

The worker publishes one complete numeric latest snapshot containing the
existing `ContextView` fields and its private process/window identity. An
error publishes Unknown and invalidates the preceding value. Consumers accept
only an exact point, at most 100 ms of wrapping-tick age, and one complete
publication. Publication for the same PID with a different process start time
replaces the old identity and produces a different opaque `TargetToken`, so a
reused PID cannot silently inherit the prior window context.

The current P04b3a owner does not feed this snapshot into `InputKernel`.
P04b2 remains listen-only and every physical event passes. A later suppression
phase must verify the private process/window identity again before activation;
this phase does not claim that a cached numeric token activates a window.

### Lifecycle and failures

Context-thread creation and readiness are bounded by existing Engine startup.
The worker observes a capacity-one wake and a 10 ms stop poll. Shutdown sets
its atomic stop, wakes it, and joins after any in-flight AX call is bounded by
the 50 ms messaging timeout. A worker-exit guard invalidates the latest
snapshot, including unwinding.

Permission denial, AX error or timeout, target switch/exit, configuration
unavailability, malformed/oversized string, stale cache, request overload, or
worker lag all degrade to Unknown. There is no retry loop beyond later
coalesced physical observations. KPI aggregation and its single category-only
stop log run only on the worker.

## Verification

`contracts/p04b3a-macos-context-resolver.json` maps fifteen independently
falsifiable obligations to fifteen uniquely named tests:
`O = 15`, `O_v = 15`, `U = 0`, `T = 15`, `T_u = 15`, `T_i = 0`, `T_e = 0`,
`T_r = 0`, `P = 0`, `D = 0`, and `F = 0`.

The Apple Silicon macOS CI job compiles every target with Clippy, runs the
library tests without requiring Accessibility trust, builds the ad-hoc signed
application, and reruns the packaged same-executable/UDS gates. Deterministic
tests cover no-prompt options, callback separation, rate limiting,
capacity-one coalescing, failure invalidation, string bounds, PID reuse,
freshness, macOS config selection, and worker stop.

The non-interactive runner cannot grant or revoke Accessibility trust and does
not inject a real AX timeout or kill a real foreground GUI process during the
query. The thin FFI leaf is compiled and its actual invalid-PID
`proc_pidinfo` path is exercised on macOS; permission-dependent success
remains manual integration evidence.

## Deferrals

P04b3a does not add:

- a Settings permission request, prompt, explanation, or System Settings link;
- event suppression, replay, action injection, target activation, or a
  self-generated-event marker;
- `InputKernel` consumption of macOS context;
- renderer, status UI, AX notification observer, or app/window activation;
- login items, updater changes, Developer ID credentials, or notarization
  evidence; or
- final P06 latency/RSS acceptance.

## Consequences

macOS now has a bounded, fail-open context capability and cache behind the
listen-only owner. The callback budget and Windows native owner remain
unchanged, while later routing can consume the existing numeric context seam
without first introducing a platform framework.
